// Search for AI agents: `luma query clip|ocr "<query>"` sends the query over the
// toggle socket to the running instance, which answers with JSON. All search
// logic stays in the daemon; this is only the transport.

use crate::backend::files::files_search::FileEntry;
use crate::backend::files::{clip, ocr};
use crate::backend::settings;
use crate::toggle::socket_path;
use anyhow::{Result, anyhow};
use serde_json::json;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

/// What a request searches by; each kind has its own prefix on the socket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// What the image shows (CLIP).
    Clip,
    /// The text written on the image (OCR).
    Ocr,
}

impl Kind {
    // Anything on the socket without a prefix stays a toggle.
    fn prefix(self) -> &'static str {
        match self {
            Kind::Clip => "clip ",
            Kind::Ocr => "ocr ",
        }
    }
}

/// `clip river in fog` → (Clip, "river in fog"); None = not a search, a toggle.
pub fn parse_request(request: &str) -> Option<(Kind, &str)> {
    [Kind::Clip, Kind::Ocr]
        .into_iter()
        .find_map(|kind| Some((kind, request.strip_prefix(kind.prefix())?)))
}

/// Daemon side, on its own thread: inference must not hold up toggles.
pub fn handle_search(mut stream: UnixStream, kind: Kind, query: &str) {
    let _ = stream.write_all(search_response(kind, query, &settings::load()).as_bytes());
}

fn error(message: &str) -> String {
    json!({"error": message}).to_string()
}

// `[{"path", "score"}]` best first (score 0-100), or `{"error"}`.
fn search_response(kind: Kind, query: &str, settings: &settings::Settings) -> String {
    if !settings.agent_api {
        return error("agent API is disabled: set \"agent_api\": true in ~/.config/luma/settings.json (no restart needed)");
    }
    let hits = match kind {
        // clip_search can't tell "no hits" from "not loaded": both are empty.
        Kind::Clip if !clip::clip_ready() => {
            return error("semantic search is not available: enable \"semantic\" in settings.json or wait for indexing to finish");
        }
        Kind::Clip => clip::clip_search(query, clip::NEVER_STALE),
        // same for ocr_search before the first snapshot is loaded
        Kind::Ocr if !ocr::ocr_ready() => {
            return error("text search is not available: enable \"ocr\" in settings.json or wait for indexing to finish");
        }
        Kind::Ocr => ocr::ocr_search(query),
    };
    hits_json(hits, &settings.hide_patterns)
}

// UI hides these only in its own views, so the API applies them itself.
fn hits_json(hits: Vec<(FileEntry, i32)>, hide_patterns: &[String]) -> String {
    let hits: Vec<_> = hits
        .into_iter()
        .filter(|(f, _)| !hide_patterns.iter().any(|p| f.name.starts_with(p.as_str())))
        .map(|(f, score)| json!({"path": f.path, "score": score}))
        .collect();
    serde_json::Value::from(hits).to_string()
}

const USAGE: &str = "usage: luma query <clip|ocr> [--limit N] \"<query>\" (default limit 5)";
pub const DEFAULT_LIMIT: usize = 5;

// Args after `query clip|ocr`: `--limit N` anywhere, the rest is the query.
fn parse_args(args: &[String]) -> Result<(String, usize)> {
    let mut words = Vec::new();
    let mut limit = DEFAULT_LIMIT;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--limit" {
            let n = args.next().and_then(|n| n.parse::<usize>().ok()).filter(|&n| n > 0);
            limit = n.ok_or_else(|| anyhow!("--limit needs a positive number\n{USAGE}"))?;
        } else {
            words.push(arg.as_str());
        }
    }
    let query = words.join(" ");
    if query.trim().is_empty() {
        return Err(anyhow!(USAGE));
    }
    Ok((query, limit))
}

