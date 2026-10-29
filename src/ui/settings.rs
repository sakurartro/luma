use crate::backend::settings::{self, Change, Settings};
use crate::{AgentRow, MainWindow, SettingField, Theme};
use serde_json::Value;
use slint::{ComponentHandle, Model, SharedString, VecModel};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::rc::Rc;
use std::sync::OnceLock;
use std::thread;

use super::Ctx;

// search_dirs / semantic / ocr as the watcher and indexers got them at startup;
// they can't be reconfigured live, so a difference asks for a restart.
static STARTED_WITH: OnceLock<(Vec<String>, bool, bool)> = OnceLock::new();

fn restart_values(st: &Settings) -> (Vec<String>, bool, bool) {
    (st.search_dirs.clone(), st.semantic, st.ocr)
}

pub fn remember_startup(st: &Settings) {
    let _ = STARTED_WITH.set(restart_values(st));
}

/// Everything that can change without a restart.
pub(super) fn apply_settings(window: &MainWindow, st: &Settings, ctx: &Ctx) {
    let theme = window.global::<Theme>();
    theme.set_bg(settings::parse_color(&st.bg));
    theme.set_surface(settings::parse_color(&st.surface));
    theme.set_hover(settings::parse_color(&st.hover));
    theme.set_selected(settings::parse_color(&st.selected));
    theme.set_border(settings::parse_color(&st.border));
    theme.set_accent(settings::parse_color(&st.accent));
    theme.set_text(settings::parse_color(&st.text));
    theme.set_text_dim(settings::parse_color(&st.text_dim));
    ctx.currency.set(st.currency);
    *ctx.hide_patterns.borrow_mut() = st.hide_patterns.clone();
}

// Menu sections in display order; keys not listed here (new Settings fields,
// keys added by hand) land in "Other" in file order.
const CATEGORIES: &[(&str, &[&str])] = &[
    ("Appearance", &["bg", "surface", "hover", "selected", "border", "accent", "text", "text_dim"]),
    ("Files", &["search_dirs", "hide_patterns"]),
    ("Photos", &["semantic", "ocr"]),
    ("Calculator", &["currency"]),
];

// Set by the Agents buttons, not by hand: no row of its own, not even in "Other".
const HIDDEN: &str = "agent_api";

fn field(key: &str, value: &Value) -> Option<SettingField> {
    let mut field = SettingField { key: key.into(), ..Default::default() };
    match value {
        Value::Bool(on) => field.on = *on,
        Value::String(s) => {
            field.kind = if settings::is_color(s) { 1 } else { 2 };
            field.text = s.into();
            field.color = settings::parse_color(s);
        }
        Value::Array(items) if items.iter().all(Value::is_string) => {
            field.kind = 3;
            let items: Vec<SharedString> =
                items.iter().filter_map(Value::as_str).map(Into::into).collect();
            field.items = Rc::new(VecModel::from(items)).into();
        }
        // shortcut: numbers, objects and mixed lists are not shown; add a kind when Settings gets one
        _ => return None,
    }
    Some(field)
}

// Rows straight from the file, grouped by CATEGORIES; the first row of each
// section carries its heading.
fn fields(obj: &serde_json::Map<String, Value>) -> Vec<SettingField> {
    let listed: Vec<&str> = CATEGORIES.iter().flat_map(|(_, keys)| keys.iter().copied()).collect();
    let other: Vec<&str> =
        obj.keys().map(String::as_str).filter(|k| !listed.contains(k) && *k != HIDDEN).collect();
    let mut sections: Vec<(&str, &[&str])> = CATEGORIES.to_vec();
    sections.push(("Other", &other));

    let mut rows = Vec::new();
    for (name, keys) in sections {
        let start = rows.len();
        rows.extend(keys.iter().filter_map(|key| field(key, obj.get(*key)?)));
        if let Some(first) = rows.get_mut(start) {
            first.heading = name.into();
        }
    }
    rows
}

// Re-reads the file into the menu; a file that doesn't parse shows the
// error and no rows, so nothing can be written over it.
fn show(window: &MainWindow) {
    match settings::read_object() {
        Ok(obj) => {
            let st = serde_json::from_value::<Settings>(Value::Object(obj.clone())).ok();
            let started = STARTED_WITH.get();
            window.set_needs_restart(matches!((st, started), (Some(st), Some(s)) if restart_values(&st) != *s));
            window.set_settings(Rc::new(VecModel::from(fields(&obj))).into());
            window.set_settings_error("".into());
        }
        Err(err) => {
            window.set_settings(Rc::new(VecModel::<SettingField>::default()).into());
            window.set_settings_error(format!("settings.json: {err}").into());
        }
    }
}

