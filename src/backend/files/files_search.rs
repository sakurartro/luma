use super::db_service::insert_batch;
use anyhow::{Ok, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use walkdir::WalkDir;

// Index progress, polled by the UI timer in main.rs. DONE starts true so the
// progress plate never shows before the first scan begins.
pub static INDEX_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static INDEXED: AtomicUsize = AtomicUsize::new(0);
pub static INDEX_DONE: AtomicBool = AtomicBool::new(true);

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
    // Counting pass (metadata only) so the UI can show an honest percentage.
    let mut total = 0;
    for dir in dirs {
        let root = expand_home(dir);
        if root.is_dir() {
            total += WalkDir::new(&root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.depth() > 0)
                .count();
        }
    }
    INDEX_TOTAL.store(total, Ordering::Relaxed);
    INDEXED.store(0, Ordering::Relaxed);
    INDEX_DONE.store(false, Ordering::Relaxed);

    conn.execute("DELETE FROM files", [])?;
    conn.execute("DELETE FROM folders", [])?;

    // Insert as we walk: INDEXED must reflect rows actually written, not just
    // collected, otherwise the bar sits at a fake 100% during the insert phase.
    let mut batch: Vec<FileEntry> = Vec::new();
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
            batch.push(FileEntry {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: path.to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
            });
            if batch.len() == 100 {
                insert_batch(&batch, conn)?;
                INDEXED.fetch_add(batch.len(), Ordering::Relaxed);
                batch.clear();
            }
        }
    }

    if !batch.is_empty() {
        insert_batch(&batch, conn)?;
        INDEXED.fetch_add(batch.len(), Ordering::Relaxed);
    }

    INDEX_DONE.store(true, Ordering::Relaxed);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_counters_reflect_scan() {
        let dir = std::env::temp_dir().join(format!("luma_progress_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE);
             CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE);",
        )
        .unwrap();

        scan_dirs(&mut conn, &[dir.to_string_lossy().into_owned()]).unwrap();

        assert_eq!(INDEX_TOTAL.load(Ordering::Relaxed), 2);
        assert_eq!(INDEXED.load(Ordering::Relaxed), 2);
        assert!(INDEX_DONE.load(Ordering::Relaxed));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}