use super::db_service::insert_batch;
use anyhow::{Ok, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// "~/" at the start (or bare "~") expands to $HOME; anything else stays as-is.
pub fn expand_home(dir: &str) -> PathBuf {
    let home = match std::env::var_os("HOME") {
        Some(home) if dir == "~" || dir.starts_with("~/") => PathBuf::from(home),
        _ => return PathBuf::from(dir),
    };
    match dir.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => home,
    }
}

// Wipe + full rescan: files carry no stats, so there is nothing to lose.
// Watcher handles changes while the process is alive.
pub fn scan_dirs(conn: &mut Connection, dirs: &[String]) -> Result<()> {
    conn.execute("DELETE FROM files", [])?;
    conn.execute("DELETE FROM folders", [])?;

    let mut entries: Vec<FileEntry> = Vec::new();
    for dir in dirs {
        let root = expand_home(dir);
        if !root.is_dir() {
            continue;
        }
        for entry_result in WalkDir::new(&root) {
            let entry = entry_result?;
            if entry.depth() == 0 {
                continue;
            }
            let path = entry.path();
            entries.push(FileEntry {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: path.to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
            });
        }
    }

    for chunk in entries.chunks(100) {
        insert_batch(chunk, conn)?;
    }

    Ok(())
}