// How Luma gets into an agent: through the agent's own CLI where it has one that
// can add and remove servers, so their config files stay theirs.
enum Backend {
    // flags for `mcp add` / `mcp remove`: Claude Code would register per project by default
    Cli(&'static [&'static str]),
    // opencode's `mcp add` is interactive and it has no `mcp remove`: only our
    // `mcp.luma` key of its opencode.json is edited
    OpencodeJson,
}

struct Agent {
    name: &'static str,
    bin: &'static str,
    backend: Backend,
}

const AGENTS: &[Agent] = &[
    Agent { name: "Claude Code", bin: "claude", backend: Backend::Cli(&["--scope", "user"]) },
    Agent { name: "Codex", bin: "codex", backend: Backend::Cli(&[]) },
    Agent { name: "opencode", bin: "opencode", backend: Backend::OpencodeJson },
];

// The GUI may not have the shell's PATH, so the usual install dirs are tried too.
fn cli(agent: &Agent, args: &[&str]) -> std::io::Result<Output> {
    let home = std::env::var("HOME").unwrap_or_default();
    let bin = agent.bin;
    let dirs = [".local/bin", ".claude/local", ".opencode/bin"];
    let mut result = Err(std::io::ErrorKind::NotFound.into());
    for path in std::iter::once(bin.to_string()).chain(dirs.map(|dir| format!("{home}/{dir}/{bin}"))) {
        result = Command::new(path).args(args).output();
        if !matches!(&result, Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
            break;
        }
    }
    result
}

fn opencode_config() -> PathBuf {
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"),
    };
    dir.join("opencode/opencode.json")
}

/// opencode.json with `mcp.luma` set to run `exe mcp` (Some) or removed (None);
/// everything else in the file is kept. Err when the text isn't plain JSON
/// (opencode accepts comments, serde_json doesn't).
fn with_luma_server(text: &str, exe: Option<&str>) -> anyhow::Result<String> {
    use serde_json::{Map, Value, json};
    let mut obj: Map<String, Value> =
        if text.trim().is_empty() { Map::new() } else { serde_json::from_str(text)? };
    match exe {
        Some(exe) => {
            let mcp = obj.entry("mcp").or_insert_with(|| json!({}));
            let mcp = mcp.as_object_mut().ok_or_else(|| anyhow::anyhow!("\"mcp\" is not an object"))?;
            mcp.insert("luma".into(), json!({"type": "local", "command": [exe, "mcp"], "enabled": true}));
        }
        None => {
            if let Some(mcp) = obj.get_mut("mcp").and_then(Value::as_object_mut) {
                mcp.remove("luma");
            }
        }
    }
    Ok(serde_json::to_string_pretty(&obj)?)
}

