use super::files_search::FileEntry;
use anyhow::Result;
use rusqlite::{params, Connection};

pub fn insert_batch(batch: &[FileEntry], conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    {
        let mut files_stmt = tx.prepare(
            r#"
            INSERT INTO files (name, path, mtime)
            VALUES (?1, ?2, ?3)
            ON CONFLICT (path) DO UPDATE SET name=excluded.name, mtime=excluded.mtime
            "#,
        )?;
        let mut folders_stmt = tx.prepare(
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
    }

    tx.commit()?;

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

pub fn del_entry(conn: &Connection, path: String) -> Result<()> {
    conn.execute("DELETE FROM files WHERE path = ?1", [&path])?;
    conn.execute("DELETE FROM folders WHERE path = ?1", [&path])?;
    Ok(())
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

        del_entry(&conn, "/d/a.txt".into()).unwrap();
        let files = get_files(&conn).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "/d/b.txt");
    }
}