use super::files_search::FileEntry;
use anyhow::Result;
use rusqlite::{params, Connection};

pub fn insert_batch(batch: &[FileEntry], conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    {
        let mut files_stmt = tx.prepare(
            r#"
            INSERT INTO files (name, path)
            VALUES (?1, ?2)
            ON CONFLICT (path) DO UPDATE SET name=excluded.name
            "#,
        )?;
        let mut folders_stmt = tx.prepare(
            r#"
            INSERT INTO folders (name, path)
            VALUES (?1, ?2)
            ON CONFLICT (path) DO UPDATE SET name=excluded.name
            "#,
        )?;

        for entry in batch {
            let stmt = if entry.is_dir { &mut folders_stmt } else { &mut files_stmt };
            stmt.execute(params![&entry.name, &entry.path])?;
        }
    }

    tx.commit()?;

    Ok(())
}

// Folders first, then files; each sorted by name (case-insensitive).
pub fn get_files(conn: &Connection) -> rusqlite::Result<Vec<FileEntry>> {
    let mut entries: Vec<FileEntry> = Vec::new();
    for (sql, is_dir) in [
        ("SELECT name, path FROM folders", true),
        ("SELECT name, path FROM files", false),
    ] {
        let mut stmt = conn.prepare(sql)?;
        let mut rows: Vec<FileEntry> = stmt
            .query_map([], |row| {
                Ok(FileEntry {
                    name: row.get(0)?,
                    path: row.get(1)?,
                    is_dir,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
            CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE);
            CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE);
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
                FileEntry { name: "b.txt".into(), path: "/d/b.txt".into(), is_dir: false },
                FileEntry { name: "sub".into(), path: "/d/sub".into(), is_dir: true },
                FileEntry { name: "a.txt".into(), path: "/d/a.txt".into(), is_dir: false },
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

        del_entry(&conn, "/d/a.txt".into()).unwrap();
        let files = get_files(&conn).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "/d/b.txt");
    }
}