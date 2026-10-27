// Воркер индексации: после скана файлов выбирает фото без вектора или с
// устаревшим mtime, считает CLIP-эмбеддинги и пишет их в embeddings.

use super::inference::{l2_normalize, models_dir, ClipModels, QueryModels};
use crate::backend::apps::db_init::open_conn;
use crate::backend::files::download::ensure_models;
use crate::backend::files::files_search::{FileEntry, image_where, SCAN_COMPLETE};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

// Progress, same polling style as INDEX_* in files_search; no UI hook yet.
pub static EMB_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static EMB_DONE: AtomicUsize = AtomicUsize::new(0);

// Model download progress in bytes; DL_TOTAL 0 = no download in progress.
pub static DL_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static DL_DONE: AtomicUsize = AtomicUsize::new(0);

// Query side of the models, filled at the end of indexing; None until then
// (or without models/clip/). The vision session is consumed and dropped by
// into_query() there — its arena is the memory hog and queries don't need it.
pub static MODELS: Mutex<Option<QueryModels>> = Mutex::new(None);

// Generation of the newest search query. Every keystroke spawns a worker that
// waits on MODELS; workers whose query got superseded while waiting skip the
// inference instead of queueing in front of the latest one.
static LATEST_QUERY: AtomicU64 = AtomicU64::new(0);

/// Called on the UI thread per keystroke, before spawning the worker, so
/// generations follow typing order whatever order the workers start in.
pub fn next_query() -> u64 {
    LATEST_QUERY.fetch_add(1, Ordering::Relaxed) + 1
}

/// A newer next_query() exists, so a newer worker will post its own results.
pub fn is_stale(generation: u64) -> bool {
    LATEST_QUERY.load(Ordering::Relaxed) != generation
}

// Embeddings + their files, snapshot taken once per startup after indexing.
// Embeddings never change during a session (the watcher does not embed new
// files), so the snapshot needs no invalidation. vec is l2-normalized.
pub struct CachedVec {
    path: String,
    name: String,
    name_lower: Vec<char>,
    mtime: i64,
    vec: Vec<f32>,
}
pub static VECTORS: Mutex<Vec<CachedVec>> = Mutex::new(Vec::new());

// match_kind values stored in FileItem; 0 = by name (the default, no badge).
pub const MATCH_NAME: i32 = 0;
pub const MATCH_SEMANTIC: i32 = 1;
// Measured CLIP ViT-B/32 match band: random pairs ~0.10-0.20, strong hits ~0.27-0.32 ("glowing city" hit 0.2697, control 0.3168, mismatch 0.1881).
const SEMANTIC_MIN_SCORE: f32 = 0.15;
const SEMANTIC_MAX_SCORE: f32 = 0.35;
const SEMANTIC_MAX_HITS: usize = 32;

const KIND: &str = "clip";

// HuggingFace export the app expects: quantized ViT-B/32 (~154 MB total).
const MODEL_FILES: [(&str, &str); 3] = [
    ("tokenizer.json", "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/tokenizer.json"),
    ("vision_model_quantized.onnx", "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/onnx/vision_model_quantized.onnx"),
    ("text_model_quantized.onnx", "https://huggingface.co/Xenova/clip-vit-base-patch32/resolve/main/onnx/text_model_quantized.onnx"),
];

// DB rows read per pass; a multiple of INFERENCE_BATCH.
const DB_CHUNK: usize = 256;

// Cosine similarity -> UI percent: SEMANTIC_MIN_SCORE maps to 0,
// SEMANTIC_MAX_SCORE to 100, values outside are clamped.
fn match_percent(score: f32) -> i32 {
    (((score - SEMANTIC_MIN_SCORE) / (SEMANTIC_MAX_SCORE - SEMANTIC_MIN_SCORE)) * 100.0)
        .round()
        .clamp(0.0, 100.0) as i32
}

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// Snapshot of all clip embeddings joined with their file rows. Photos whose
// file row vanished between scan and here are excluded by the JOIN.
fn load_vectors(conn: &Connection) -> Vec<CachedVec> {
    let sql = format!(
        "SELECT e.file_path, f.name, f.mtime, e.vec FROM embeddings e
         JOIN files f ON f.path = e.file_path WHERE e.kind = '{KIND}'"
    );
    let Ok(rows) = conn
        .prepare(&sql)
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|err| eprintln!("clip vector cache error: {err}"))
    else {
        return Vec::new();
    };
    rows.into_iter()
        .map(|(path, name, mtime, blob)| {
            let mut vec = blob_to_vec(&blob);
            l2_normalize(&mut vec);
            CachedVec {
                name_lower: name.to_lowercase().chars().collect(),
                path,
                name,
                mtime,
                vec,
            }
        })
        .collect()
}

