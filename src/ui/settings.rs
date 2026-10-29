use crate::backend::settings::{self, Change, Settings};
use crate::{MainWindow, SettingField, Theme};
use serde_json::Value;
use slint::{ComponentHandle, SharedString, VecModel};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::rc::Rc;
use std::sync::OnceLock;

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
    ("Agents", &["agent_api"]),
];

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
    let other: Vec<&str> = obj.keys().map(String::as_str).filter(|k| !listed.contains(k)).collect();
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

pub(super) fn wire(main_window: &MainWindow, ctx: &Ctx) {
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
}
