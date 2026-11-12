// Video part of the ocr worker: after the photos, recognizes text on sampled
// frames of every video without a fresh 'ocr' row in video_meta into
// video_frames.ocr.

use super::inference::OcrModels;
use crate::backend::files::video::{self, frames};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

const KIND: &str = "ocr";

// Replaces the file's frame texts and marks it done at `mtime`. Rows keep
// their vector; a row left with neither vec nor ocr is dropped. Text is
// lowercased like ocr_text (SQLite LIKE folds only ASCII).
fn save(conn: &mut Connection, path: &str, mtime: i64, texts: &[(f64, String)]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("UPDATE video_frames SET ocr = NULL WHERE file_path = ?1", [path])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO video_frames (file_path, t_sec, ocr) VALUES (?1, ?2, ?3)
             ON CONFLICT (file_path, t_sec) DO UPDATE SET ocr = excluded.ocr",
        )?;
        for (t, text) in texts {
            stmt.execute(params![path, t, text.to_lowercase()])?;
        }
    }
    tx.execute("DELETE FROM video_frames WHERE file_path = ?1 AND vec IS NULL AND ocr IS NULL", [path])?;
    tx.execute(
        "INSERT INTO video_meta (file_path, kind, mtime) VALUES (?1, ?2, ?3)
         ON CONFLICT (file_path, kind) DO UPDATE SET mtime = excluded.mtime",
        params![path, KIND, mtime],
    )?;
    tx.commit()?;
    Ok(())
}

// (t_sec, text) per frame that has text. A broken video gives none, and
// save() still marks it so it isn't retried until it changes.
fn recognize_video(models: &mut OcrModels, path: &str, every_sec: u32, max_frames: u32) -> Vec<(f64, String)> {
    let duration = match frames::duration_sec(Path::new(path)) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("video: skip {path}: {err}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for t in frames::timestamps(duration, every_sec, max_frames) {
        let img = match frames::frame_at(Path::new(path), t) {
            Ok(img) => img.to_rgb8(),
            Err(err) => {
                eprintln!("video: {path}: {err}");
                continue;
            }
        };
        // A panicking frame (decoder, ndarray index) is skipped, not the worker.
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| models.recognize_image(img))) {
            Ok(Ok(text)) if !text.is_empty() => out.push((t, text)),
            Ok(Ok(_)) => {}
            Ok(Err(err)) => eprintln!("video: {path}: ocr failed: {err}"),
            Err(_) => eprintln!("video: {path}: ocr panicked at {t:.1}s"),
        }
    }
    out
}

/// Recognizes every stale video; called by the ocr worker after the photos.
/// `done` is bumped once per video (the caller's progress counters).
pub(super) fn index_videos(
    models: &mut OcrModels,
    conn: &mut Connection,
    every_sec: u32,
    max_frames: u32,
    done: &AtomicUsize,
) {
    let mut after = String::new();
    loop {
        let Ok(chunk) = video::fetch_stale(conn, KIND, &after, 16) else { break };
        let Some((last, _)) = chunk.last() else { break };
        after = last.clone();
        for (path, mtime) in &chunk {
            let texts = recognize_video(models, path, every_sec, max_frames);
            if let Err(err) = save(conn, path, *mtime, &texts) {
                eprintln!("video: save {path}: {err}");
            }
            done.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE video_meta (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, PRIMARY KEY (file_path, kind));
             CREATE TABLE video_frames (file_path TEXT NOT NULL, t_sec REAL NOT NULL, vec BLOB, ocr TEXT, PRIMARY KEY (file_path, t_sec));",
        )
        .unwrap();
        conn
    }

    #[test]
    fn save_replaces_text_keeps_vectors_and_marks_done() {
        let mut conn = conn();
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('a', '/v/a.mp4', 9)", []).unwrap();
        // old pass: frame 1 has only text, frame 3 has a vector and text
        conn.execute_batch(
            "INSERT INTO video_frames VALUES ('/v/a.mp4', 1.0, NULL, 'old'), ('/v/a.mp4', 3.0, x'00', 'old');",
        )
        .unwrap();
        save(&mut conn, "/v/a.mp4", 9, &[(2.0, "Hello".into()), (3.0, "WORLD".into())]).unwrap();

        let rows: Vec<(f64, bool, Option<String>)> = conn
            .prepare("SELECT t_sec, vec IS NOT NULL, ocr FROM video_frames ORDER BY t_sec")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        // 1.0 had only text -> gone; 3.0 keeps its vector with the new text
        assert_eq!(rows, [(2.0, false, Some("hello".into())), (3.0, true, Some("world".into()))]);
        // marked done: a rerun skips it
        assert!(video::fetch_stale(&conn, KIND, "", 100).unwrap().is_empty());

        // a broken video (no text) is marked too, so it isn't retried every start
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('b', '/v/b.mp4', 1)", []).unwrap();
        save(&mut conn, "/v/b.mp4", 1, &[]).unwrap();
        assert!(video::fetch_stale(&conn, KIND, "", 100).unwrap().is_empty());
    }
}