/// Semantic photo search: embeds the query once (~10-50 ms), scores the
/// in-memory vector cache with dot products, best matches first. Each hit
/// comes with its match strength as a 0-100 percent for the UI badge,
/// rescaled linearly from the measured 0.15-0.35 cosine band (raw CLIP
/// scores for good matches stay below ~0.35).
/// Empty when models are not loaded yet (still indexing / no models) or when
/// a newer query (see next_query) arrived while this one waited for them.
pub fn clip_search(query: &str, generation: u64) -> Vec<(FileEntry, i32)> {
    let mut query_vec = {
        let mut guard = MODELS.lock().unwrap();
        if is_stale(generation) {
            return Vec::new();
        }
        let Some(models) = guard.as_mut() else {
            return Vec::new();
        };
        match models.embed_text(query) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("clip query embed error: {err}");
                return Vec::new();
            }
        }
    };
    l2_normalize(&mut query_vec);
    let vectors = VECTORS.lock().unwrap();
    let mut scored: Vec<(f32, &CachedVec)> =
        vectors.iter().map(|c| (dot(&query_vec, &c.vec), c)).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .filter(|(score, _)| *score >= SEMANTIC_MIN_SCORE)
        .take(SEMANTIC_MAX_HITS)
        .map(|(score, c)| {
            (
                FileEntry {
                    name: c.name.clone(),
                    name_lower: c.name_lower.clone(),
                    path: c.path.clone(),
                    is_dir: false,
                    mtime: c.mtime,
                },
                // cutoff -> 0%, perfect match -> 100%
                match_percent(score),
            )
        })
        .collect()
}

// Photos from `files` with no clip-embedding or a stale mtime, paged by path.
fn fetch_candidates(conn: &Connection, after: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    let sql = format!(
        "SELECT f.path, f.mtime FROM files f
         LEFT JOIN embeddings e ON e.file_path = f.path AND e.kind = '{KIND}' AND e.mtime = f.mtime
         WHERE e.file_path IS NULL AND {} AND f.path > ?1
         ORDER BY f.path LIMIT ?2",
        image_where()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![after, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// Embeddings whose file vanished from the rescan.
fn delete_orphans(conn: &Connection) -> Result<()> {
    conn.execute(
        &format!("DELETE FROM embeddings WHERE kind = '{KIND}' AND file_path NOT IN (SELECT path FROM files)"),
        [],
    )?;
    Ok(())
}

fn upsert_batch(conn: &mut Connection, rows: &[((String, i64), Vec<f32>)]) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO embeddings (file_path, kind, mtime, vec) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (file_path, kind) DO UPDATE SET mtime = excluded.mtime, vec = excluded.vec",
        )?;
        for ((path, mtime), vec) in rows {
            stmt.execute(params![path, KIND, mtime, vec_to_blob(vec)])?;
        }
    }
    tx.commit()?;
    Ok(())
}

// Set once the clip worker is over for this session (finished, disabled,
// failed or panicked); the ocr worker waits for it so the two ONNX sessions
// never live and compete for the CPU at the same time.
pub static CLIP_DONE: AtomicBool = AtomicBool::new(false);

struct SetDoneOnDrop;
impl Drop for SetDoneOnDrop {
    fn drop(&mut self) {
        CLIP_DONE.store(true, Ordering::Release);
    }
}

