// Query side of the speech index: FTS5 MATCH ranked by bm25(), one hit per
// video (its best segment) with the segment's start time.

use crate::backend::files::docs::fts_query;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashSet;

pub struct SpeechHit {
    pub path: String,
    /// Where the matching segment starts, seconds.
    pub t_sec: f64,
    /// A few words around the match, on one line.
    pub snippet: String,
    /// 1-100, bm25 relative to the best hit (the best one is 100).
    pub score: i32,
}

// Segments fetched before keeping one per video: a video with many matching
// segments must not crowd the others out.
const FETCH: i64 = 200;

/// Best matches first. English words match across forms (porter stemming:
/// "lease" finds "leasing").
pub fn speech_search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SpeechHit>> {
    let Some(q) = fts_query(query) else { return Ok(Vec::new()) };
    let mut stmt = conn.prepare(
        "SELECT file_path, t_sec, snippet(speech_fts, 2, '', '', '…', 12), bm25(speech_fts) FROM speech_fts
         WHERE speech_fts MATCH ?1 ORDER BY bm25(speech_fts) LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![q, FETCH], |row| {
            let snippet: String = row.get(2)?;
            let hit = SpeechHit {
                path: row.get(0)?,
                t_sec: row.get(1)?,
                snippet: snippet.split_whitespace().collect::<Vec<_>>().join(" "),
                score: 0,
            };
            Ok((hit, row.get::<_, f64>(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // bm25 is negative, more negative = better; rows come best first
    let best = rows.first().map_or(0.0, |r| r.1);
    let mut seen = HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|(hit, _)| seen.insert(hit.path.clone()))
        .take(limit)
        .map(|(mut hit, rank)| {
            hit.score = if best < 0.0 { ((rank / best * 100.0).round() as i32).clamp(1, 100) } else { 100 };
            hit
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(rows: &[(&str, f64, &str)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE speech_fts USING fts5(file_path UNINDEXED, t_sec UNINDEXED, body, tokenize = 'porter unicode61 remove_diacritics 2');",
        )
        .unwrap();
        for (path, t, body) in rows {
            conn.execute("INSERT INTO speech_fts VALUES (?1, ?2, ?3)", params![path, t, body]).unwrap();
        }
        conn
    }

    #[test]
    fn lease_finds_leasing_with_its_timecode() {
        let c = conn(&[("/v/a.mp4", 0.0, "hello there"), ("/v/a.mp4", 42.5, "we signed the leasing contract"), ("/v/b.mp4", 3.0, "nothing")]);
        let hits = speech_search(&c, "lease", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].path.as_str(), hits[0].t_sec, hits[0].score), ("/v/a.mp4", 42.5, 100));
        assert!(hits[0].snippet.contains("leasing") && !hits[0].snippet.contains('\n'));
    }

    #[test]
    fn one_hit_per_video_best_segment_first() {
        let long = "filler ".repeat(40);
        let c = conn(&[
            ("/v/a.mp4", 1.0, &format!("{long} lease {long}")),
            ("/v/a.mp4", 9.0, "lease lease lease"),
            ("/v/b.mp4", 2.0, &format!("{long} lease {long}")),
        ]);
        let hits = speech_search(&c, "lease", 10).unwrap();
        assert_eq!(hits.iter().map(|h| (h.path.as_str(), h.t_sec)).collect::<Vec<_>>(), [("/v/a.mp4", 9.0), ("/v/b.mp4", 2.0)]);
        assert!(hits[1].score < 100);
        assert_eq!(speech_search(&c, "lease", 1).unwrap().len(), 1);
    }

    #[test]
    fn requires_all_words_and_survives_fts_syntax() {
        let c = conn(&[("/v/a.mp4", 0.0, "the lease ends in may")]);
        assert_eq!(speech_search(&c, "lease may", 10).unwrap().len(), 1);
        assert!(speech_search(&c, "lease zebra", 10).unwrap().is_empty());
        for q in ["\"", "foo AND (", "NEAR(", "col:x", "*", ""] {
            assert!(speech_search(&c, q, 10).is_ok(), "{q:?}");
        }
    }
}
