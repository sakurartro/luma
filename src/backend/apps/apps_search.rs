use super::db_service::{app_paths, get_data, insert_batch, prune_apps};
use anyhow::Result;
use rusqlite::params;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Hash)]
pub struct App {
    pub app_name: String,
    // lowercase name for fuzzy search, filled where the struct is built
    pub name_lower: Vec<char>,
    pub path: String,
    pub command: String,
    pub icon_path: String,
    pub categories: Vec<String>,
    pub time_launched: i64,
    pub last_launched: i64,
    pub score: i64,
    pub pinned: bool,
}

pub struct Apps {
    pub apps: Vec<App>,
}

/// XDG data dirs in lookup priority: $XDG_DATA_HOME, then $XDG_DATA_DIRS
/// (flatpak/snap add their export dirs there).
fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{home}/.local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());

    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in std::iter::once(data_home.as_str()).chain(data_dirs.split(':')) {
        let dir = PathBuf::from(dir);
        // spec: relative entries are invalid
        if dir.is_absolute() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Desktop file ID per spec: path under the applications dir, '/' → '-'.
fn desktop_id(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('/', "-")
}

/// Splits an Exec value into argv per the Desktop Entry spec: unquoted
/// whitespace separates args, inside "..." `\"` `\`` `\$` `\\` are escapes.
/// Field codes and flatpak's @@ markers are dropped — we never pass files/URLs.
/// shortcut: the string-level unescape (`\\` → `\`, `\s`) that the spec applies
/// before quoting is skipped, upgrade if a .desktop file needs it.
pub fn exec_args(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut arg = String::new();
    let (mut in_arg, mut quoted) = (false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            '\\' if quoted => match chars.next() {
                Some(next @ ('"' | '`' | '$' | '\\')) => arg.push(next),
                Some(next) => {
                    arg.push('\\');
                    arg.push(next);
                }
                None => arg.push('\\'),
            },
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut arg));
                    in_arg = false;
                }
            }
            c => {
                arg.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(arg);
    }
    args.into_iter()
        .filter(|arg| {
            !matches!(
                arg.as_str(),
                "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k"
                    | "%d" | "%D" | "%n" | "%N" | "%v" | "%m"
                    | "@@" | "@@u" | "@@f"
            )
        })
        .map(|arg| arg.replace("%%", "%"))
        .collect()
}

/// Existing applications dirs, highest priority first.
pub fn app_dirs() -> Vec<PathBuf> {
    data_dirs()
        .into_iter()
        .map(|dir| dir.join("applications"))
        .filter(|dir| dir.is_dir())
        .collect()
}

