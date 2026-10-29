// MCP server for AI agents: `luma mcp`. The agent starts it as a subprocess and
// speaks newline-delimited JSON-RPC over stdin/stdout. Handwritten: one tool
// doesn't need an SDK. The searches themselves are api::hits, same as `luma query`.
//
// Serves both protocol eras: legacy clients open with `initialize` (2025-11-25
// and earlier), modern ones put the version into every request's `_meta`
// (2026-07-28) and may probe with `server/discover` first.

use crate::api::{self, Kind};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const MODERN: &str = "2026-07-28";
// Tool calls look the same in all of these; the first is the answer to unknown ones.
const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

const TOOL_IMAGES: &str = "luma_search_images";
const TOOL_TEXT: &str = "luma_search_text";

// Reaches the model with the server's name even when the tools themselves are
// deferred (Claude Code shows only their names until it loads the schemas), so
// it is the one place that makes the agent think of Luma first.
const INSTRUCTIONS: &str = "Luma has already indexed the user's photos and screenshots on this \
    computer. When the user wants to find a photo, picture or screenshot, call \
    luma_search_images (what it shows) or luma_search_text (words written on it) FIRST, \
    before find, grep, ls or running OCR yourself: it answers in under a second.";

/// `luma mcp`: answers until the agent closes stdin. Nothing but protocol
/// messages may go to stdout.
pub fn run() -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg),
            Err(_) => Some(error(Value::Null, -32700, "Parse error", None)),
        };
        // serde_json escapes newlines inside strings, so one message stays one line.
        if let Some(reply) = reply {
            writeln!(out, "{reply}")?;
        }
    }
    Ok(())
}

/// One incoming message → the reply; None for notifications (no `id`) and
/// stray responses (no `method`), which get no answer.
fn handle(msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?.clone();
    let params = &msg["params"];
    let version = params["_meta"][VERSION_KEY].as_str();
    if version.is_some_and(|v| v != MODERN) {
        let data = json!({"supported": [MODERN], "requested": version});
        return Some(error(id, -32022, "Unsupported protocol version", Some(data)));
    }
    let mut result = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let agreed = if LEGACY.contains(&asked) { asked } else { LEGACY[0] };
            json!({"protocolVersion": agreed, "capabilities": {"tools": {}}, "serverInfo": server_info(), "instructions": INSTRUCTIONS})
        }
        "server/discover" => json!({
            "supportedVersions": [MODERN],
            "capabilities": {"tools": {}},
            "_meta": {"io.modelcontextprotocol/serverInfo": server_info()},
            "instructions": INSTRUCTIONS,
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": [images_tool(), text_tool()]}),
        "tools/call" => {
            let kind = match params["name"].as_str() {
                Some(TOOL_IMAGES) => Kind::Clip,
                Some(TOOL_TEXT) => Kind::Ocr,
                _ => {
                    let msg = format!("Unknown tool: {}", params["name"]);
                    return Some(error(id, -32602, &msg, None));
                }
            };
            search(kind, &params["arguments"])
        }
        _ => return Some(error(id, -32601, &format!("Method not found: {method}"), None)),
    };
    if version.is_some() {
        result["resultType"] = json!("complete");
    }
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn server_info() -> Value {
    json!({"name": "luma", "version": env!("CARGO_PKG_VERSION")})
}

fn error(id: Value, code: i32, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

// The descriptions are all the agent knows about the tools: keep them in sync
// with agent/skills/luma/SKILL.md.
fn images_tool() -> Value {
    tool(
        TOOL_IMAGES,
        "Search photos by content",
        "ALWAYS call this FIRST whenever the user wants to find, locate or show a \
            photo, picture or image of anything (a person, animal, place, object, scene; any \
            language, e.g. \"photo of a bear\", \"где фотка тигра\"). File names rarely describe \
            what a photo shows, so find/ls/grep on names will miss it. This searches by what is \
            visible in the image (CLIP) in under a second. Translate the request to English and \
            describe what is visible (\"brown bear in a forest\"). For a screenshot or document \
            the user remembers by the words written on it, use luma_search_text instead. Returns \
            JSON [{\"path\", \"score\"}], best first; score 0-100 is relative within one answer, \
            and 50+ is often a real match. [] means nothing matched: rephrase before giving up. \
            Matches are similar-looking, not guaranteed. If the top hit scores 50+ and is clearly \
            ahead of the second, trust it: give its path without opening it. If scores are low or \
            close, or the user wants a specific person, place or detail, open the top 1-2 hits and \
            look at the image itself. Never reject a hit because of its file name, and never say \
            nothing was found before you have opened the top hits of a low-scoring answer. Only \
            folders in search_dirs of ~/.config/luma/settings.json are indexed. Needs \
            \"agent_api\": true there.",
        "What is in the picture, in English",
    )
}

fn text_tool() -> Value {
    tool(
        TOOL_TEXT,
        "Search images by the text on them",
        "Find a screenshot, photo of a document, receipt, sign or slide by the TEXT written \
            on it (OCR), in under a second. Use it when the user remembers words that appear in \
            the image: an error message, a name, a number, a title (\"the screenshot with \
            connection refused\"). To find an image by what it SHOWS use luma_search_images \
            instead; if one of the two finds nothing, try the other. The query is the words as \
            they are written on the image: no translation, no description. Only English text is \
            recognized, so text in other languages will not be found. Words shorter than 3 \
            letters are ignored; a few distinctive words work better than a sentence. Returns \
            JSON [{\"path\", \"score\"}], best first; score 0-100 is how much of the query was \
            found in the image's text, 100 = every word exactly. A score of 100 means the words \
            are really on that image: trust it. Open the hit only to read the text itself or \
            when scores are lower (the text is not returned). [] means no image has that text: \
            try other words from the same text, or luma_search_images. Only folders in \
            search_dirs of ~/.config/luma/settings.json are indexed. Needs \"agent_api\": true \
            and \"ocr\": true there.",
        "Words written on the image, as written (English)",
    )
}

fn tool(name: &str, title: &str, description: &str, query_help: &str) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": query_help},
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Max hits, default 5; Luma returns at most 32",
                },
            },
            "required": ["query"],
            "additionalProperties": false,
        },
        "annotations": {"readOnlyHint": true},
    })
}

