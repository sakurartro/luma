// Search for AI agents: `luma query clip|ocr|video "<query>"` sends the query over the
// toggle socket to the running instance, which answers with JSON. All search
// logic stays in the daemon; this is only the transport.

use crate::backend::apps::db_init::open_conn;
use crate::backend::files::files_search::FileEntry;
use crate::backend::files::video::frames::ffmpeg_available;
use crate::backend::files::video::speech_index::speech_enabled;
use crate::backend::files::video::speech_search::{speech_search, SpeechHit};
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
    /// Videos: what a frame shows, text on a frame, what is said.
    Video,
}

impl Kind {
    // Anything on the socket without a prefix stays a toggle.
    fn prefix(self) -> &'static str {
        match self {
            Kind::Clip => "clip ",
            Kind::Ocr => "ocr ",
            Kind::Video => "video ",
        }
    }
}

/// `clip river in fog` → (Clip, "river in fog"); None = not a search, a toggle.
pub fn parse_request(request: &str) -> Option<(Kind, &str)> {
    [Kind::Clip, Kind::Ocr, Kind::Video]
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
        // photos only: videos get their own tool
        Kind::Clip => clip::clip_search(query, clip::NEVER_STALE)
            .into_iter()
            .filter_map(|(f, score, t_sec)| t_sec.is_none().then_some((f, score)))
            .collect(),
        // same for ocr_search before the first snapshot is loaded
        Kind::Ocr if !ocr::ocr_ready() => {
            return error("text search is not available: enable \"ocr\" in settings.json or wait for indexing to finish");
        }
        // photos only: videos get their own tool
        Kind::Ocr => ocr::ocr_search(query)
            .into_iter()
            .filter_map(|(f, score, t_sec)| t_sec.is_none().then_some((f, score)))
            .collect(),
        Kind::Video => return video_response(query, settings),
    };
    hits_json(hits, &settings.hide_patterns)
}

// Hits per answer, whatever the limit.
const MAX_HITS: usize = 32;

// Every video source that is ready answers; one that is not is skipped.
fn video_response(query: &str, settings: &settings::Settings) -> String {
    if !settings.video {
        return error("video search is disabled: set \"video\": true in ~/.config/luma/settings.json and restart Luma");
    }
    if !ffmpeg_available() {
        return error("video search is not available: ffmpeg/ffprobe not found in PATH, install ffmpeg and restart Luma");
    }
    let (frames_on, ocr_on, speech_on) = (clip::clip_ready(), ocr::ocr_ready(), speech_enabled());
    if !(frames_on || ocr_on || speech_on) {
        return error("video search is not available: enable \"semantic\", \"ocr\" or \"video_speech\" in settings.json or wait for indexing to finish");
    }
    let frames = if frames_on { clip::clip_search(query, clip::NEVER_STALE) } else { Vec::new() };
    let texts = if ocr_on { ocr::ocr_search(query) } else { Vec::new() };
    let speech = if speech_on {
        open_conn().and_then(|conn| speech_search(&conn, query, MAX_HITS)).unwrap_or_else(|err| {
            eprintln!("speech search error: {err}");
            Vec::new()
        })
    } else {
        Vec::new()
    };
    video_hits_json(frames, texts, speech, &settings.hide_patterns)
}

type FrameHits = Vec<(FileEntry, i32, Option<f64>)>;

