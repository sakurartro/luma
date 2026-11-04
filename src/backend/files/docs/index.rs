// Worker: after the file scan takes documents from `files` that have no fresh
// doc_meta row (by mtime), extracts their text and writes doc_meta + doc_fts.
// No models, so no ONNX_LOCK.

use super::extract::{doc_where, extract, Limits, SKIP_DIRS};
use crate::backend::apps::db_init::open_conn;
use crate::backend::files::files_search::{expand_home, SCAN_COMPLETE};
use crate::backend::settings::Settings;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// Progress, polled by the UI like OCR_*.
pub static DOCS_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static DOCS_DONE: AtomicUsize = AtomicUsize::new(0);

// Extraction is slow for big files, so keep chunks small.
const DB_CHUNK: i64 = 32;

struct Config {
    limits: Limits,
    roots: Vec<PathBuf>,
    hide_patterns: Vec<String>,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

// Only files under search_dirs (exclude_dirs are already out of `files`),
// not in SKIP_DIRS and not hidden by a hide_pattern, checked on every folder
// and the file name below the search dir.
fn allowed(path: &Path, cfg: &Config) -> bool {
    let Some(rel) = cfg.roots.iter().find_map(|root| path.strip_prefix(root).ok()) else { return false };
    rel.components().all(|c| match c {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            !SKIP_DIRS.contains(&&*name)
                && !cfg.hide_patterns.iter().any(|p| !p.is_empty() && name.starts_with(p.as_str()))
        }
        _ => true,
    })
}

