// Speech worker: after clip and ocr, recognizes the audio of every video
// without a fresh 'speech' row in video_meta into speech_fts.

use super::frames::{self, ffmpeg_available};
use super::speech::{ensure_model, SpeechModels};
use crate::backend::apps::db_init::open_conn;
use crate::backend::files::clip::{CLIP_DONE, ONNX_LOCK};
use crate::backend::files::files_search::SCAN_COMPLETE;
use crate::backend::files::ocr::OCR_PASS_DONE;
use crate::backend::files::video;
use crate::backend::settings::Settings;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const KIND: &str = "speech";

// Progress, same polling style as OCR_*; shown by ui/search.rs.
pub static SPEECH_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static SPEECH_DONE: AtomicUsize = AtomicUsize::new(0);

// Videos per db page: recognition is slow, so results reach the db often.
const DB_CHUNK: i64 = 8;

// video_speech_max_min in seconds; set once by start_indexing when speech is on.
static MAX_SEC: OnceLock<f64> = OnceLock::new();

// Replaces the file's segments and marks it done at `mtime`.
fn save(conn: &mut Connection, path: &str, mtime: i64, segments: &[(f64, String)]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM speech_fts WHERE file_path = ?1", [path])?;
    {
        let mut stmt = tx.prepare("INSERT INTO speech_fts (file_path, t_sec, body) VALUES (?1, ?2, ?3)")?;
        for (t, text) in segments {
            stmt.execute(params![path, t, text])?;
        }
    }
    tx.execute(
        "INSERT INTO video_meta (file_path, kind, mtime) VALUES (?1, ?2, ?3)
         ON CONFLICT (file_path, kind) DO UPDATE SET mtime = excluded.mtime",
        params![path, KIND, mtime],
    )?;
    tx.commit()?;
    Ok(())
}

// Segments of one video. Too long, silent or broken videos give none and are
// still marked by save(), so they aren't retried until the file changes.
// shortcut: raising video_speech_max_min does not revisit videos skipped as
// too long until they change.
fn recognize_video(models: &mut SpeechModels, path: &str, max_sec: f64) -> Vec<(f64, String)> {
    let p = Path::new(path);
    match frames::duration_sec(p) {
        Ok(d) if d <= max_sec => {}
        Ok(_) => return Vec::new(),
        Err(err) => {
            eprintln!("speech: skip {path}: {err}");
            return Vec::new();
        }
    }
    if !frames::has_audio(p).unwrap_or(false) {
        return Vec::new();
    }
    // A panicking video (decoder, ndarray index) is skipped, not the worker.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| models.transcribe(p))) {
        Ok(Ok(segments)) => segments,
        Ok(Err(err)) => {
            eprintln!("speech: {path}: {err}");
            Vec::new()
        }
        Err(_) => {
            eprintln!("speech: {path}: panicked, skipped until it changes");
            Vec::new()
        }
    }
}

/// Start the worker in the background; gated by `video` and `video_speech`
/// (off = no model, no RAM, no CPU). The model is downloaded on first
/// enable. No-op without ffmpeg.
pub fn start_indexing(st: &Settings) {
    if !st.video || !st.video_speech {
        return;
    }
    if !ffmpeg_available() {
        eprintln!("speech disabled: ffmpeg/ffprobe not found in PATH");
        return;
    }
    let _ = MAX_SEC.set(st.video_speech_max_min as f64 * 60.0);
    SPEECH_ON.store(true, Ordering::Relaxed);
    std::thread::spawn(|| index_pass(false));
}

/// Whether the `video_speech` setting was on at startup: speech hits are searched.
pub fn speech_enabled() -> bool {
    SPEECH_ON.load(Ordering::Relaxed)
}

// Same debounce as ocr::schedule_reindex: one pass per burst of new videos.
const REINDEX_DELAY: Duration = Duration::from_secs(10);
static SPEECH_ON: AtomicBool = AtomicBool::new(false);
static LAST_VIDEO_EVENT: Mutex<Option<Instant>> = Mutex::new(None);
static REINDEX_PENDING: AtomicBool = AtomicBool::new(false);

