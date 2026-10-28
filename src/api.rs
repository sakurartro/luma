// Search for AI agents: `luma query clip "<query>"` sends the query over the
// toggle socket to the running instance, which answers with JSON. All search
// logic stays in the daemon; this is only the transport.

use crate::backend::files::clip;
use crate::backend::settings;
use crate::toggle::socket_path;
use anyhow::{Result, anyhow};
use serde_json::json;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

/// Request prefix on the socket; anything else stays a toggle.
pub const CLIP_PREFIX: &str = "clip ";

/// Daemon side, on its own thread: CLIP inference must not hold up toggles.
pub fn handle_clip(mut stream: UnixStream, query: &str) {
    let _ = stream.write_all(clip_response(query).as_bytes());
}

// `[{"path", "score"}]` best first (score 0-100), or `{"error"}`.
fn clip_response(query: &str) -> String {
    // clip_search can't tell "no hits" from "not loaded": both are empty.
    if !clip::clip_ready() {
        return json!({"error": "semantic search is not available: enable \"semantic\" in settings.json or wait for indexing to finish"})
            .to_string();
    }
    // UI hides these only in its own views, so the API applies them itself.
    let hide_patterns = settings::load().hide_patterns;
    let hits: Vec<_> = clip::clip_search(query, clip::NEVER_STALE)
        .into_iter()
        .filter(|(f, _)| !hide_patterns.iter().any(|p| f.name.starts_with(p.as_str())))
        .map(|(f, score)| json!({"path": f.path, "score": score}))
        .collect();
    serde_json::Value::from(hits).to_string()
}

const USAGE: &str = "usage: luma query clip [--limit N] \"<query>\" (default limit 5)";
pub const DEFAULT_LIMIT: usize = 5;

// Args after `query clip`: `--limit N` anywhere, the rest is the query.
fn parse_clip_args(args: &[String]) -> Result<(String, usize)> {
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
    match args.first().map(String::as_str) {
        Some("clip") => {}
        Some(kind) => return Err(anyhow!("unknown search kind '{kind}' (available: clip)\n{USAGE}")),
        None => return Err(anyhow!(USAGE)),
    }
    let (query, limit) = parse_clip_args(&args[1..])?;
    println!("{}", clip_hits(&query, limit)?);
    Ok(())
}

/// Client side, shared by the CLI and MCP: the daemon's hits, best first, cut
/// to `limit`; Err when Luma is not running or answers with an error.
pub fn clip_hits(query: &str, limit: usize) -> Result<serde_json::Value> {
    let mut stream =
        UnixStream::connect(socket_path()).map_err(|_| anyhow!("Luma is not running"))?;
    stream.write_all(format!("{CLIP_PREFIX}{query}").as_bytes())?;
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

    #[test]
    fn not_ready_answers_with_error() {
        // MODELS is never loaded in tests.
        let value: serde_json::Value = serde_json::from_str(&clip_response("cat")).unwrap();
        assert!(value.get("error").is_some());
    }

    #[test]
    fn limit_is_taken_out_of_the_query() {
        let args = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(parse_clip_args(&args("foggy river")).unwrap(), ("foggy river".into(), DEFAULT_LIMIT));
        assert_eq!(parse_clip_args(&args("--limit 7 foggy river")).unwrap(), ("foggy river".into(), 7));
        assert_eq!(parse_clip_args(&args("foggy --limit 3 river")).unwrap(), ("foggy river".into(), 3));
        assert!(parse_clip_args(&args("foggy --limit")).is_err());
        assert!(parse_clip_args(&args("foggy --limit 0")).is_err());
        assert!(parse_clip_args(&args("foggy --limit x")).is_err());
        assert!(parse_clip_args(&args("--limit 5")).is_err()); // no query
    }
}