// Documents from `files` with no doc_meta row or a stale mtime, paged by path.
fn fetch_candidates(conn: &Connection, after: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    let sql = format!(
        "SELECT f.path, f.mtime FROM files f
         LEFT JOIN doc_meta d ON d.file_path = f.path AND d.mtime = f.mtime
         WHERE d.file_path IS NULL AND {} AND f.path > ?1
         ORDER BY f.path LIMIT ?2",
        doc_where()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![after, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// Rows of files that vanished from the rescan.
fn delete_orphans(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM doc_meta WHERE file_path NOT IN (SELECT path FROM files)", [])?;
    conn.execute("DELETE FROM doc_fts WHERE file_path NOT IN (SELECT path FROM files)", [])?;
    Ok(())
}

// A row with empty text (scan, encrypted, corrupt) only gets doc_meta, so the
// file isn't read again until it changes.
fn upsert_batch(conn: &mut Connection, rows: &[(String, i64, String)]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut meta = tx.prepare(
            "INSERT INTO doc_meta (file_path, mtime) VALUES (?1, ?2)
             ON CONFLICT (file_path) DO UPDATE SET mtime = excluded.mtime",
        )?;
        let mut del = tx.prepare("DELETE FROM doc_fts WHERE file_path = ?1")?;
        let mut fts = tx.prepare("INSERT INTO doc_fts (file_path, body) VALUES (?1, ?2)")?;
        for (path, mtime, text) in rows {
            meta.execute(params![path, mtime])?;
            del.execute([path])?;
            if !text.trim().is_empty() {
                fts.execute(params![path, text])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

// One pass over everything stale; `prune` also drops rows of vanished files.
fn run_pass(conn: &mut Connection, cfg: &Config, prune: bool) -> Result<()> {
    if prune {
        delete_orphans(conn)?;
    }
    let total_sql = format!(
        "SELECT COUNT(*) FROM files f LEFT JOIN doc_meta d
         ON d.file_path = f.path AND d.mtime = f.mtime
         WHERE d.file_path IS NULL AND {}",
        doc_where()
    );
    let total: i64 = conn.query_row(&total_sql, [], |r| r.get(0))?;
    DOCS_TOTAL.store(total.max(0) as usize, Ordering::Relaxed);
    DOCS_DONE.store(0, Ordering::Relaxed);

    let mut after = String::new();
    loop {
        let chunk = fetch_candidates(conn, &after, DB_CHUNK)?;
        let Some((last, _)) = chunk.last() else { break };
        after = last.clone();
        // Files outside the rules get no row; they are skipped again (cheaply) next time.
        let rows: Vec<(String, i64, String)> = chunk
            .iter()
            .filter(|(path, _)| allowed(Path::new(path), cfg))
            .map(|(path, mtime)| (path.clone(), *mtime, extract(Path::new(path), &cfg.limits)))
            .collect();
        upsert_batch(conn, &rows)?;
        DOCS_DONE.fetch_add(chunk.len(), Ordering::Relaxed);
    }
    DOCS_DONE.store(DOCS_TOTAL.load(Ordering::Relaxed), Ordering::Relaxed);
    Ok(())
}

// Startup and watcher passes never overlap.
static PASS_LOCK: Mutex<()> = Mutex::new(());

fn index_pass() {
    let Some(cfg) = CONFIG.get() else { return };
    let _guard = PASS_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = open_conn().and_then(|mut conn| run_pass(&mut conn, cfg, SCAN_COMPLETE.load(Ordering::Relaxed)));
    if let Err(err) = result {
        eprintln!("docs indexing error: {err}");
    }
}

/// Starts the indexing pass in the background; gated by the `docs` setting
/// (off = no thread, no reads of the user's files). Call after the file scan.
pub fn start_indexing(st: &Settings) {
    if !st.docs {
        return;
    }
    let cfg = Config {
        limits: Limits::from(st),
        roots: st.search_dirs.iter().map(|d| expand_home(d)).collect(),
        hide_patterns: st.hide_patterns.clone(),
    };
    if CONFIG.set(cfg).is_ok() {
        std::thread::spawn(index_pass);
    }
}

// Same debounce as ocr::schedule_reindex: one pass per burst of changed documents.
const REINDEX_DELAY: Duration = Duration::from_secs(10);
static LAST_DOC_EVENT: Mutex<Option<Instant>> = Mutex::new(None);
static REINDEX_PENDING: AtomicBool = AtomicBool::new(false);

/// True when the `docs` setting is on (the index is built and searched).
pub fn docs_enabled() -> bool {
    CONFIG.get().is_some()
}

/// Called by the files watcher for a created/changed document.
pub fn schedule_reindex() {
    if !docs_enabled() {
        return;
    }
    *LAST_DOC_EVENT.lock().unwrap() = Some(Instant::now());
    if REINDEX_PENDING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| loop {
        let Some(last) = *LAST_DOC_EVENT.lock().unwrap() else { return };
        let wait = REINDEX_DELAY.saturating_sub(last.elapsed());
        if !wait.is_zero() {
            std::thread::sleep(wait);
            continue;
        }
        let started = Instant::now();
        index_pass();
        // Under the lock so an event can't land between the check and the flag.
        let last = LAST_DOC_EVENT.lock().unwrap();
        if last.is_none_or(|t| t <= started) {
            REINDEX_PENDING.store(false, Ordering::Relaxed);
            return;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(root: &Path) -> Config {
        Config { limits: Limits::from(&Settings::default()), roots: vec![root.to_path_buf()], hide_patterns: vec![".".into()] }
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE doc_meta (file_path TEXT PRIMARY KEY, mtime INTEGER NOT NULL);
             CREATE VIRTUAL TABLE doc_fts USING fts5(file_path UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');",
        )
        .unwrap();
        conn
    }

    fn add(conn: &Connection, path: &Path, mtime: i64) {
        conn.execute("INSERT OR REPLACE INTO files (name, path, mtime) VALUES ('x', ?1, ?2)", params![path.to_string_lossy(), mtime]).unwrap();
    }

    fn body(conn: &Connection, path: &Path) -> Option<String> {
        conn.query_row("SELECT body FROM doc_fts WHERE file_path = ?1", [path.to_string_lossy()], |r| r.get(0)).ok()
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("luma_docs_idx_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("node_modules")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        d
    }

    #[test]
    fn rules_for_paths() {
        let c = cfg(Path::new("/home/u/Documents"));
        assert!(allowed(Path::new("/home/u/Documents/a/b.txt"), &c));
        assert!(!allowed(Path::new("/home/u/Documents/node_modules/b.txt"), &c));
        assert!(!allowed(Path::new("/home/u/Documents/.git/b.txt"), &c));
        assert!(!allowed(Path::new("/home/u/Documents/.hidden/b.txt"), &c));
        assert!(!allowed(Path::new("/home/u/Documents/.b.txt"), &c));
        assert!(!allowed(Path::new("/elsewhere/b.txt"), &c));
    }

    #[test]
    fn indexes_once_and_again_after_change() {
        let d = dir("once");
        let (a, e, skip, hid, img) = (d.join("a.txt"), d.join("empty.txt"), d.join("node_modules/s.txt"), d.join(".hidden/h.txt"), d.join("p.png"));
        for (p, t) in [(&a, "leasing contract"), (&e, "  "), (&skip, "x"), (&hid, "x"), (&img, "x")] {
            std::fs::write(p, t).unwrap();
        }
        let mut conn = conn();
        for p in [&a, &e, &skip, &hid, &img] {
            add(&conn, p, 1);
        }
        let c = cfg(&d);
        run_pass(&mut conn, &c, false).unwrap();
        assert_eq!(body(&conn, &a).as_deref(), Some("leasing contract"));
        // no text: meta row only
        assert!(body(&conn, &e).is_none());
        let metas: i64 = conn.query_row("SELECT COUNT(*) FROM doc_meta", [], |r| r.get(0)).unwrap();
        assert_eq!(metas, 2);

        // same mtime: the file is not read again
        std::fs::write(&a, "changed").unwrap();
        run_pass(&mut conn, &c, false).unwrap();
        assert_eq!(body(&conn, &a).as_deref(), Some("leasing contract"));

        // new mtime: replaced, not duplicated
        add(&conn, &a, 2);
        run_pass(&mut conn, &c, false).unwrap();
        assert_eq!(body(&conn, &a).as_deref(), Some("changed"));
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM doc_fts WHERE file_path = ?1", [a.to_string_lossy()], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn prune_drops_rows_of_vanished_files() {
        let d = dir("prune");
        let a = d.join("a.txt");
        std::fs::write(&a, "hello").unwrap();
        let mut conn = conn();
        add(&conn, &a, 1);
        run_pass(&mut conn, &cfg(&d), false).unwrap();
        conn.execute("DELETE FROM files", []).unwrap();
        run_pass(&mut conn, &cfg(&d), false).unwrap();
        assert!(body(&conn, &a).is_some());
        run_pass(&mut conn, &cfg(&d), true).unwrap();
        assert!(body(&conn, &a).is_none());
    }
}
