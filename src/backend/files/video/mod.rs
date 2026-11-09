// Search inside videos: frames (CLIP, OCR) in video_meta + video_frames.

use rusqlite::Connection;

/// Drops everything indexed from videos (the `video` setting went off).
pub fn clear(conn: &Connection) -> anyhow::Result<()> {
    Ok(conn.execute_batch("DELETE FROM video_meta; DELETE FROM video_frames;")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_empties_both_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE video_meta (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, PRIMARY KEY (file_path, kind));
             CREATE TABLE video_frames (file_path TEXT NOT NULL, t_sec REAL NOT NULL, vec BLOB, ocr TEXT, PRIMARY KEY (file_path, t_sec));
             INSERT INTO video_meta VALUES ('/v/a.mp4', 'clip', 1);
             INSERT INTO video_frames VALUES ('/v/a.mp4', 5.0, x'00', NULL);",
        )
        .unwrap();
        clear(&conn).unwrap();
        let count = |t: &str| -> i64 { conn.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap() };
        assert_eq!((count("video_meta"), count("video_frames")), (0, 0));
    }
}
