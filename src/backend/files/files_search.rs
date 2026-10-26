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

// True once this start's scan finished with every search dir present. The
// clip/ocr workers prune rows of vanished files only then: after a failed
// scan or with an unmounted dir, `files` is incomplete and the prune would
// wipe their whole index.
pub static SCAN_COMPLETE: AtomicBool = AtomicBool::new(false);

// Image extensions: the clip/ocr indexing workers (image_where) and the
// Photos view (is_image) both filter by this list; file_kind maps it to the
// image icon.
const IMAGE_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "webp", "gif", "bmp"];

// SQL predicate for image files in `files f`; LIKE is ASCII case-insensitive.
pub fn image_where() -> String {
    let likes: Vec<String> = IMAGE_EXTS.iter().map(|ext| format!("f.path LIKE '%.{ext}'")).collect();
    format!("({})", likes.join(" OR "))
}

// Same rule as image_where, case-insensitive like LIKE.
pub fn is_image(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

// FileItem.kind, picks the icon in app-window.slint: 1 document, 2 spreadsheet,
// 3 presentation, 4 archive, 5 code, 6 audio, 7 video, 8 image; 0 = generic
// file and folders.
pub fn file_kind(path: &str, is_dir: bool) -> i32 {
    const KINDS: [&[&str]; 8] = [
        &["pdf", "doc", "docx", "odt", "rtf", "txt", "md", "epub"],
        &["xls", "xlsx", "ods", "csv"],
        &["ppt", "pptx", "odp", "key"],
        &["zip", "tar", "gz", "tgz", "xz", "zst", "bz2", "7z", "rar"],
        &["rs", "py", "js", "ts", "c", "h", "cpp", "go", "java", "sh", "json", "toml", "yaml", "yml", "html", "css"],
        &["mp3", "flac", "wav", "ogg", "opus", "m4a"],
        &["mp4", "mkv", "webm", "avi", "mov"],
        &IMAGE_EXTS,
    ];
    let Some((_, ext)) = path.rsplit_once('.').filter(|_| !is_dir) else { return 0 };
    KINDS
        .iter()
        .position(|exts| exts.iter().any(|e| ext.eq_ignore_ascii_case(e)))
        .map_or(0, |i| i as i32 + 1)
}

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
    let mut all_dirs_present = true;
    for dir in dirs {
        let root = expand_home(dir);
        if !root.is_dir() {
            all_dirs_present = false;
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
    SCAN_COMPLETE.store(all_dirs_present, Ordering::Relaxed);

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
        assert!(SCAN_COMPLETE.load(Ordering::Relaxed));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn is_image_by_extension() {
        assert!(is_image("/p/a.JPG"));
        assert!(is_image("/p/a.jpeg"));
        assert!(!is_image("/p/a.txt"));
        assert!(!is_image("/p/a.jpeg.txt"));
        assert!(!is_image("/p.png/a"));
    }

    #[test]
    fn file_kind_by_extension() {
        assert_eq!(file_kind("/p/a.PDF", false), 1);
        assert_eq!(file_kind("/p/a.Xlsx", false), 2);
        assert_eq!(file_kind("/p/a.tar.gz", false), 4);
        assert_eq!(file_kind("/p/a.rs", false), 5);
        assert_eq!(file_kind("/p/a.JPG", false), 8);
        assert_eq!(file_kind("/p/a.unknown", false), 0);
        assert_eq!(file_kind("/p/noext", false), 0);
        assert_eq!(file_kind("/p.png/a", false), 0);
        assert_eq!(file_kind("/p/dir.zip", true), 0);
    }
}