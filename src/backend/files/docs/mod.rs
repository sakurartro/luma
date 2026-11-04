// Full-text search inside documents (SQLite FTS5): doc_meta + doc_fts.

pub mod extract;
mod index;
mod search;

pub use extract::is_doc;
pub use search::docs_search;
pub use index::{docs_enabled, schedule_reindex, start_indexing, DOCS_DONE, DOCS_TOTAL};

use rusqlite::Connection;

// match_kind in FileItem: the file was found by the text inside it.
pub const MATCH_DOC: i32 = 3;

/// Drops everything indexed from documents (the `docs` setting went off).
pub fn clear(conn: &Connection) -> anyhow::Result<()> {
    Ok(conn.execute_batch("DELETE FROM doc_meta; DELETE FROM doc_fts;")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_empties_both_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE doc_meta (file_path TEXT PRIMARY KEY, mtime INTEGER NOT NULL);
             CREATE VIRTUAL TABLE doc_fts USING fts5(file_path UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');
             INSERT INTO doc_meta VALUES ('/d/a.txt', 1);
             INSERT INTO doc_fts VALUES ('/d/a.txt', 'hello');",
        )
        .unwrap();
        clear(&conn).unwrap();
        let n: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM doc_meta) + (SELECT COUNT(*) FROM doc_fts)", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
