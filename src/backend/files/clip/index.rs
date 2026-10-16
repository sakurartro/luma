// Воркер индексации: после скана файлов выбирает фото без вектора или с
// устаревшим mtime, считает CLIP-эмбеддинги и пишет их в embeddings.

use super::inference::ClipModels;
use crate::backend::apps::db_init::open_conn;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::sync::atomic::{AtomicUsize, Ordering};

// Progress, same polling style as INDEX_* in files_search; no UI hook yet.
pub static EMB_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static EMB_DONE: AtomicUsize = AtomicUsize::new(0);

const KIND: &str = "clip";
const CANDIDATE_WHERE: &str = "e.file_path IS NULL AND (f.path LIKE '%.jpg' \
     OR f.path LIKE '%.jpeg' OR f.path LIKE '%.png' OR f.path LIKE '%.webp' \
     OR f.path LIKE '%.gif' OR f.path LIKE '%.bmp')";
// DB rows read per pass; a multiple of INFERENCE_BATCH.
const DB_CHUNK: usize = 256;

fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// Photos from `files` with no clip-embedding or a stale mtime, paged by path.
fn fetch_candidates(conn: &Connection, after: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    let sql = format!(
        "SELECT f.path, f.mtime FROM files f
         LEFT JOIN embeddings e ON e.file_path = f.path AND e.kind = '{KIND}' AND e.mtime = f.mtime
         WHERE {CANDIDATE_WHERE} AND f.path > ?1
         ORDER BY f.path LIMIT ?2"
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

/// Start the indexing worker in the background; silently no-ops when
/// models/clip/ is missing.
pub fn start_indexing() {
    std::thread::spawn(|| {
        let Some(mut models) = ClipModels::load() else { return };
        let Ok(mut conn) = open_conn() else { return };

        if let Err(err) = delete_orphans(&conn) {
            eprintln!("clip orphan cleanup error: {err}");
        }

        let total_sql = format!(
            "SELECT COUNT(*) FROM files f LEFT JOIN embeddings e
             ON e.file_path = f.path AND e.kind = '{KIND}' AND e.mtime = f.mtime
             WHERE {CANDIDATE_WHERE}"
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
}
