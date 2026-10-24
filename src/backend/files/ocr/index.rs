// Воркер индексации: после скана файлов выбирает картинки без OCR-текста или
// с устаревшим mtime, распознаёт текст и пишет его в ocr_text.

use super::inference::{models_dir, OcrModels};
use super::search;
use crate::backend::apps::db_init::open_conn;
use crate::backend::files::clip::CLIP_DONE;
use crate::backend::files::download::ensure_models;
use crate::backend::files::files_search::{IMAGE_WHERE, SCAN_COMPLETE};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::atomic::{AtomicUsize, Ordering};

// Progress, same polling style as EMB_* in clip; shown by main.rs.
pub static OCR_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static OCR_DONE: AtomicUsize = AtomicUsize::new(0);

// Model download progress in bytes; OCR_DL_TOTAL 0 = no download in progress.
pub static OCR_DL_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static OCR_DL_DONE: AtomicUsize = AtomicUsize::new(0);

// English PP-OCRv4 (~12 MB total): detector from the RapidOCR repo (it finds
// text regardless of language, the recognizer + dict carry the language),
// recognizer and its dict from other HF repos. Local names are fixed.
const MODEL_FILES: [(&str, &str); 3] = [
    ("det.onnx", "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_det_infer.onnx"),
    ("rec.onnx", "https://huggingface.co/breezedeus/cnocr-ppocr-en_PP-OCRv4/resolve/main/en_PP-OCRv4_rec_infer.onnx"),
    ("dict.txt", "https://huggingface.co/deepghs/paddleocr/resolve/main/rec/en_PP-OCRv4_rec/dict.txt"),
];

// Images per pass: OCR is slow (hundreds of ms per image), so keep chunks
// small — results reach the db often and an interrupted run loses little.
const DB_CHUNK: i64 = 32;

