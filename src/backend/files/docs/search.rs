// Query side of the document index: FTS5 MATCH ranked by bm25().

use anyhow::Result;
use rusqlite::{params, Connection};

pub struct DocHit {
    pub path: String,
    /// A few words around the match, on one line.
    pub snippet: String,
    /// 1-100, bm25 relative to the best hit (the best one is 100).
    pub score: i32,
}

// User text → FTS5 query: every word quoted (so `-`, `"`, AND, NEAR... are
// plain text, not syntax) and all of them required; the last one is a
// prefix, so results follow the typing. None when there are no words.
pub(crate) fn fts_query(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let last = words.len().checked_sub(1)?;
    let parts: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| format!("\"{w}\"{}", if i == last { "*" } else { "" }))
        .collect();
    Some(parts.join(" "))
}

/// Best matches first. English words match across forms (porter stemming:
/// "lease" finds "leasing"); other languages match exact words and prefixes.
pub fn docs_search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<DocHit>> {
    let Some(q) = fts_query(query) else { return Ok(Vec::new()) };
    let mut stmt = conn.prepare(
        "SELECT file_path, snippet(doc_fts, 1, '', '', '…', 12), bm25(doc_fts) FROM doc_fts
         WHERE doc_fts MATCH ?1 ORDER BY bm25(doc_fts) LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![q, limit as i64], |row| {
            let snippet: String = row.get(1)?;
            Ok((DocHit { path: row.get(0)?, snippet: snippet.split_whitespace().collect::<Vec<_>>().join(" "), score: 0 }, row.get::<_, f64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // bm25 is negative, more negative = better; rows come best first
    let best = rows.first().map_or(0.0, |r| r.1);
    Ok(rows
        .into_iter()
        .map(|(mut hit, rank)| {
            hit.score = if best < 0.0 { ((rank / best * 100.0).round() as i32).clamp(1, 100) } else { 100 };
            hit
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(docs: &[(&str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE doc_fts USING fts5(file_path UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');",
        )
        .unwrap();
        for (path, body) in docs {
            conn.execute("INSERT INTO doc_fts VALUES (?1, ?2)", [path, body]).unwrap();
        }
        conn
    }

    fn paths(conn: &Connection, q: &str) -> Vec<String> {
        docs_search(conn, q, 10).unwrap().into_iter().map(|h| h.path).collect()
    }

    #[test]
    fn lease_finds_leasing_and_requires_all_words() {
        let c = conn(&[("/a", "Terms of leasing the flat"), ("/b", "lease only"), ("/c", "unrelated")]);
        assert_eq!(paths(&c, "lease").len(), 2);
        assert_eq!(paths(&c, "lease flat"), ["/a"]);
        assert!(paths(&c, "lease zebra").is_empty());
    }

    #[test]
    fn last_word_is_a_prefix_and_case_and_diacritics_fold() {
        let c = conn(&[("/a", "Договор аренды квартиры"), ("/b", "Café menu")]);
        assert_eq!(paths(&c, "договор аре"), ["/a"]);
        assert_eq!(paths(&c, "ДОГОВОР"), ["/a"]);
        assert_eq!(paths(&c, "cafe"), ["/b"]);
    }

    #[test]
    fn fts_syntax_in_the_query_is_plain_text() {
        let c = conn(&[("/a", "foo-bar baz")]);
        for q in ["\"", "foo AND (", "NEAR(", "col:x", "*", "a-b -c", "", "   ", "---"] {
            assert!(docs_search(&c, q, 10).is_ok(), "{q:?}");
        }
        assert_eq!(paths(&c, "foo-bar"), ["/a"]);
    }

    #[test]
    fn better_match_ranks_first_and_snippet_is_one_line() {
        let long = "filler ".repeat(40);
        let c = conn(&[("/weak", &format!("{long} lease {long}")), ("/strong", "lease lease lease")]);
        let hits = docs_search(&c, "lease", 10).unwrap();
        assert_eq!(hits[0].path, "/strong");
        assert!(hits.iter().all(|h| h.snippet.contains("lease") && !h.snippet.contains('\n')));
        assert_eq!(docs_search(&c, "lease", 1).unwrap().len(), 1);
    }
}
