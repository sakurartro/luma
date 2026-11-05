// Plain text out of a document file. Any failure (corrupt, encrypted, scan,
// too big, unsupported) gives "": the caller then still records the mtime.

use crate::backend::settings::Settings;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::{BufReader, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

/// Per-file limits; the defaults live in `Settings`.
pub struct Limits {
    pub file_bytes: u64,
    pub text_bytes: usize,
    pub pdf_pages: u32,
}

impl From<&Settings> for Limits {
    fn from(s: &Settings) -> Self {
        Limits {
            file_bytes: s.docs_max_file_mb.saturating_mul(1024 * 1024),
            text_bytes: s.docs_max_text_kb.saturating_mul(1024),
            pdf_pages: s.docs_max_pdf_pages,
        }
    }
}

// Decompressed XML we are willing to scan (zip bomb guard).
const MAX_XML_BYTES: u64 = 64 * 1024 * 1024;
/// Never walked for documents, whatever `exclude_dirs` says.
pub const SKIP_DIRS: [&str; 4] = ["node_modules", "target", ".git", ".venv"];

const TEXT_EXTS: [&str; 6] = ["txt", "md", "rst", "org", "log", "csv"];
const ZIP_EXTS: [&str; 3] = ["docx", "pptx", "odt"];

/// SQL predicate for document files in `files f` (LIKE is ASCII case-insensitive).
pub fn doc_where() -> String {
    let likes: Vec<String> = ["pdf"]
        .iter()
        .chain(&TEXT_EXTS)
        .chain(&ZIP_EXTS)
        .map(|ext| format!("f.path LIKE '%.{ext}'"))
        .collect();
    format!("({})", likes.join(" OR "))
}

pub fn is_doc(path: &Path) -> bool {
    let ext = ext_of(path);
    ext == "pdf" || TEXT_EXTS.contains(&ext.as_str()) || ZIP_EXTS.contains(&ext.as_str())
}

fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn extract(path: &Path, lim: &Limits) -> String {
    if !is_doc(path) || std::fs::metadata(path).map_or(true, |m| m.len() > lim.file_bytes) {
        return String::new();
    }
    // pdf-extract and lopdf can panic on malformed files
    let text = catch_unwind(AssertUnwindSafe(|| match ext_of(path).as_str() {
        "pdf" => pdf(path, lim),
        "docx" | "pptx" | "odt" => zipped(path, lim.text_bytes),
        _ => plain(path, lim.text_bytes),
    }))
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or_default();
    cut(text, lim.text_bytes)
}

fn cut(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn plain(path: &Path, max: usize) -> anyhow::Result<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(max as u64).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn pdf(path: &Path, lim: &Limits) -> anyhow::Result<String> {
    let mut doc = pdf_extract::Document::load(path)?;
    // encrypted with a non-empty password: no text, not an error
    if doc.is_encrypted() && doc.decrypt("").is_err() {
        return Ok(String::new());
    }
    let mut out = String::new();
    for page in 1..=lim.pdf_pages {
        let mut page_text = String::new();
        let mut dev = pdf_extract::PlainTextOutput::new(&mut page_text);
        if pdf_extract::output_doc_page(&doc, &mut dev, page).is_err() {
            break; // past the last page (or a broken page: keep what we have)
        }
        out.push_str(&page_text);
        out.push('\n');
        if out.len() >= lim.text_bytes {
            break;
        }
    }
    Ok(out)
}

// docx: word/document.xml; pptx: ppt/slides/slideN.xml; odt: content.xml.
fn zipped(path: &Path, max: usize) -> anyhow::Result<String> {
    let mut zip = zip::ZipArchive::new(BufReader::new(std::fs::File::open(path)?))?;
    let mut names: Vec<String> = zip
        .file_names()
        .flatten()
        .filter(|n| {
            matches!(n.as_ref(), "word/document.xml" | "content.xml")
                || (n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        })
        .map(String::from)
        .collect();
    names.sort();
    let mut out = String::new();
    for name in names {
        let entry = zip.by_name(&name)?.take(MAX_XML_BYTES);
        xml_text(BufReader::new(entry), &mut out, max)?;
        if out.len() >= max {
            break;
        }
    }
    Ok(out)
}

// Text nodes joined as written (words are often split across runs); a space
// after paragraphs/headings and for ODF's <text:s/>, <text:tab/>, <text:line-break/>.
fn xml_text(src: impl std::io::BufRead, out: &mut String, max: usize) -> anyhow::Result<()> {
    let mut reader = Reader::from_reader(src);
    let mut buf = Vec::new();
    while out.len() < max {
        match reader.read_event_into(&mut buf)? {
            Event::Text(t) => out.push_str(&t.xml10_content()),
            Event::GeneralRef(r) => match r.resolve_char_ref()? {
                Some(c) => out.push(c),
                None => match &*r.xml10_content() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ => {}
                },
            },
            Event::End(e) if matches!(e.local_name().into_inner(), "p" | "h") => out.push(' '),
            Event::Empty(e) if matches!(e.local_name().into_inner(), "s" | "tab" | "line-break") => out.push(' '),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn lim() -> Limits {
        Limits::from(&Settings::default())
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("luma_docs_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn make_zip(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let path = tmp(name);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (n, body) in files {
            zip.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn text_formats_and_unsupported() {
        let p = tmp("a.md");
        std::fs::write(&p, "# Lease\nrent & more").unwrap();
        assert_eq!(extract(&p, &lim()), "# Lease\nrent & more");
        let p = tmp("a.bin");
        std::fs::write(&p, "lease").unwrap();
        assert_eq!(extract(&p, &lim()), "");
        assert_eq!(extract(&tmp("missing.txt"), &lim()), "");
    }

    #[test]
    fn text_is_capped_on_a_char_boundary() {
        let p = tmp("big.txt");
        std::fs::write(&p, "é".repeat(lim().text_bytes)).unwrap();
        let t = extract(&p, &lim());
        assert!(t.len() <= lim().text_bytes && t.starts_with('é'));
    }

    #[test]
    fn configured_limits_apply() {
        let p = tmp("limits.txt");
        std::fs::write(&p, "abcdefghij").unwrap();
        let mut l = lim();
        l.text_bytes = 4;
        assert_eq!(extract(&p, &l), "abcd");
        l.file_bytes = 5;
        assert_eq!(extract(&p, &l), "");
        let p = tmp("limits.pdf");
        std::fs::write(&p, pdf_bytes("Hello lease")).unwrap();
        let mut l = lim();
        l.pdf_pages = 0;
        assert_eq!(extract(&p, &l), "");
        // settings.json values are converted from MB / KB
        let s = Settings { docs_max_file_mb: 2, docs_max_text_kb: 3, ..Settings::default() };
        let l = Limits::from(&s);
        assert_eq!((l.file_bytes, l.text_bytes), (2 << 20, 3 << 10));
    }

    #[test]
    fn docx_words_split_across_runs_stay_whole() {
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>Lea</w:t></w:r><w:r><w:t>se &amp; rent</w:t></w:r></w:p><w:p><w:r><w:t>Tenant</w:t></w:r></w:p></w:body></w:document>"#;
        let p = make_zip("a.docx", &[("word/document.xml", xml)]);
        assert_eq!(extract(&p, &lim()), "Lease & rent Tenant ");
    }

    #[test]
    fn pptx_reads_every_slide() {
        let s = |t: &str| format!(r#"<p:sld xmlns:p="x" xmlns:a="y"><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:sld>"#);
        let p = make_zip("a.pptx", &[("ppt/slides/slide1.xml", &s("alpha")), ("ppt/slides/slide2.xml", &s("beta")), ("ppt/media/x.xml", "<a>no</a>")]);
        assert_eq!(extract(&p, &lim()), "alpha beta ");
    }

    #[test]
    fn odt_spaces_and_headings() {
        let xml = r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><text:h>Title</text:h><text:p>one<text:s/>two</text:p></office:body></office:document-content>"#;
        let p = make_zip("a.odt", &[("content.xml", xml)]);
        assert_eq!(extract(&p, &lim()), "Title one two ");
    }

    #[test]
    fn corrupt_files_give_empty_text() {
        for name in ["bad.docx", "bad.pdf"] {
            let p = tmp(name);
            std::fs::write(&p, "not a real file").unwrap();
            assert_eq!(extract(&p, &lim()), "", "{name}");
        }
    }

    // Smallest valid one-page PDF with Helvetica text, xref offsets computed.
    fn pdf_bytes(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 20 100 Td ({text}) Tj ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for off in offsets {
            out.extend(format!("{off:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    #[test]
    fn pdf_text_is_extracted() {
        let p = tmp("a.pdf");
        std::fs::write(&p, pdf_bytes("Hello lease")).unwrap();
        assert!(extract(&p, &lim()).contains("Hello lease"), "{:?}", extract(&p, &lim()));
    }
}