/// CLI side, args after `query`: prints the daemon's JSON answer; Err (exit 1)
/// when Luma is not running or answers with an error.
pub fn run_query(args: &[String]) -> Result<()> {
    let kind = match args.first().map(String::as_str) {
        Some("clip") => Kind::Clip,
        Some("ocr") => Kind::Ocr,
        Some(kind) => return Err(anyhow!("unknown search kind '{kind}' (available: clip, ocr)\n{USAGE}")),
        None => return Err(anyhow!(USAGE)),
    };
    let (query, limit) = parse_args(&args[1..])?;
    println!("{}", hits(kind, &query, limit)?);
    Ok(())
}

/// Client side, shared by the CLI and MCP: the daemon's hits, best first, cut
/// to `limit`; Err when Luma is not running or answers with an error.
pub fn hits(kind: Kind, query: &str, limit: usize) -> Result<serde_json::Value> {
    let mut stream =
        UnixStream::connect(socket_path()).map_err(|_| anyhow!("Luma is not running"))?;
    stream.write_all(format!("{}{query}", kind.prefix()).as_bytes())?;
    // EOF ends the request for the daemon.
    stream.shutdown(Shutdown::Write)?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    let mut value: serde_json::Value = serde_json::from_str(&answer)
        .map_err(|_| anyhow!("unexpected answer from Luma (an older version still running?)"))?;
    if let Some(err) = value.get("error").and_then(|e| e.as_str()) {
        return Err(anyhow!("{err}"));
    }
    // Cut on the client: the daemon sends at most 32 hits anyway.
    if let Some(hits) = value.as_array_mut() {
        hits.truncate(limit);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(kind: Kind, settings: &settings::Settings) -> serde_json::Value {
        serde_json::from_str(&search_response(kind, "cat", settings)).unwrap()
    }

    #[test]
    fn disabled_by_default() {
        for kind in [Kind::Clip, Kind::Ocr] {
            let value = answer(kind, &settings::Settings::default());
            assert!(value["error"].as_str().unwrap().starts_with("agent API is disabled"));
        }
    }

    #[test]
    fn not_ready_answers_with_error() {
        // MODELS is never loaded and no OCR snapshot is taken in tests.
        let on = settings::Settings { agent_api: true, ..Default::default() };
        let clip = answer(Kind::Clip, &on);
        assert!(clip["error"].as_str().unwrap().starts_with("semantic search is not available"));
        let ocr = answer(Kind::Ocr, &on);
        assert!(ocr["error"].as_str().unwrap().starts_with("text search is not available"));
    }

    #[test]
    fn requests_are_told_apart_by_prefix() {
        assert_eq!(parse_request("clip river in fog"), Some((Kind::Clip, "river in fog")));
        assert_eq!(parse_request("ocr connection refused"), Some((Kind::Ocr, "connection refused")));
        assert_eq!(parse_request("toggle"), None);
        // a word that merely starts like a prefix is a toggle
        assert_eq!(parse_request("clipboard"), None);
    }

    #[test]
    fn hidden_names_are_not_returned() {
        let file = |name: &str| FileEntry {
            name: name.into(),
            name_lower: name.to_lowercase().chars().collect(),
            path: format!("/p/{name}"),
            is_dir: false,
            mtime: 0,
        };
        let hits = vec![(file(".secret.png"), 90), (file("fog.png"), 70)];
        let out: serde_json::Value = serde_json::from_str(&hits_json(hits, &[".".to_string()])).unwrap();
        assert_eq!(out, json!([{"path": "/p/fog.png", "score": 70}]));
    }

    #[test]
    fn limit_is_taken_out_of_the_query() {
        let args = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(parse_args(&args("foggy river")).unwrap(), ("foggy river".into(), DEFAULT_LIMIT));
        assert_eq!(parse_args(&args("--limit 7 foggy river")).unwrap(), ("foggy river".into(), 7));
        assert_eq!(parse_args(&args("foggy --limit 3 river")).unwrap(), ("foggy river".into(), 3));
        assert!(parse_args(&args("foggy --limit")).is_err());
        assert!(parse_args(&args("foggy --limit 0")).is_err());
        assert!(parse_args(&args("foggy --limit x")).is_err());
        assert!(parse_args(&args("--limit 5")).is_err()); // no query
    }
}
