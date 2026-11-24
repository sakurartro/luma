use super::files_search::FileEntry;
use anyhow::Result;
use rusqlite::{params, Connection};

pub fn insert_batch(batch: &[FileEntry], conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    insert_entries(batch, &tx)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn insert_entries(batch: &[FileEntry], conn: &Connection) -> Result<()> {
    let mut files_stmt = conn.prepare(
        r#"
        INSERT INTO files (name, path, mtime)
        VALUES (?1, ?2, ?3)
        ON CONFLICT (path) DO UPDATE SET name=excluded.name, mtime=excluded.mtime
        "#,
    )?;
    let mut folders_stmt = conn.prepare(
        r#"
        INSERT INTO folders (name, path, mtime)
        VALUES (?1, ?2, ?3)
        ON CONFLICT (path) DO UPDATE SET name=excluded.name, mtime=excluded.mtime
        "#,
    )?;

    for entry in batch {
        let stmt = if entry.is_dir { &mut folders_stmt } else { &mut files_stmt };
        stmt.execute(params![&entry.name, &entry.path, entry.mtime])?;
    }

    Ok(())
}

// Folders first, then files; each sorted by name (case-insensitive).
pub fn get_files(conn: &Connection) -> rusqlite::Result<Vec<FileEntry>> {
    let mut entries: Vec<FileEntry> = Vec::new();
    for (sql, is_dir) in [
        ("SELECT name, path, mtime FROM folders", true),
        ("SELECT name, path, mtime FROM files", false),
    ] {
        let mut stmt = conn.prepare(sql)?;
        let mut rows: Vec<FileEntry> = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let name_lower = name.to_lowercase().chars().collect();
                Ok(FileEntry {
                    name,
                    name_lower,
                    path: row.get(1)?,
                    is_dir,
                    mtime: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // same ordering the lowercase-string sort gave, without per-compare allocs
        rows.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));
        entries.extend(rows);
    }

    Ok(entries)
}

pub fn del_entry(conn: &Connection, path: &str) -> Result<()> {
    conn.execute("DELETE FROM files WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM folders WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM doc_meta WHERE file_path = ?1", [path])?;
    conn.execute("DELETE FROM doc_fts WHERE file_path = ?1", [path])?;
    conn.execute("DELETE FROM video_meta WHERE file_path = ?1", [path])?;
    conn.execute("DELETE FROM video_frames WHERE file_path = ?1", [path])?;
    conn.execute("DELETE FROM speech_fts WHERE file_path = ?1", [path])?;
    Ok(())
}

pub(super) fn del_path(conn: &Connection, path: &str, check_subtree: bool) -> Result<()> {
    if !check_subtree || !conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE path = ?1)",
        [path],
        |row| row.get::<_, bool>(0),
    )? {
        return del_entry(conn, path);
    }

    // Under SQLite's binary collation, [path + "/", path + "0") contains
    // exactly the descendants. This also handles wildcard characters in names.
    let start = format!("{path}/");
    let end = format!("{path}0");
    for (table, column) in [
        ("files", "path"),
        ("folders", "path"),
        ("embeddings", "file_path"),
        ("ocr_text", "file_path"),
        ("doc_meta", "file_path"),
        ("doc_fts", "file_path"),
        ("video_meta", "file_path"),
        ("video_frames", "file_path"),
        ("speech_fts", "file_path"),
    ] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE {column} = ?1 OR ({column} >= ?2 AND {column} < ?3)"),
            params![path, start, end],
        )?;
    }
    Ok(())
}