// Images from `files` with no ocr_text row or a stale mtime, paged by path.
fn fetch_candidates(conn: &Connection, after: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    let sql = format!(
        "SELECT f.path, f.mtime FROM files f
         LEFT JOIN ocr_text o ON o.file_path = f.path AND o.mtime = f.mtime
         WHERE o.file_path IS NULL AND {IMAGE_WHERE} AND f.path > ?1
         ORDER BY f.path LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![after, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// Texts whose file vanished from the rescan.
fn delete_orphans(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM ocr_text WHERE file_path NOT IN (SELECT path FROM files)", [])?;
    Ok(())
}

// Text is stored lowercased: SQLite lower()/LIKE fold only ASCII, so the
// search side lowercases the query in Rust and matches with plain LIKE.
fn upsert_batch(conn: &mut Connection, rows: &[(String, i64, String)]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO ocr_text (file_path, mtime, text) VALUES (?1, ?2, ?3)
             ON CONFLICT (file_path) DO UPDATE SET mtime = excluded.mtime, text = excluded.text",
        )?;
        for (path, mtime, text) in rows {
            stmt.execute(params![path, mtime, text.to_lowercase()])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Start the indexing worker in the background; gated by the `ocr` setting
/// (off = the feature costs nothing: no models, no RAM, no CPU). On first
/// enable the models are downloaded from HuggingFace. Silently no-ops when
/// the download fails or models/ocr/ is broken.
pub fn start_indexing(ocr: bool) {
    if !ocr {
        return;
    }
    std::thread::spawn(|| {
        // After clip is done (or off): one ONNX session and one CPU-heavy
        // worker at a time. Needs clip::start_indexing to be called, it sets
        // the flag even when `semantic` is off.
        while !CLIP_DONE.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if !ensure_models("ocr", &models_dir(), &MODEL_FILES, &OCR_DL_TOTAL, &OCR_DL_DONE) {
            return;
        }
        let Some(mut models) = OcrModels::load() else { return };
        let Ok(mut conn) = open_conn() else { return };

        if SCAN_COMPLETE.load(Ordering::Relaxed) {
            if let Err(err) = delete_orphans(&conn) {
                eprintln!("ocr orphan cleanup error: {err}");
            }
        }

        // Texts from earlier runs are searchable while the rest is indexed.
        search::reload(&conn);

        let total_sql = format!(
            "SELECT COUNT(*) FROM files f LEFT JOIN ocr_text o
             ON o.file_path = f.path AND o.mtime = f.mtime
             WHERE o.file_path IS NULL AND {IMAGE_WHERE}"
        );
        let total = conn.query_row(&total_sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
        OCR_TOTAL.store(total.max(0) as usize, Ordering::Relaxed);
        OCR_DONE.store(0, Ordering::Relaxed);

        let mut after = String::new();
        loop {
            let Ok(chunk) = fetch_candidates(&conn, &after, DB_CHUNK) else { break };
            let Some((last, _)) = chunk.last() else { break };
            after = last.clone();
            // A failed file is skipped without a row, so the next start retries it.
            // A panicking one (decoder, ndarray index) gets an empty row instead:
            // otherwise every start would die on it and never reach the files after.
            let rows: Vec<(String, i64, String)> = chunk
                .iter()
                .filter_map(|(path, mtime)| {
                    let path_ref = std::path::Path::new(path);
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| models.recognize(path_ref))) {
                        Ok(Ok(text)) => Some((path.clone(), *mtime, text)),
                        Ok(Err(err)) => {
                            eprintln!("ocr {path}: {err}");
                            None
                        }
                        Err(_) => {
                            eprintln!("ocr {path}: panicked, skipped until it changes");
                            Some((path.clone(), *mtime, String::new()))
                        }
                    }
                })
                .collect();
            if let Err(err) = upsert_batch(&mut conn, &rows) {
                eprintln!("ocr upsert error: {err}");
            }
            OCR_DONE.fetch_add(chunk.len(), Ordering::Relaxed);
        }
        OCR_DONE.store(OCR_TOTAL.load(Ordering::Relaxed), Ordering::Relaxed);
        search::reload(&conn);
        // Free the ORT arenas, then give glibc's freed pages back to the OS.
        drop(models);
        unsafe { libc::malloc_trim(0) };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE ocr_text (file_path TEXT PRIMARY KEY, mtime INTEGER NOT NULL DEFAULT 0, text TEXT NOT NULL);
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn candidates_need_image_and_fresh_mtime() {
        let conn = conn();
        for (path, mtime) in [
            ("/d/a.png", 5), // no text row -> candidate
            ("/d/b.png", 5), // stale row -> candidate
            ("/d/c.png", 7), // fresh row (even empty text) -> skip
            ("/d/d.txt", 5), // not an image -> skip
        ] {
            conn.execute("INSERT INTO files (name, path, mtime) VALUES ('x', ?1, ?2)", params![path, mtime]).unwrap();
        }
        conn.execute("INSERT INTO ocr_text VALUES ('/d/b.png', 4, 'old')", []).unwrap();
        conn.execute("INSERT INTO ocr_text VALUES ('/d/c.png', 7, '')", []).unwrap();

        let candidates = fetch_candidates(&conn, "", 100).unwrap();
        let paths: Vec<&str> = candidates.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["/d/a.png", "/d/b.png"]);

        // path paging excludes everything up to `after`
        let rest = fetch_candidates(&conn, "/d/a.png", 100).unwrap();
        assert_eq!(rest.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["/d/b.png"]);
    }

    #[test]
    fn orphans_are_removed() {
        let conn = conn();
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('x', '/d/a.png', 1)", []).unwrap();
        conn.execute("INSERT INTO ocr_text VALUES ('/d/a.png', 1, 'a')", []).unwrap();
        conn.execute("INSERT INTO ocr_text VALUES ('/gone/b.png', 1, 'b')", []).unwrap();

        delete_orphans(&conn).unwrap();

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM ocr_text", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn upsert_replaces_and_lowercases() {
        let mut conn = conn();
        upsert_batch(&mut conn, &[("/d/a.png".into(), 1, "Hello МИР".into())]).unwrap();
        upsert_batch(&mut conn, &[("/d/a.png".into(), 2, "Второй".into())]).unwrap();

        let (mtime, text): (i64, String) = conn
            .query_row("SELECT mtime, text FROM ocr_text WHERE file_path = '/d/a.png'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((mtime, text.as_str()), (2, "второй"));
    }
}