/// Called by the files watcher for a created/changed video.
pub fn schedule_reindex() {
    if !SPEECH_ON.load(Ordering::Relaxed) {
        return;
    }
    let mut last = LAST_VIDEO_EVENT.lock().unwrap();
    *last = Some(Instant::now());
    if REINDEX_PENDING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| loop {
        let Some(last) = *LAST_VIDEO_EVENT.lock().unwrap() else { return };
        let wait = REINDEX_DELAY.saturating_sub(last.elapsed());
        if !wait.is_zero() {
            std::thread::sleep(wait);
            continue;
        }
        let started = Instant::now();
        index_pass(true);
        // Under the lock so an event can't land between the check and the flag.
        let last = LAST_VIDEO_EVENT.lock().unwrap();
        if last.is_none_or(|t| t <= started) {
            REINDEX_PENDING.store(false, Ordering::Relaxed);
            return;
        }
    });
}

// One pass; `rerun` = a later pass of the session: skips loading the model
// when there is nothing to recognize.
fn index_pass(rerun: bool) {
    // After clip and ocr are done (or off): one ONNX session at a time.
    while !(CLIP_DONE.load(Ordering::Acquire) && OCR_PASS_DONE.load(Ordering::Acquire)) {
        std::thread::sleep(Duration::from_millis(500));
    }
    let _onnx = ONNX_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(mut conn) = open_conn() else { return };
    let total = video::count_stale(&conn, KIND).unwrap_or(0);
    if total == 0 && rerun {
        return;
    }
    if !ensure_model() {
        return;
    }
    let Some(mut models) = SpeechModels::load() else { return };
    let max_sec = MAX_SEC.get().copied().unwrap_or(0.0);

    if SCAN_COMPLETE.load(Ordering::Relaxed) {
        if let Err(err) = video::delete_orphans(&conn) {
            eprintln!("speech orphan cleanup error: {err}");
        }
    }

    SPEECH_TOTAL.store(total.max(0) as usize, Ordering::Relaxed);
    SPEECH_DONE.store(0, Ordering::Relaxed);
    let mut after = String::new();
    loop {
        let Ok(chunk) = video::fetch_stale(&conn, KIND, &after, DB_CHUNK) else { break };
        let Some((last, _)) = chunk.last() else { break };
        after = last.clone();
        for (path, mtime) in &chunk {
            let segments = recognize_video(&mut models, path, max_sec);
            if let Err(err) = save(&mut conn, path, *mtime, &segments) {
                eprintln!("speech: save {path}: {err}");
            }
            SPEECH_DONE.fetch_add(1, Ordering::Relaxed);
        }
    }
    SPEECH_DONE.store(SPEECH_TOTAL.load(Ordering::Relaxed), Ordering::Relaxed);
    // Free the ORT arenas, then give glibc's freed pages back to the OS.
    drop(models);
    unsafe { libc::malloc_trim(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE video_meta (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, PRIMARY KEY (file_path, kind));
             CREATE VIRTUAL TABLE speech_fts USING fts5(file_path UNINDEXED, t_sec UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn saved_video_is_not_stale_until_it_changes() {
        let mut conn = conn();
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('a', '/v/a.mp4', 5)", []).unwrap();
        assert_eq!(video::count_stale(&conn, KIND).unwrap(), 1);

        save(&mut conn, "/v/a.mp4", 5, &[(0.0, "hello there".into()), (12.5, "the lease ends".into())]).unwrap();
        assert_eq!(video::count_stale(&conn, KIND).unwrap(), 0);
        // a silent or broken video: no segments, but marked
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('b', '/v/b.mkv', 1)", []).unwrap();
        save(&mut conn, "/v/b.mkv", 1, &[]).unwrap();
        assert_eq!(video::count_stale(&conn, KIND).unwrap(), 0);

        // the file changed: stale again, and a re-save replaces the old segments
        conn.execute("UPDATE files SET mtime = 6 WHERE path = '/v/a.mp4'", []).unwrap();
        assert_eq!(video::count_stale(&conn, KIND).unwrap(), 1);
        save(&mut conn, "/v/a.mp4", 6, &[(3.0, "new words".into())]).unwrap();
        let n = |q: &str| -> i64 { conn.query_row(q, [], |r| r.get(0)).unwrap() };
        assert_eq!(n("SELECT COUNT(*) FROM speech_fts WHERE file_path = '/v/a.mp4'"), 1);
        assert_eq!(n("SELECT COUNT(*) FROM speech_fts WHERE speech_fts MATCH 'lease'"), 0);
        assert_eq!(video::count_stale(&conn, KIND).unwrap(), 0);
    }
}
