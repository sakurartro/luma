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

// Bumped whenever the files DB changes (scan finished or watcher applied an
// edit); the UI uses it to know when its in-memory copy is stale.
pub static DB_VERSION: AtomicUsize = AtomicUsize::new(0);

// SQL predicate for image files in `files f`; shared by the clip and ocr
// indexing workers.
pub const IMAGE_WHERE: &str = "(f.path LIKE '%.jpg' OR f.path LIKE '%.jpeg' \
     OR f.path LIKE '%.png' OR f.path LIKE '%.webp' OR f.path LIKE '%.gif' \
     OR f.path LIKE '%.bmp')";

#[derive(Debug)]
pub struct FileEntry {
    pub name: String,
    // lowercase name for fuzzy search, filled where the struct is built
    pub name_lower: Vec<char>,
    pub path: String,
    pub is_dir: bool,
    // unix seconds of the modification time; 0 when unavailable
    pub mtime: i64,
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

// Unix seconds of the path's modification time; 0 when it can't be read.
pub fn mtime_of(path: &std::path::Path) -> i64 {
    path.metadata()
        .and_then(|m| m.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
        .unwrap_or(0)
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
            // skip unreadable/vanished paths instead of aborting the whole scan
            let Some(entry) = entry_result.ok() else { continue };
            if entry.depth() == 0 {
                continue;
            }
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let name_lower = name.to_lowercase().chars().collect();
            batch.push(FileEntry {
                name,
                name_lower,
                path: path.to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
                mtime: mtime_of(path),
            });
            if batch.len() == 5000 {
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
    DB_VERSION.fetch_add(1, Ordering::Relaxed);

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
            "CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();

        scan_dirs(&mut conn, &[dir.to_string_lossy().into_owned()]).unwrap();

        assert_eq!(INDEX_TOTAL.load(Ordering::Relaxed), 2);
        assert_eq!(INDEXED.load(Ordering::Relaxed), 2);
        assert!(INDEX_DONE.load(Ordering::Relaxed));
        assert_eq!(DB_VERSION.load(Ordering::Relaxed), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}