impl Apps {
    fn find_apps_paths(dir: &Path, extension: &str) -> Vec<PathBuf> {
        let mut apps = Vec::new();

        // an unreadable subdir in any of the app dirs must not abort the scan
        for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
            let path = entry.path();

            if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
                apps.push(path.to_path_buf());
            }
        }

        apps
    }

    /// Desktop file ID → path. An ID in an earlier dir shadows the same ID
    /// later, so a user override with Hidden/NoDisplay still hides the system entry.
    fn desktop_paths(dirs: &[PathBuf]) -> HashMap<String, PathBuf> {
        let mut by_id = HashMap::new();
        for dir in dirs {
            for path in Self::find_apps_paths(dir, "desktop") {
                by_id.entry(desktop_id(dir, &path)).or_insert(path);
            }
        }
        by_id
    }

    /// Icon-name → path map, cached in the icons table. The filesystem walk of
    /// the icon dirs dominates cold start (~0.3 s warm, up to 2 s cold), so it
    /// runs only once per machine; the watcher refreshes the cache when
    /// .desktop files change (package install/remove).
    pub fn initial_icons_scan(conn: &mut Connection) -> anyhow::Result<HashMap<String, PathBuf>> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM icons", [], |row| row.get(0))?;
        if count > 0 {
            let mut stmt = conn.prepare("SELECT name, path FROM icons")?;
            let icons = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        PathBuf::from(row.get::<_, String>(1)?),
                    ))
                })?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?;
            return Ok(icons);
        }
        Self::refresh_icons(conn)
    }

    /// Forced rescan of the icon dirs + persist into the icons table.
    pub fn refresh_icons(conn: &mut Connection) -> anyhow::Result<HashMap<String, PathBuf>> {
        let icons = Self::scan_icon_dirs();
        let tx = conn.transaction()?;
        {
            tx.execute("DELETE FROM icons", [])?;
            let mut stmt = tx.prepare("INSERT OR REPLACE INTO icons (name, path) VALUES (?1, ?2)")?;
            for (name, path) in &icons {
                stmt.execute(params![name, path.to_string_lossy()])?;
            }
        }
        tx.commit()?;
        Ok(icons)
    }

    fn scan_icon_dirs() -> HashMap<String, PathBuf> {
        let mut icons: HashMap<String, PathBuf> = HashMap::new();
        let home = std::env::var("HOME").unwrap_or_default();
        let mut roots = vec![PathBuf::from(format!("{home}/.icons"))];
        roots.extend(data_dirs().into_iter().map(|dir| dir.join("icons")));
        roots.push(PathBuf::from("/usr/share/pixmaps"));

        for root in roots {
            if !root.exists() {
                continue;
            }
            for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
                let path = entry.path();
                if path.is_file()
                    && let Some(ext) = path.extension()
                {
                    if ext == "png" || ext == "svg" {
                        icons.entry(
                            path.file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                        )
                        .or_insert(path.to_path_buf());
                    }
                }
            }
        }

        icons
    }

    pub fn parse_config(
        icons: &HashMap<String, PathBuf>,
        paths: Vec<PathBuf>,
    ) -> Result<Self> {
        let mut apps: Vec<App> = Vec::new();
        for path in paths {
            // one unreadable/non-UTF-8 file in a user dir must not abort the scan
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            let desktop_entry: Vec<&str> = content
                .lines()
                .map(str::trim)
                .skip_while(|line| *line != "[Desktop Entry]")
                .skip(1)
                .take_while(|line| !line.starts_with('['))
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect();

            let is_application = desktop_entry.iter().any(|line| *line == "Type=Application");
            let no_display = desktop_entry.iter().any(|line| *line == "NoDisplay=true");
            let hidden = desktop_entry.iter().any(|line| *line == "Hidden=true");

            if !is_application || no_display || hidden {
                continue;
            }

            let name = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Name="));
            let command = desktop_entry
                .iter()
                // raw Exec value; exec_args() splits it at launch time
                .find_map(|line| line.strip_prefix("Exec="))
                .map(String::from);

            let icon = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Icon="));

            let str_categories = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Categories="));

            let categories: Vec<String> = str_categories
                .unwrap_or("")
                .split(";")
                .map(String::from)
                .collect();

            let mut icon_path = String::new();

            if let Some(icon_name) = icon {
                let clean_icon = icon_name.trim().trim_matches('"');
                let path = Path::new(clean_icon);

                if path.is_absolute() {
                    if path.exists() {
                        icon_path.push_str(clean_icon);
                    }
                } else {
                    if let Some(icon_from_hash) = icons.get(clean_icon) {                        icon_path.push_str(&icon_from_hash.to_string_lossy());
                    }
                }
            }

            if name.is_none() || command.is_none() {
                continue;
            } else {
                let app = App {
                    app_name: name.unwrap_or("").to_owned(),
                    name_lower: name.unwrap_or("").to_lowercase().chars().collect(),
                    path: path.to_string_lossy().into_owned(),
                    command: command.unwrap_or_default(),
                    icon_path: icon_path,
                    categories: categories,
                    time_launched: 0,
                    last_launched: 0,
                    score: 0,
                    pinned: false,
                };
                apps.push(app);
            }
        }

        Ok(Self { apps })
    }

    pub fn refresh(conn: &mut Connection) -> Result<Self> {
        let apps = get_data(conn)?;
        Ok(Self { apps })
    }

    pub fn find_apps(conn: &mut Connection, icons: &HashMap<String, PathBuf>) -> Result<Self> {
        let dirs = app_dirs();
        let by_id = Self::desktop_paths(&dirs);
        let apps = Self::parse_config(icons, by_id.values().cloned().collect())?;
        let shown: HashSet<&str> = apps.apps.iter().map(|app| app.path.as_str()).collect();

        // Rows not shown by this scan: if another shown file now owns the same
        // desktop ID (override added, or removed and the system file is back)
        // → move to it, keeping pin/stats; otherwise (gone, hidden, shadowed
        // by a hidden override) → delete. Existing files outside the scanned
        // dirs (luma started with a narrower XDG_DATA_DIRS) or skipped by the
        // walk are left alone so a bad env can't wipe pins and stats.
        let stale: Vec<(String, Option<String>)> = app_paths(conn)?
            .into_iter()
            .filter(|row| !shown.contains(row.as_str()))
            .filter_map(|row| {
                let path = Path::new(&row);
                let winner = dirs
                    .iter()
                    .find(|dir| path.starts_with(dir))
                    .and_then(|dir| by_id.get(&desktop_id(dir, path)));
                if winner.is_none() && path.exists() {
                    return None;
                }
                let heir = winner
                    .map(|winner| winner.to_string_lossy().into_owned())
                    .filter(|winner| *winner != row && shown.contains(winner.as_str()));
                Some((row, heir))
            })
            .collect();
        prune_apps(conn, &stale)?;

        for chunk in apps.apps.chunks(100) {
            insert_batch(chunk, conn)?;
        }
        Ok(apps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_paths_first_dir_wins_by_id() {
        let root = std::env::temp_dir().join(format!("luma_apps_test_{}", std::process::id()));
        let (user, system) = (root.join("user"), root.join("system"));
        fs::create_dir_all(user.join("kde")).unwrap();
        fs::create_dir_all(&system).unwrap();
        for file in [user.join("foo.desktop"), user.join("kde/bar.desktop"),
                     system.join("foo.desktop"), system.join("kde-bar.desktop"),
                     system.join("baz.desktop")] {
            fs::write(file, "").unwrap();
        }

        let mut paths: Vec<PathBuf> = Apps::desktop_paths(&[user.clone(), system.clone()]).into_values().collect();
        paths.sort();
        fs::remove_dir_all(&root).unwrap();

        let mut expected = vec![system.join("baz.desktop"), user.join("foo.desktop"), user.join("kde/bar.desktop")];
        expected.sort();
        assert_eq!(paths, expected);
    }

    #[test]
    fn exec_args_follows_spec() {
        assert_eq!(exec_args("firefox %u"), ["firefox"]);
        assert_eq!(exec_args(r#"sh -c "echo hi there""#), ["sh", "-c", "echo hi there"]);
        assert_eq!(exec_args(r#"sh -c "echo \"hi\" \$HOME \\ \`x\`""#), ["sh", "-c", r#"echo "hi" $HOME \ `x`"#]);
        assert_eq!(exec_args("printf 100%% %%u"), ["printf", "100%", "%u"]);
        assert_eq!(exec_args(r#""/opt/My App/app" """#), ["/opt/My App/app", ""]);
        assert_eq!(
            exec_args("/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=firefox --file-forwarding org.mozilla.firefox @@u %u @@"),
            ["/usr/bin/flatpak", "run", "--branch=stable", "--arch=x86_64", "--command=firefox", "--file-forwarding", "org.mozilla.firefox"]
        );
    }
}