// Check inside the write transaction so an in-flight indexer cannot restore
// rows the watcher removed while the video was being processed.
pub(super) fn file_is_current(conn: &Connection, path: &str, mtime: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM files WHERE path = ?1 AND mtime = ?2)",
        params![path, mtime],
        |row| row.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::files::files_search::FileEntry;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE embeddings (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, vec BLOB NOT NULL, PRIMARY KEY (file_path, kind));
            CREATE TABLE ocr_text (file_path TEXT PRIMARY KEY, mtime INTEGER NOT NULL, text TEXT NOT NULL);
            CREATE TABLE doc_meta (file_path TEXT PRIMARY KEY, mtime INTEGER NOT NULL);
            CREATE VIRTUAL TABLE doc_fts USING fts5(file_path UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');
            CREATE TABLE video_meta (file_path TEXT NOT NULL, kind TEXT NOT NULL, mtime INTEGER NOT NULL, PRIMARY KEY (file_path, kind));
            CREATE TABLE video_frames (file_path TEXT NOT NULL, t_sec REAL NOT NULL, vec BLOB, ocr TEXT, PRIMARY KEY (file_path, t_sec));
            CREATE VIRTUAL TABLE speech_fts USING fts5(file_path UNINDEXED, t_sec UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn insert_get_del_roundtrip() {
        let mut conn = conn();
        insert_batch(
            &[
                FileEntry { name: "b.txt".into(), name_lower: vec!['b', '.', 't', 'x', 't'], path: "/d/b.txt".into(), is_dir: false, mtime: 200 },
                FileEntry { name: "sub".into(), name_lower: vec!['s', 'u', 'b'], path: "/d/sub".into(), is_dir: true, mtime: 300 },
                FileEntry { name: "a.txt".into(), name_lower: vec!['a', '.', 't', 'x', 't'], path: "/d/a.txt".into(), is_dir: false, mtime: 100 },
            ],
            &mut conn,
        )
        .unwrap();

        let files = get_files(&conn).unwrap();
        assert_eq!(files.len(), 3);
        // folders first, then files sorted by name
        assert!(files[0].is_dir);
        assert_eq!(files[1].name, "a.txt");
        assert_eq!(files[2].name, "b.txt");
        assert_eq!(files[2].mtime, 200);

        del_entry(&conn, "/d/a.txt").unwrap();
        let files = get_files(&conn).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "/d/b.txt");
    }

    #[test]
    fn del_entry_drops_document_index() {
        let conn = conn();
        conn.execute_batch(
            "INSERT INTO doc_meta VALUES ('/d/a.txt', 1), ('/d/b.txt', 1);
             INSERT INTO doc_fts VALUES ('/d/a.txt', 'lease agreement'), ('/d/b.txt', 'lease');",
        )
        .unwrap();
        let hits = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM doc_fts WHERE doc_fts MATCH 'lease'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(hits(&conn), 2);
        del_entry(&conn, "/d/a.txt").unwrap();
        assert_eq!(hits(&conn), 1);
        let meta: i64 = conn.query_row("SELECT COUNT(*) FROM doc_meta", [], |r| r.get(0)).unwrap();
        assert_eq!(meta, 1);
    }

    #[test]
    fn del_entry_drops_video_index() {
        let conn = conn();
        conn.execute_batch(
            "INSERT INTO video_meta VALUES ('/v/a.mp4', 'clip', 1), ('/v/a.mp4', 'ocr', 1), ('/v/b.mp4', 'clip', 1);
             INSERT INTO speech_fts VALUES ('/v/a.mp4', 5.0, 'hello'), ('/v/b.mp4', 5.0, 'world');
             INSERT INTO video_frames VALUES ('/v/a.mp4', 5.0, x'00', 'hi'), ('/v/a.mp4', 15.0, NULL, NULL), ('/v/b.mp4', 5.0, x'00', NULL);",
        )
        .unwrap();
        del_entry(&conn, "/v/a.mp4").unwrap();
        let left = |t: &str| -> Vec<String> {
            let mut stmt = conn.prepare(&format!("SELECT DISTINCT file_path FROM {t}")).unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(left("video_meta"), ["/v/b.mp4"]);
        assert_eq!(left("video_frames"), ["/v/b.mp4"]);
        assert_eq!(left("speech_fts"), ["/v/b.mp4"]);
    }

    #[test]
    fn del_path_removes_only_the_indexed_subtree() {
        let mut conn = conn();
        conn.execute_batch(
            "INSERT INTO folders (name, path) VALUES
                 ('sample[1]', '/d/sample[1]'), ('nested', '/d/sample[1]/nested'),
                 ('sample[1]-backup', '/d/sample[1]-backup'), ('sample1', '/d/sample1');
             INSERT INTO files (name, path) VALUES
                 ('a.jpg', '/d/sample[1]/nested/a.jpg'),
                 ('b.jpg', '/d/sample[1]-backup/b.jpg'), ('c.jpg', '/d/sample1/c.jpg');
             INSERT INTO embeddings SELECT path, 'clip', 1, x'00' FROM files;
             INSERT INTO ocr_text SELECT path, 1, 'test' FROM files;
             INSERT INTO doc_meta SELECT path, 1 FROM files;
             INSERT INTO doc_fts SELECT path, 'test' FROM files;
             INSERT INTO video_meta SELECT path, 'ocr', 1 FROM files;
             INSERT INTO video_frames SELECT path, 0, NULL, 'test' FROM files;
             INSERT INTO speech_fts SELECT path, 0, 'test' FROM files;",
        )
        .unwrap();

        let tx = conn.transaction().unwrap();
        del_path(&tx, "/d/sample[1]", true).unwrap();
        del_path(&tx, "/d/sample[1]", true).unwrap();
        tx.commit().unwrap();

        let paths = |table: &str, column: &str| {
            let mut stmt = conn.prepare(&format!("SELECT {column} FROM {table} ORDER BY 1")).unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(paths("folders", "path"), ["/d/sample1", "/d/sample[1]-backup"]);
        let files = ["/d/sample1/c.jpg", "/d/sample[1]-backup/b.jpg"];
        assert_eq!(paths("files", "path"), files);
        for table in ["embeddings", "ocr_text", "doc_meta", "doc_fts", "video_meta", "video_frames", "speech_fts"] {
            assert_eq!(paths(table, "file_path"), files);
        }
    }
}
