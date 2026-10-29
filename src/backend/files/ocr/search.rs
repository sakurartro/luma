// Поиск по OCR-тексту картинок. Слова всех распознанных картинок лежат в
// памяти (как VECTORS у clip), матчинг — тот же fuzzy, что и по именам файлов.
// Оценка = насколько хорошо совпал запрос: для каждого слова запроса берётся
// лучшее слово картинки (100 за точное, иначе доля этого слова, покрытая
// запросом), и это усредняется по словам запроса.

use crate::backend::apps::fuzzy::is_match;
use crate::backend::files::files_search::FileEntry;
use rusqlite::Connection;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

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

// The snapshot was loaded at least once. Before that an empty answer is not
// "no match" but "not indexed yet / feature off" (the agent API tells them apart).
static LOADED: AtomicBool = AtomicBool::new(false);

pub fn ocr_ready() -> bool {
    LOADED.load(Ordering::Acquire)
}

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
    LOADED.store(true, Ordering::Release);
}

// How well the query matched, averaged over query words: each takes its best
// image word, 100 if equal, else the share of that word the query covers
// ("invoic" vs "invoice" = 85), 0 if none matches. At least 1 for any match,
// so a hit never shows as 0%.
fn percent(doc_words: &[Vec<char>], query: &[Vec<char>]) -> i32 {
    let total: usize = query
        .iter()
        .map(|q| {
            doc_words
                .iter()
                .map(|w| {
                    if w == q {
                        100
                    } else if is_match(w, q) {
                        (q.len() * 100 / w.len()).max(1)
                    } else {
                        0
                    }
                })
                .max()
                .unwrap_or(0)
        })
        .sum();
    if total == 0 {
        return 0;
    }
    ((total + query.len() / 2) / query.len()).max(1) as i32
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

/// Images whose recognized text fuzzy-matches the query, with how well the
/// query matched (0-100, see `percent`) for the UI badge. Best first.
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
    fn exact_word_is_100_however_busy_the_image() {
        let docs = [doc("/d/a.png", 1, "invoice total paid today")];
        assert_eq!(found(&search(&docs, "invoice")), [("/d/a.png", 100)]);
        assert_eq!(found(&search(&docs, "INVOICE")), [("/d/a.png", 100)]);
        let big = [doc("/d/big.png", 1, &format!("{} invoice", "word ".repeat(499)))];
        assert_eq!(found(&search(&big, "invoice")), [("/d/big.png", 100)]);
    }

    #[test]
    fn partial_match_is_share_of_the_word_covered() {
        let docs = [doc("/d/a.png", 1, "invoice total")];
        assert_eq!(found(&search(&docs, "invoic")), [("/d/a.png", 85)]); // prefix, 6/7
        assert_eq!(found(&search(&docs, "ivc")), [("/d/a.png", 42)]); // fuzzy subsequence, 3/7
    }

    #[test]
    fn missing_query_word_counts_zero() {
        let docs = [doc("/d/a.png", 1, "invoice total paid today")];
        assert_eq!(found(&search(&docs, "invoice paid")), [("/d/a.png", 100)]);
        assert_eq!(found(&search(&docs, "invoice zebra")), [("/d/a.png", 50)]);
        assert!(search(&docs, "zebra").is_empty());
    }

    #[test]
    fn best_match_first_then_newest() {
        let docs = [
            doc("/d/partial.png", 9, "invoices"),
            doc("/d/old.png", 1, "invoice paid"),
            doc("/d/new.png", 5, "lots of words around the invoice"),
            doc("/d/none.png", 5, "nothing here"),
        ];
        assert_eq!(
            found(&search(&docs, "invoice")),
            [("/d/new.png", 100), ("/d/old.png", 100), ("/d/partial.png", 87)]
        );
    }

    #[test]
    fn short_query_words_and_empty_docs() {
        let docs = [doc("/d/a.png", 1, "ok invoice")];
        assert!(search(&docs, "in").is_empty());
        assert!(search(&[], "invoice").is_empty());
        // short word is dropped, not counted as missing
        assert_eq!(found(&search(&docs, "ok invoice")), [("/d/a.png", 100)]);
    }

    #[test]
    fn cyrillic_and_tiny_match_never_zero() {
        let docs = [doc("/d/ru.png", 1, "счёт на оплату")];
        assert_eq!(found(&search(&docs, "Оплату")), [("/d/ru.png", 100)]);
        assert_eq!(found(&search(&docs, "Оплат")), [("/d/ru.png", 83)]);
        // 3 of 403 chars covered, one of three query words: rounds to 0 -> 1
        let long = [doc("/d/long.png", 1, &format!("a{}bc", "x".repeat(400)))];
        assert_eq!(found(&search(&long, "abc zzz qqq")), [("/d/long.png", 1)]);
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