// `[{"path", "source", "t_sec", "score", "snippet"?}]`, best score first: a
// frame by meaning ("frame"), text on a frame ("ocr") or words said ("speech",
// with the sentence around the match). One hit per video and source, the
// photos in the same indexes are left out. Scores are relative within one
// source, so ties keep frame, ocr, speech order.
fn video_hits_json(frames: FrameHits, texts: FrameHits, speech: Vec<SpeechHit>, hide_patterns: &[String]) -> String {
    let hidden = |name: &str| hide_patterns.iter().any(|p| name.starts_with(p.as_str()));
    let at = |t: f64| (t * 10.0).round() / 10.0;
    let mut hits: Vec<(i32, serde_json::Value)> = Vec::new();
    for (source, list) in [("frame", frames), ("ocr", texts)] {
        let mut seen = std::collections::HashSet::new();
        for (file, score, t_sec) in list {
            let Some(t) = t_sec else { continue };
            if !hidden(&file.name) && seen.insert(file.path.clone()) {
                hits.push((score, json!({"path": file.path, "source": source, "t_sec": at(t), "score": score})));
            }
        }
    }
    for hit in speech {
        let name = std::path::Path::new(&hit.path).file_name().unwrap_or_default().to_string_lossy();
        if !hidden(&name) {
            let value = json!({"path": hit.path, "source": "speech", "t_sec": at(hit.t_sec), "score": hit.score, "snippet": hit.snippet});
            hits.push((hit.score, value));
        }
    }
    hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    hits.truncate(MAX_HITS);
    serde_json::Value::from(hits.into_iter().map(|(_, hit)| hit).collect::<Vec<_>>()).to_string()
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

const USAGE: &str = "usage: luma query <clip|ocr|video> [--limit N] \"<query>\" (default limit 5)";
pub const DEFAULT_LIMIT: usize = 5;

// Args after `query clip|ocr|video`: `--limit N` anywhere, the rest is the query.
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
        Some("video") => Kind::Video,
        Some(kind) => return Err(anyhow!("unknown search kind '{kind}' (available: clip, ocr, video)\n{USAGE}")),
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
        for kind in [Kind::Clip, Kind::Ocr, Kind::Video] {
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
    fn video_needs_the_setting_and_a_ready_source() {
        let on = settings::Settings { agent_api: true, ..Default::default() };
        let off = answer(Kind::Video, &on);
        assert!(off["error"].as_str().unwrap().starts_with("video search is disabled"));
        // video on, but no index is loaded in tests
        let video = answer(Kind::Video, &settings::Settings { video: true, ..on });
        assert!(video["error"].as_str().unwrap().starts_with("video search is not available"));
    }

    #[test]
    fn requests_are_told_apart_by_prefix() {
        assert_eq!(parse_request("clip river in fog"), Some((Kind::Clip, "river in fog")));
        assert_eq!(parse_request("ocr connection refused"), Some((Kind::Ocr, "connection refused")));
        assert_eq!(parse_request("video the lease ends"), Some((Kind::Video, "the lease ends")));
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

    fn entry(name: &str) -> FileEntry {
        FileEntry { name: name.into(), name_lower: Vec::new(), path: format!("/v/{name}"), is_dir: false, mtime: 0 }
    }

    fn video_hits(frames: FrameHits, texts: FrameHits, speech: Vec<SpeechHit>) -> serde_json::Value {
        serde_json::from_str(&video_hits_json(frames, texts, speech, &[".".to_string()])).unwrap()
    }

    #[test]
    fn frame_hits_carry_their_timecode_and_skip_photos() {
        let frames = vec![(entry("a.mp4"), 80, Some(42.46)), (entry("photo.jpg"), 90, None), (entry("a.mp4"), 60, Some(5.0))];
        let out = video_hits(frames, Vec::new(), Vec::new());
        assert_eq!(out, json!([{"path": "/v/a.mp4", "source": "frame", "t_sec": 42.5, "score": 80}]));
    }

    #[test]
    fn ocr_hits_carry_their_timecode_and_skip_photos() {
        let texts = vec![(entry("shot.png"), 100, None), (entry("b.mkv"), 100, Some(7.0))];
        let out = video_hits(Vec::new(), texts, Vec::new());
        assert_eq!(out, json!([{"path": "/v/b.mkv", "source": "ocr", "t_sec": 7.0, "score": 100}]));
    }

    #[test]
    fn speech_hits_carry_timecode_and_snippet() {
        let speech = vec![SpeechHit { path: "/v/c.webm".into(), t_sec: 12.5, snippet: "the lease ends".into(), score: 100 }];
        let out = video_hits(Vec::new(), Vec::new(), speech);
        assert_eq!(out, json!([{"path": "/v/c.webm", "source": "speech", "t_sec": 12.5, "score": 100, "snippet": "the lease ends"}]));
    }

    #[test]
    fn sources_merge_by_score_and_hidden_names_drop() {
        let frames = vec![(entry("a.mp4"), 70, Some(1.0)), (entry(".hidden.mp4"), 99, Some(1.0))];
        let texts = vec![(entry("a.mp4"), 90, Some(2.0))];
        let speech = vec![
            SpeechHit { path: "/v/a.mp4".into(), t_sec: 3.0, snippet: "x".into(), score: 70 },
            SpeechHit { path: "/v/.secret.mp4".into(), t_sec: 3.0, snippet: "x".into(), score: 100 },
        ];
        let out = video_hits(frames, texts, speech);
        let order: Vec<_> = out.as_array().unwrap().iter().map(|h| h["source"].as_str().unwrap()).collect();
        assert_eq!(order, ["ocr", "frame", "speech"]);
    }
}