fn read_opencode_config() -> anyhow::Result<String> {
    match std::fs::read_to_string(opencode_config()) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

fn edit_opencode(exe: Option<&str>) -> anyhow::Result<()> {
    let path = opencode_config();
    let json = with_luma_server(&read_opencode_config()?, exe)
        .map_err(|err| anyhow::anyhow!("{}: {err} (comments in it? edit it by hand)", path.display()))?;
    // canonical path: the rename below must replace a symlink's target, not the link
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // tmp + rename: a crash mid-write never leaves a truncated config
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Err: opencode.json can't be read as plain JSON.
fn opencode_registered() -> anyhow::Result<bool> {
    let text = read_opencode_config()?;
    if text.trim().is_empty() {
        return Ok(false);
    }
    Ok(serde_json::from_str::<Value>(&text)?.pointer("/mcp/luma").is_some())
}

fn connected(agent: &Agent) -> bool {
    match agent.backend {
        Backend::Cli(_) => cli(agent, &["mcp", "get", "luma"]).is_ok_and(|out| out.status.success()),
        Backend::OpencodeJson => opencode_registered().unwrap_or(false),
    }
}

/// (status line, button label); no label = nothing to click.
fn agent_state(agent: &Agent, search_on: bool) -> (&'static str, &'static str) {
    let registered = match agent.backend {
        Backend::Cli(_) => match cli(agent, &["mcp", "get", "luma"]) {
            Err(_) => return ("not installed", ""),
            Ok(out) => out.status.success(),
        },
        Backend::OpencodeJson => {
            if cli(agent, &["--version"]).is_err() {
                return ("not installed", "");
            }
            match opencode_registered() {
                Ok(registered) => registered,
                Err(_) => return ("opencode.json has comments, edit it by hand", ""),
            }
        }
    };
    match (registered, search_on) {
        (true, _) => ("connected", "Disconnect"),
        // the agent could connect, but every search would answer "not available"
        (false, false) => ("enable semantic or ocr and restart first", ""),
        (false, true) => ("not connected", "Connect"),
    }
}

fn run(agent: &Agent, args: &[&str]) -> anyhow::Result<()> {
    let out = cli(agent, args)?;
    if !out.status.success() {
        anyhow::bail!("{} mcp: {}", agent.bin, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn remove(agent: &Agent) -> anyhow::Result<()> {
    match agent.backend {
        Backend::Cli(scope) => {
            let mut args = vec!["mcp", "remove"];
            args.extend(scope);
            args.push("luma");
            run(agent, &args)
        }
        Backend::OpencodeJson => edit_opencode(None),
    }
}

fn connect_agent(agent: &Agent) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy();
    match agent.backend {
        Backend::Cli(scope) => {
            // add fails when luma is already registered (e.g. pointing at an old binary)
            let _ = remove(agent);
            let mut args = vec!["mcp", "add"];
            args.extend(scope);
            args.extend(["luma", "--", exe.as_ref(), "mcp"]);
            run(agent, &args)?;
        }
        Backend::OpencodeJson => edit_opencode(Some(&exe))?,
    }
    settings::update("agent_api", Change::Set(true))?;
    Ok(())
}

fn disconnect_agent(agent: &Agent) -> anyhow::Result<()> {
    remove(agent)?;
    // the flag stays on while another agent still uses it
    let in_use = AGENTS.iter().any(|other| other.bin != agent.bin && connected(other));
    settings::update("agent_api", Change::Set(in_use))?;
    Ok(())
}

// What the running indexers got at startup, not the file: toggles apply after a restart.
fn search_on() -> bool {
    STARTED_WITH.get().is_some_and(|started| started.1 || started.2)
}

fn set_agent(window: &MainWindow, i: usize, status: &str, action: &str) {
    let agents = window.get_agents();
    if let Some(mut row) = agents.row_data(i) {
        row.status = status.into();
        row.action = action.into();
        agents.set_row_data(i, row);
    }
}

// The CLIs take a second or more (node): never on the UI thread. No action
// while one runs, so the button can't be pressed twice.
fn refresh_agent(window: &MainWindow, i: usize) {
    let search_on = search_on();
    set_agent(window, i, "checking...", "");
    let weak = window.as_weak();
    thread::spawn(move || {
        let (status, action) = agent_state(&AGENTS[i], search_on);
        let _ = weak.upgrade_in_event_loop(move |window| set_agent(&window, i, status, action));
    });
}

pub(super) fn wire(main_window: &MainWindow, ctx: &Ctx) {
    let rows: Vec<AgentRow> =
        AGENTS.iter().map(|a| AgentRow { name: a.name.into(), ..Default::default() }).collect();
    main_window.set_agents(Rc::new(VecModel::from(rows)).into());

    let edit = {
        let weak = main_window.as_weak();
        let ctx = ctx.clone();
        move |key: SharedString, change: Change| {
            let Some(window) = weak.upgrade() else { return };
            let result = settings::update(&key, change);
            if let Ok(st) = &result {
                apply_settings(&window, st, &ctx);
                window.invoke_refresh_view();
            }
            show(&window);
            // rejected edit: the row re-renders the file's value, the reason stays visible
            if let Err(err) = result {
                eprintln!("settings change rejected: {err}");
                window.set_settings_error(err.to_string().into());
            }
        }
    };
    let edit = Rc::new(edit);

    let weak = main_window.as_weak();
    main_window.on_show_settings(move || {
        if let Some(window) = weak.upgrade() {
            show(&window);
            window.set_agents_idle(!search_on());
            (0..AGENTS.len()).for_each(|i| refresh_agent(&window, i));
        }
    });
    main_window.on_agent_toggle({
        let weak = main_window.as_weak();
        move |i| {
            let i = i as usize;
            let Some(window) = weak.upgrade() else { return };
            let connect = window.get_agents().row_data(i).is_some_and(|row| row.action == "Connect");
            set_agent(&window, i, "working...", "");
            let weak = weak.clone();
            thread::spawn(move || {
                let agent = &AGENTS[i];
                let result = if connect { connect_agent(agent) } else { disconnect_agent(agent) };
                let _ = weak.upgrade_in_event_loop(move |window| {
                    show(&window);
                    if let Err(err) = result {
                        window.set_settings_error(err.to_string().into());
                    }
                    refresh_agent(&window, i);
                });
            });
        }
    });
    main_window.on_setting_toggle({
        let edit = edit.clone();
        move |key| edit(key, Change::Toggle)
    });
    main_window.on_setting_text({
        let edit = edit.clone();
        move |key, text| edit(key, Change::Text(text.into()))
    });
    main_window.on_setting_list_set({
        let edit = edit.clone();
        move |key, i, text| edit(key, Change::ListSet(i as usize, text.into()))
    });
    main_window.on_setting_list_add({
        let edit = edit.clone();
        move |key, text| edit(key, Change::ListAdd(text.into()))
    });
    main_window.on_setting_list_remove(move |key, i| edit(key, Change::ListRemove(i as usize)));

    main_window.on_restart(|| {
        // Without --toggle the new process opens its window right away; the
        // toggle socket fd is CLOEXEC and bind_socket clears the stale file.
        let args = std::env::args().skip(1).filter(|arg| arg != "--toggle");
        let err = match std::env::current_exe() {
            Ok(exe) => Command::new(exe).args(args).exec(),
            Err(err) => err,
        };
        eprintln!("restart failed: {err}");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_grouped_by_category_with_unknown_keys_last() {
        let obj = serde_json::json!({"custom": "x", "currency": true, "accent": "#A9B665", "bg": "#000000"});
        let rows = fields(obj.as_object().unwrap());
        let keys: Vec<_> = rows.iter().map(|r| (r.key.to_string(), r.heading.to_string())).collect();
        assert_eq!(keys, [
            ("bg".into(), "Appearance".into()),
            ("accent".into(), String::new()),
            ("currency".into(), "Calculator".into()),
            ("custom".into(), "Other".into()),
        ]);
    }

    #[test]
    fn opencode_luma_is_added_and_removed_without_touching_the_rest() {
        let text = r#"{"$schema": "x", "plugin": ["p"], "mcp": {"other": {"type": "remote"}}}"#;
        let added: Value = serde_json::from_str(&with_luma_server(text, Some("/bin/luma")).unwrap()).unwrap();
        assert_eq!(added["mcp"]["luma"]["command"], serde_json::json!(["/bin/luma", "mcp"]));
        assert_eq!(added["mcp"]["luma"]["type"], "local");
        assert_eq!(added["mcp"]["other"]["type"], "remote");
        assert_eq!(added["plugin"], serde_json::json!(["p"]));

        let removed: Value =
            serde_json::from_str(&with_luma_server(&added.to_string(), None).unwrap()).unwrap();
        assert!(removed["mcp"].get("luma").is_none());
        assert_eq!(removed["mcp"]["other"]["type"], "remote");
    }

    #[test]
    fn opencode_config_edge_cases() {
        // no file yet: created with just our key
        let added: Value = serde_json::from_str(&with_luma_server("", Some("/bin/luma")).unwrap()).unwrap();
        assert_eq!(added["mcp"]["luma"]["enabled"], true);
        // removing from a file without "mcp" must not invent one
        let same: Value = serde_json::from_str(&with_luma_server("{\"a\": 1}", None).unwrap()).unwrap();
        assert!(same.get("mcp").is_none());
        // comments or a non-object "mcp": refuse, so the file is never overwritten
        assert!(with_luma_server("{\n// comment\n}", Some("/bin/luma")).is_err());
        assert!(with_luma_server("{\"mcp\": 5}", Some("/bin/luma")).is_err());
    }

    #[test]
    fn agent_api_has_no_row() {
        let obj = serde_json::json!({"agent_api": true, "semantic": false});
        let keys: Vec<_> = fields(obj.as_object().unwrap()).iter().map(|r| r.key.to_string()).collect();
        assert_eq!(keys, ["semantic"]);
    }
}