// Bad arguments and search failures go back to the model as isError (not
// JSON-RPC errors), so it can retry or tell the user how to fix Luma.
fn search(kind: Kind, args: &Value) -> Value {
    let query = args["query"].as_str().unwrap_or("").trim();
    if query.is_empty() {
        return text_result("query must be a non-empty string", true);
    }
    let limit = match &args["limit"] {
        Value::Null => Some(api::DEFAULT_LIMIT),
        n => n.as_u64().filter(|&n| n > 0).map(|n| n as usize),
    };
    let Some(limit) = limit else {
        return text_result("limit must be a positive integer", true);
    };
    match api::hits(kind, query, limit) {
        Ok(hits) => text_result(&hits.to_string(), false),
        Err(e) => text_result(&e.to_string(), true),
    }
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: Value) -> Value {
        handle(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).unwrap()
    }

    fn modern(mut params: Value) -> Value {
        params["_meta"] = json!({VERSION_KEY: MODERN});
        params
    }

    #[test]
    fn initialize_agrees_on_a_legacy_version() {
        let asked = |v: &str| request("initialize", json!({"protocolVersion": v}))["result"]["protocolVersion"].clone();
        assert_eq!(asked("2025-06-18"), "2025-06-18");
        assert_eq!(asked("1999-01-01"), LEGACY[0]);
    }

    #[test]
    fn both_handshakes_carry_instructions_naming_the_tools() {
        let legacy = request("initialize", json!({"protocolVersion": LEGACY[0]}));
        let modern = request("server/discover", modern(json!({})));
        for result in [&legacy["result"], &modern["result"]] {
            let text = result["instructions"].as_str().unwrap();
            assert!(text.contains(TOOL_IMAGES) && text.contains(TOOL_TEXT));
        }
    }

    #[test]
    fn notifications_and_responses_get_no_answer() {
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        assert!(handle(&json!({"jsonrpc": "2.0", "id": 1, "result": {}})).is_none());
    }

    #[test]
    fn modern_requests_are_versioned() {
        let discover = request("server/discover", modern(json!({})));
        assert_eq!(discover["result"]["supportedVersions"], json!([MODERN]));
        assert_eq!(discover["result"]["resultType"], "complete");

        let list = request("tools/list", modern(json!({})));
        let names: Vec<_> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].clone()).collect();
        assert_eq!(names, [TOOL_IMAGES, TOOL_TEXT]);
        // Legacy answers stay as they were.
        assert!(request("tools/list", json!({}))["result"].get("resultType").is_none());

        let future = request("tools/list", json!({"_meta": {VERSION_KEY: "2099-01-01"}}));
        assert_eq!(future["error"]["code"], -32022);
        assert_eq!(future["error"]["data"]["supported"], json!([MODERN]));
    }

    #[test]
    fn bad_calls() {
        assert_eq!(request("nope", json!({}))["error"]["code"], -32601);
        assert_eq!(request("tools/call", json!({"name": "rm_rf"}))["error"]["code"], -32602);
        // Argument errors stop before the socket, so no daemon is needed.
        let call = |args: Value| request("tools/call", json!({"name": TOOL_IMAGES, "arguments": args}))["result"].clone();
        assert_eq!(call(json!({"query": "  "}))["isError"], true);
        assert_eq!(call(json!({"query": "cat", "limit": 0}))["isError"], true);
        assert_eq!(call(json!({"query": "cat", "limit": "5"}))["isError"], true);
        let text = request("tools/call", json!({"name": TOOL_TEXT, "arguments": {"query": ""}}));
        assert_eq!(text["result"]["isError"], true);
    }
}
