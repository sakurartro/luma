// Search inside videos: frames (CLIP, OCR) in video_meta + video_frames.

pub mod frames;
pub mod speech;
pub mod vad;

use crate::backend::files::files_search::video_where;
use anyhow::Result;
use rusqlite::{params, Connection};

/// Drops everything indexed from videos (the `video` setting went off).
pub fn clear(conn: &Connection) -> anyhow::Result<()> {
    Ok(conn.execute_batch("DELETE FROM video_meta; DELETE FROM video_frames; DELETE FROM speech_fts;")?)
}

/// Drops only the recognized speech (the `video_speech` setting went off).
pub fn clear_speech(conn: &Connection) -> anyhow::Result<()> {
    Ok(conn.execute_batch("DELETE FROM speech_fts; DELETE FROM video_meta WHERE kind = 'speech';")?)
}

// Videos not processed by `kind` ('clip' | 'ocr') at their current mtime.
fn stale_join() -> String {
    format!(
        "FROM files f LEFT JOIN video_meta m
         ON m.file_path = f.path AND m.kind = ?1 AND m.mtime = f.mtime
         WHERE m.file_path IS NULL AND {}",
        video_where()
    )
}

pub fn count_stale(conn: &Connection, kind: &str) -> Result<i64> {
    Ok(conn.query_row(&format!("SELECT COUNT(*) {}", stale_join()), [kind], |r| r.get(0))?)
}

/// Stale videos (path, mtime), paged by path.
pub fn fetch_stale(conn: &Connection, kind: &str, after: &str, limit: i64) -> Result<Vec<(String, i64)>> {
    let sql = format!("SELECT f.path, f.mtime {} AND f.path > ?2 ORDER BY f.path LIMIT ?3", stale_join());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![kind, after, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every video row of a file that vanished from the rescan, whichever source
/// wrote it: the file is gone for all of them.
pub fn delete_orphans(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM video_meta WHERE file_path NOT IN (SELECT path FROM files);
         DELETE FROM video_frames WHERE file_path NOT IN (SELECT path FROM files);
         DELETE FROM speech_fts WHERE file_path NOT IN (SELECT path FROM files);",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE video_meta (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, PRIMARY KEY (file_path, kind));
             CREATE TABLE video_frames (file_path TEXT NOT NULL, t_sec REAL NOT NULL, vec BLOB, ocr TEXT, PRIMARY KEY (file_path, t_sec));
             CREATE VIRTUAL TABLE speech_fts USING fts5(file_path UNINDEXED, t_sec UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn stale_needs_video_and_fresh_mtime_of_the_same_kind() {
        let conn = conn();
        for (path, mtime) in [
            ("/v/a.mp4", 5), // never processed -> stale
            ("/v/b.mkv", 5), // processed at an old mtime -> stale
            ("/v/c.mp4", 7), // fresh -> skip
            ("/v/d.jpg", 5), // not a video -> skip
            ("/v/e.mov", 5), // fresh for ocr only -> stale for clip
        ] {
            conn.execute("INSERT INTO files (name, path, mtime) VALUES ('x', ?1, ?2)", params![path, mtime]).unwrap();
        }
        conn.execute_batch(
            "INSERT INTO video_meta VALUES ('/v/b.mkv', 'clip', 4), ('/v/c.mp4', 'clip', 7), ('/v/c.mp4', 'ocr', 7), ('/v/e.mov', 'ocr', 5);",
        )
        .unwrap();
        let paths = |kind| -> Vec<String> {
            fetch_stale(&conn, kind, "", 100).unwrap().into_iter().map(|(p, _)| p).collect()
        };
        assert_eq!(paths("clip"), ["/v/a.mp4", "/v/b.mkv", "/v/e.mov"]);
        assert_eq!(paths("ocr"), ["/v/a.mp4", "/v/b.mkv"]);
        assert_eq!(count_stale(&conn, "clip").unwrap(), 3);
        assert_eq!(fetch_stale(&conn, "clip", "/v/b.mkv", 100).unwrap().len(), 1);
    }

    #[test]
    fn orphans_of_every_source_are_removed() {
        let conn = conn();
        conn.execute("INSERT INTO files (name, path, mtime) VALUES ('a', '/v/a.mp4', 1)", []).unwrap();
        conn.execute_batch(
            "INSERT INTO video_meta VALUES ('/v/a.mp4', 'clip', 1), ('/gone.mp4', 'clip', 1), ('/gone.mp4', 'ocr', 1);
             INSERT INTO video_frames VALUES ('/v/a.mp4', 1.0, x'00', NULL), ('/gone.mp4', 1.0, x'00', 'x');
             INSERT INTO speech_fts VALUES ('/v/a.mp4', 1.0, 'kept'), ('/gone.mp4', 1.0, 'dropped');",
        )
        .unwrap();
        delete_orphans(&conn).unwrap();
        let count = |t: &str| -> i64 { conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap() };
        assert_eq!((count("video_meta"), count("video_frames"), count("speech_fts")), (1, 1, 1));
    }

    #[test]
    fn clear_empties_all_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE video_meta (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, PRIMARY KEY (file_path, kind));
             CREATE TABLE video_frames (file_path TEXT NOT NULL, t_sec REAL NOT NULL, vec BLOB, ocr TEXT, PRIMARY KEY (file_path, t_sec));
             CREATE VIRTUAL TABLE speech_fts USING fts5(file_path UNINDEXED, t_sec UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');
             INSERT INTO video_meta VALUES ('/v/a.mp4', 'clip', 1);
             INSERT INTO video_frames VALUES ('/v/a.mp4', 5.0, x'00', NULL);
             INSERT INTO speech_fts VALUES ('/v/a.mp4', 5.0, 'hi');",
        )
        .unwrap();
        clear(&conn).unwrap();
        let count = |t: &str| -> i64 { conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap() };
        assert_eq!((count("video_meta"), count("video_frames"), count("speech_fts")), (0, 0, 0));
    }

    #[test]
    fn speech_roundtrip_insert_search_clear() {
        let conn = conn();
        conn.execute_batch(
            "INSERT INTO video_meta VALUES ('/v/a.mp4', 'clip', 1), ('/v/a.mp4', 'speech', 1);
             INSERT INTO video_frames VALUES ('/v/a.mp4', 5.0, x'00', NULL);
             INSERT INTO speech_fts VALUES ('/v/a.mp4', 12.5, 'we signed the leasing contract');",
        )
        .unwrap();
        // porter stemming: "lease" matches "leasing"
        let hit: (String, f64) = conn
            .query_row("SELECT file_path, t_sec FROM speech_fts WHERE speech_fts MATCH 'lease'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(hit, ("/v/a.mp4".into(), 12.5));
        clear_speech(&conn).unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM speech_fts"), 0);
        // frames and the other sources' meta stay
        assert_eq!(count("SELECT COUNT(*) FROM video_meta"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM video_frames"), 1);
    }
}
