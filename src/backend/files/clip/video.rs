// Video part of the clip worker: after the photos, embeds sampled frames of
// every video without a fresh 'clip' row in video_meta into video_frames.vec.

use super::inference::{image_tensor, ClipModels, INFERENCE_BATCH};
use super::index::vec_to_blob;
use crate::backend::files::video::{self, frames};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

// Progress of the video part, polled by the UI like EMB_*.
pub static VID_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static VID_DONE: AtomicUsize = AtomicUsize::new(0);

const KIND: &str = "clip";

// Replaces the file's frame vectors and marks it done at `mtime`. Rows keep
// their ocr text; a row left with neither vec nor ocr (old timestamp after the
// video changed) is dropped.
fn save(conn: &mut Connection, path: &str, mtime: i64, vecs: &[(f64, Vec<f32>)]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("UPDATE video_frames SET vec = NULL WHERE file_path = ?1", [path])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO video_frames (file_path, t_sec, vec) VALUES (?1, ?2, ?3)
             ON CONFLICT (file_path, t_sec) DO UPDATE SET vec = excluded.vec",
        )?;
        for (t, vec) in vecs {
            stmt.execute(params![path, t, vec_to_blob(vec)])?;
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

// (t_sec, vector) per frame that decoded and embedded. A broken video gives
// none, and save() still marks it so it isn't retried until it changes.
fn embed_video(models: &mut ClipModels, path: &str, every_sec: u32, max_frames: u32) -> Vec<(f64, Vec<f32>)> {
    let duration = match frames::duration_sec(Path::new(path)) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("video: skip {path}: {err}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for group in frames::timestamps(duration, every_sec, max_frames).chunks(INFERENCE_BATCH) {
        // full-size frame dropped right after the 224x224 tensor is made
        let (ts, tensors): (Vec<f64>, Vec<_>) = group
            .iter()
            .filter_map(|&t| match frames::frame_at(Path::new(path), t) {
                Ok(img) => Some((t, image_tensor(&img))),
                Err(err) => {
                    eprintln!("video: {path}: {err}");
                    None
                }
            })
            .unzip();
        if tensors.is_empty() {
            continue;
        }
        match models.embed_tensors(&tensors) {
            Ok(rows) => out.extend(ts.into_iter().zip(rows)),
            Err(err) => eprintln!("video: {path}: inference failed: {err}"),
        }
    }
    out
}

/// Embeds every stale video; called by the clip worker after the photos.
pub(super) fn index_videos(models: &mut ClipModels, conn: &mut Connection, every_sec: u32, max_frames: u32) {
    VID_TOTAL.store(video::count_stale(conn, KIND).unwrap_or(0).max(0) as usize, Ordering::Relaxed);
    VID_DONE.store(0, Ordering::Relaxed);
    let mut after = String::new();
    loop {
        let Ok(chunk) = video::fetch_stale(conn, KIND, &after, 16) else { break };
        if chunk.is_empty() {
            break;
        }
        for (path, mtime) in &chunk {
            let vecs = embed_video(models, path, every_sec, max_frames);
            if let Err(err) = save(conn, path, *mtime, &vecs) {
                eprintln!("video: save {path}: {err}");
            }
            VID_DONE.fetch_add(1, Ordering::Relaxed);
        }
        after = chunk.last().unwrap().0.clone();
    }
    VID_DONE.store(VID_TOTAL.load(Ordering::Relaxed), Ordering::Relaxed);
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
    fn save_replaces_vectors_keeps_ocr_and_marks_done() {
        let mut conn = conn();
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('a', '/v/a.mp4', 9)", []).unwrap();
        // old pass: frames at 1 and 3, frame 3 also has ocr text
        conn.execute_batch(
            "INSERT INTO video_frames VALUES ('/v/a.mp4', 1.0, x'00', NULL), ('/v/a.mp4', 3.0, x'00', 'hi');",
        )
        .unwrap();
        save(&mut conn, "/v/a.mp4", 9, &[(2.0, vec![1.0]), (3.0, vec![2.0])]).unwrap();

        let rows: Vec<(f64, bool, Option<String>)> = conn
            .prepare("SELECT t_sec, vec IS NOT NULL, ocr FROM video_frames ORDER BY t_sec")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        // 1.0 had only a vector -> gone; 3.0 keeps its ocr text with the new vector
        assert_eq!(rows, [(2.0, true, None), (3.0, true, Some("hi".into()))]);
        // marked done: a rerun skips it
        assert!(video::fetch_stale(&conn, KIND, "", 100).unwrap().is_empty());

        // a broken video (no frames) is marked too, so it isn't retried every start
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('b', '/v/b.mp4', 1)", []).unwrap();
        save(&mut conn, "/v/b.mp4", 1, &[]).unwrap();
        assert!(video::fetch_stale(&conn, KIND, "", 100).unwrap().is_empty());
    }

    // Needs the clip models in ~/.config/luma/models/clip and a real video:
    //   VIDEO=~/Videos/x.mp4 cargo test -- --ignored embed_real_video --nocapture
    #[test]
    #[ignore]
    fn embed_real_video() {
        let path = std::env::var("VIDEO").unwrap();
        let mut models = ClipModels::load().expect("no clip models");
        let start = std::time::Instant::now();
        let vecs = embed_video(&mut models, &path, 10, 32);
        println!("{path}: {} frames in {:?}", vecs.len(), start.elapsed());
        assert!(!vecs.is_empty());
        assert!(vecs.iter().all(|(_, v)| v.len() == vecs[0].1.len()));
    }
}