/// Start the indexing worker in the background; gated by the `semantic`
/// setting (off = the feature costs nothing: no models, no RAM, no CPU).
/// On first enable the models are downloaded from HuggingFace. Silently
/// no-ops when the download fails or models/clip/ is broken.
pub fn start_indexing(semantic: bool) {
    if !semantic {
        CLIP_DONE.store(true, Ordering::Release);
        return;
    }
    std::thread::spawn(|| {
        let _done = SetDoneOnDrop;
        if !ensure_models("clip", &models_dir(), &MODEL_FILES, &DL_TOTAL, &DL_DONE) {
            return;
        }
        let Some(mut models) = ClipModels::load() else { return };
        let Ok(mut conn) = open_conn() else { return };

        if SCAN_COMPLETE.load(Ordering::Relaxed) {
            if let Err(err) = delete_orphans(&conn) {
                eprintln!("clip orphan cleanup error: {err}");
            }
        }

        let total_sql = format!(
            "SELECT COUNT(*) FROM files f LEFT JOIN embeddings e
             ON e.file_path = f.path AND e.kind = '{KIND}' AND e.mtime = f.mtime
             WHERE e.file_path IS NULL AND {}",
            image_where()
        );
        let total = conn.query_row(&total_sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
        EMB_TOTAL.store(total.max(0) as usize, Ordering::Relaxed);
        EMB_DONE.store(0, Ordering::Relaxed);

        let mut after = String::new();
        loop {
            let Ok(chunk) = fetch_candidates(&conn, &after, DB_CHUNK as i64) else { break };
            if chunk.is_empty() {
                break;
            }
            let paths: Vec<String> = chunk.iter().map(|(p, _)| p.clone()).collect();
            let vecs = models.embed_images(&paths);
            let pairs: Vec<_> = chunk.iter().cloned().zip(vecs).filter_map(|(k, v)| v.map(|v| (k, v))).collect();
            if let Err(err) = upsert_batch(&mut conn, &pairs) {
                eprintln!("clip upsert error: {err}");
            }
            EMB_DONE.fetch_add(chunk.len(), Ordering::Relaxed);
            after = chunk.last().unwrap().0.clone();
        }
        EMB_DONE.store(EMB_TOTAL.load(Ordering::Relaxed), Ordering::Relaxed);
        // Models + vector snapshot go live only now: a query during indexing
        // would score an incomplete cache. into_query() drops the vision
        // session here, releasing its ORT arena back to the OS.
        *MODELS.lock().unwrap() = Some(models.into_query());
        *VECTORS.lock().unwrap() = load_vectors(&conn);
        // glibc holds the freed pages in its arenas; give them back to the OS.
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
            CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE embeddings (file_path TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind = trim(kind) AND kind <> ''), mtime INTEGER NOT NULL DEFAULT 0, vec BLOB NOT NULL, PRIMARY KEY (file_path, kind));
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn only_the_newest_query_is_fresh() {
        let older = next_query();
        let newer = next_query();
        assert!(is_stale(older));
        assert!(!is_stale(newer));
    }

    #[test]
    fn candidates_need_photo_and_fresh_mtime() {
        let conn = conn();
        for (path, mtime) in [
            ("/d/a.jpg", 5),        // no embedding -> candidate
            ("/d/b.jpg", 5),        // stale embedding -> candidate
            ("/d/c.jpg", 7),        // fresh embedding -> skip
            ("/d/d.txt", 5),        // not a photo -> skip
            ("/d/e.jpg", 5),        // fresh embedding of other kind -> candidate
        ] {
            conn.execute("INSERT INTO files (name, path, mtime) VALUES ('x', ?1, ?2)", params![path, mtime]).unwrap();
        }
        conn.execute("INSERT INTO embeddings VALUES ('/d/b.jpg', 'clip', 4, x'00')", []).unwrap();
        conn.execute("INSERT INTO embeddings VALUES ('/d/c.jpg', 'clip', 7, x'00')", []).unwrap();
        conn.execute("INSERT INTO embeddings VALUES ('/d/e.jpg', 'other', 5, x'00')", []).unwrap();

        let candidates = fetch_candidates(&conn, "", 100).unwrap();
        let paths: Vec<&str> = candidates.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["/d/a.jpg", "/d/b.jpg", "/d/e.jpg"]);

        // path paging excludes everything up to `after`
        let rest = fetch_candidates(&conn, "/d/b.jpg", 100).unwrap();
        assert_eq!(rest.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["/d/e.jpg"]);
    }

    #[test]
    fn orphans_are_removed() {
        let conn = conn();
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('x', '/d/a.jpg', 1)", []).unwrap();
        conn.execute("INSERT INTO embeddings VALUES ('/d/a.jpg', 'clip', 1, x'00')", []).unwrap();
        conn.execute("INSERT INTO embeddings VALUES ('/gone/b.jpg', 'clip', 1, x'00')", []).unwrap();
        conn.execute("INSERT INTO embeddings VALUES ('/gone/c.jpg', 'other', 1, x'00')", []).unwrap();

        delete_orphans(&conn).unwrap();

        let n: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings WHERE kind = 'clip'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let other: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings WHERE kind = 'other'", [], |r| r.get(0)).unwrap();
        assert_eq!(other, 1); // other kinds are not our business
    }

    #[test]
    fn blob_roundtrip() {
        let vec: Vec<f32> = [0.0, -1.5, 3.25, f32::MIN_POSITIVE].into();
        assert_eq!(blob_to_vec(&vec_to_blob(&vec)), vec);
    }

    #[test]
    fn match_percent_rescales_from_band() {
        assert_eq!(match_percent(SEMANTIC_MIN_SCORE), 0);
        assert_eq!(match_percent(SEMANTIC_MAX_SCORE), 100);
        assert_eq!(match_percent(0.25), 50); // midpoint of 0.15..0.35
        assert_eq!(match_percent(0.0), 0); // below cutoff clamps, no negatives
        assert_eq!(match_percent(0.151), 0); // rounds down to 0
    }

    #[test]
    fn cosine_scoring_and_threshold() {
        // Two unnormalized vectors, one query collinear with b.
        let mut a = vec![1.0, 0.0];
        let mut b = vec![0.0, 3.0];
        let mut q = vec![0.0, 0.5];
        for v in [&mut a, &mut b, &mut q] {
            l2_normalize(v);
        }
        // Same direction => score 1.0; orthogonal => ~0.0, below the threshold.
        assert!((dot(&q, &b) - 1.0).abs() < 1e-6);
        assert!(dot(&q, &a) < SEMANTIC_MIN_SCORE);
    }
}
