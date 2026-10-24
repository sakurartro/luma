// Поиск по OCR-тексту картинок. Слова всех распознанных картинок лежат в
// памяти (как VECTORS у clip), матчинг — тот же fuzzy, что и по именам файлов.
// Оценка = какая доля слов картинки совпала с запросом.

use crate::backend::apps::fuzzy::is_match;
use crate::backend::files::files_search::FileEntry;
use rusqlite::Connection;
use std::sync::Mutex;

// match_kind in FileItem: the file was found by the text on it.
pub const MATCH_OCR: i32 = 2;

// Subsequence matching is loose: query words shorter than this match almost
// every word, so they are ignored.
const MIN_WORD_CHARS: usize = 3;
const MAX_HITS: usize = 32;

struct OcrDoc {
    name: String,
    path: String,
    mtime: i64,
    // lowercase words of the recognized text, repeats kept
    words: Vec<Vec<char>>,
}

// Snapshot of ocr_text joined with files; filled by the indexing worker
// (reload) at its start and end, empty while the feature is off.
static DOCS: Mutex<Vec<OcrDoc>> = Mutex::new(Vec::new());

fn words(text: &str) -> Vec<Vec<char>> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.chars().collect())
        .collect()
}

// Texts of files that vanished from the scan are excluded by the JOIN;
// empty texts (no text found on the image) have nothing to match.
fn load_docs(conn: &Connection) -> Vec<OcrDoc> {
    let rows = conn
        .prepare(
            "SELECT f.name, f.path, f.mtime, o.text FROM ocr_text o
             JOIN files f ON f.path = o.file_path WHERE o.text <> ''",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(OcrDoc {
                    name: row.get(0)?,
                    path: row.get(1)?,
                    mtime: row.get(2)?,
                    // stored lowercased (see index.rs)
                    words: words(&row.get::<_, String>(3)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        });
    rows.unwrap_or_else(|err| {
        eprintln!("ocr search cache error: {err}");
        Vec::new()
    })
}

pub(super) fn reload(conn: &Connection) {
    *DOCS.lock().unwrap() = load_docs(conn);
}

// Percent of the image's words that fuzzy-match any query word; at least 1
// for any match, so a hit never shows as 0%.
fn percent(doc_words: &[Vec<char>], query: &[Vec<char>]) -> i32 {
    let matched = doc_words
        .iter()
        .filter(|w| query.iter().any(|q| is_match(w, q)))
        .count();
    if matched == 0 {
        return 0;
    }
    ((matched * 100 + doc_words.len() / 2) / doc_words.len()).max(1) as i32
}

fn search(docs: &[OcrDoc], query: &str) -> Vec<(FileEntry, i32)> {
    let query: Vec<Vec<char>> = words(&query.to_lowercase())
        .into_iter()
        .filter(|w| w.len() >= MIN_WORD_CHARS)
        .collect();
    if query.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(i32, &OcrDoc)> = docs
        .iter()
        .map(|d| (percent(&d.words, &query), d))
        .filter(|(p, _)| *p > 0)
        .collect();
    // best match first, then newest, then by path for a stable order
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.mtime.cmp(&a.1.mtime))
            .then_with(|| a.1.path.cmp(&b.1.path))
    });
    scored
        .into_iter()
        .take(MAX_HITS)
        .map(|(p, d)| {
            (
                FileEntry {
                    name: d.name.clone(),
                    name_lower: d.name.to_lowercase().chars().collect(),
                    path: d.path.clone(),
                    is_dir: false,
                    mtime: d.mtime,
                },
                p,
            )
        })
        .collect()
}

/// Images whose recognized text fuzzy-matches the query, with the share of
/// the image's words that matched (0-100) for the UI badge. Best first.
/// Empty until the indexing worker has loaded the snapshot.
pub fn ocr_search(query: &str) -> Vec<(FileEntry, i32)> {
    search(&DOCS.lock().unwrap(), query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(path: &str, mtime: i64, text: &str) -> OcrDoc {
        OcrDoc { name: path.rsplit('/').next().unwrap().into(), path: path.into(), mtime, words: words(text) }
    }

    fn found(hits: &[(FileEntry, i32)]) -> Vec<(&str, i32)> {
        hits.iter().map(|(f, p)| (f.path.as_str(), *p)).collect()
    }

    #[test]
    fn percent_is_share_of_matched_words() {
        let docs = [doc("/d/a.png", 1, "invoice total paid today")];
        assert_eq!(found(&search(&docs, "invoice")), [("/d/a.png", 25)]);
        assert_eq!(found(&search(&docs, "invoice paid")), [("/d/a.png", 50)]); // two query words
        assert_eq!(found(&search(&docs, "ivc")), [("/d/a.png", 25)]); // fuzzy subsequence
        assert_eq!(found(&search(&docs, "INVOICE")), [("/d/a.png", 25)]);
    }

    #[test]
    fn best_share_first_then_newest() {
        let docs = [
            doc("/d/long.png", 9, "invoice one two three four five six seven"),
            doc("/d/short.png", 1, "invoice paid"),
            doc("/d/new.png", 5, "invoice paid"),
            doc("/d/none.png", 5, "nothing here"),
        ];
        assert_eq!(
            found(&search(&docs, "invoice")),
            [("/d/new.png", 50), ("/d/short.png", 50), ("/d/long.png", 13)]
        );
    }

    #[test]
    fn short_query_words_and_empty_docs() {
        let docs = [doc("/d/a.png", 1, "ok invoice")];
        assert!(search(&docs, "in").is_empty());
        assert!(search(&[], "invoice").is_empty());
        // short word is dropped, the long one still counts: 1 of 2 words
        assert_eq!(found(&search(&docs, "ok invoice")), [("/d/a.png", 50)]);
    }

    #[test]
    fn cyrillic_and_tiny_share_never_zero() {
        let docs = [doc("/d/ru.png", 1, "счёт на оплату")];
        assert_eq!(found(&search(&docs, "Оплат")), [("/d/ru.png", 33)]);
        let big = [doc("/d/big.png", 1, &format!("invoice {}", "word ".repeat(499)))];
        assert_eq!(found(&search(&big, "invoice")), [("/d/big.png", 1)]); // 0.2% -> 1
    }

    #[test]
    fn load_docs_joins_files_and_skips_empty() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, mtime INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE ocr_text (file_path TEXT PRIMARY KEY, mtime INTEGER NOT NULL DEFAULT 0, text TEXT NOT NULL);
            INSERT INTO files (name, path, mtime) VALUES ('a.png', '/d/a.png', 3), ('b.png', '/d/b.png', 3);
            INSERT INTO ocr_text VALUES ('/d/a.png', 3, 'hello, world: 100%');
            INSERT INTO ocr_text VALUES ('/d/b.png', 3, '');
            INSERT INTO ocr_text VALUES ('/gone/c.png', 3, 'ghost text');
            "#,
        )
        .unwrap();

        let docs = load_docs(&conn);

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].path, "/d/a.png");
        assert_eq!(docs[0].words.len(), 3); // hello, world, 100
    }
}
