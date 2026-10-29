use serde::Deserialize;
use slint::Color;
use std::path::PathBuf;

use crate::backend::apps::db_init::config_dir;

#[derive(serde::Serialize, Deserialize)]
pub struct Settings {
    pub bg: String,
    pub surface: String,
    pub hover: String,
    pub selected: String,
    pub border: String,
    pub accent: String,
    pub text: String,
    pub text_dim: String,
    #[serde(default = "default_search_dirs")]
    pub search_dirs: Vec<String>,
    // Prefix match on file/folder name; "." hides all dotfiles
    #[serde(default = "default_hide_patterns")]
    pub hide_patterns: Vec<String>,
    // Semantic (CLIP) photo search; off by default. When enabled, the models
    // are downloaded from HuggingFace on the next start (~154 MB).
    #[serde(default)]
    pub semantic: bool,
    // OCR text search in images; off by default. When enabled, the models
    // are downloaded from HuggingFace on the next start (~12 MB).
    #[serde(default)]
    pub ocr: bool,
    // Local API for AI agents (`luma query`, MCP); off by default.
    #[serde(default)]
    pub agent_api: bool,
    // Currency conversion ("100 usd to eur") via open.er-api.com; with false
    // no network request is made, the calculator keeps working.
    #[serde(default = "default_currency")]
    pub currency: bool,
}

fn default_currency() -> bool {
    true
}

fn default_hide_patterns() -> Vec<String> {
    vec![".".into()]
}

fn default_search_dirs() -> Vec<String> {
    vec![
        "~/Documents".into(),
        "~/Pictures".into(),
        "~/Desktop".into(),
    ]
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bg: "#0E120B".into(),
            surface: "#151B10".into(),
            hover: "#1E2716".into(),
            selected: "#28331C".into(),
            border: "#2C3620".into(),
            accent: "#A9B665".into(),
            text: "#E4E7D0".into(),
            text_dim: "#7E8A6C".into(),
            search_dirs: default_search_dirs(),
            hide_patterns: default_hide_patterns(),
            semantic: false,
            ocr: false,
            agent_api: false,
            currency: default_currency(),
        }
    }
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// Missing file → defaults + write a template to edit; broken file → defaults, file kept;
/// fields added since the file was written → appended with their defaults.
pub fn load() -> Settings {
    let path = settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        let s = Settings::default();
        if let Ok(json) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(&path, json);
        }
        return s;
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(s) => {
            if let Some(json) = with_missing_fields(&text, &s) {
                let _ = std::fs::write(&path, json);
            }
            s
        }
        Err(err) => {
            eprintln!("settings.json parse error, using defaults: {err}");
            Settings::default()
        }
    }
}

// New options (like `ocr`) must show up in an existing file to be found:
// the user's JSON plus the fields it lacks; None when nothing is missing.
// User values and unknown keys are kept.
fn with_missing_fields(text: &str, s: &Settings) -> Option<String> {
    let serde_json::Value::Object(mut user) = serde_json::from_str(text).ok()? else { return None };
    let serde_json::Value::Object(full) = serde_json::to_value(s).ok()? else { return None };
    let before = user.len();
    for (key, value) in full {
        user.entry(key).or_insert(value);
    }
    (user.len() != before).then(|| serde_json::to_string_pretty(&user).ok()).flatten()
}

/// The file as a JSON object, keys in file order (settings menu).
pub fn read_object() -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let text = std::fs::read_to_string(settings_path())?;
    Ok(serde_json::from_str(&text)?)
}

/// One edit from the settings menu.
pub enum Change {
    Toggle,
    Set(bool),
    Text(String),
    ListSet(usize, String),
    ListAdd(String),
    ListRemove(usize),
}

/// Applies the edit to the file and returns the new settings; the file is
/// left untouched when it doesn't parse or the result isn't valid Settings.
pub fn update(key: &str, change: Change) -> anyhow::Result<Settings> {
    // canonical path: the rename below must replace a symlink's target, not the link
    let path = std::fs::canonicalize(settings_path())?;
    let (json, s) = with_change(&std::fs::read_to_string(&path)?, key, change)?;
    // tmp + rename: a crash mid-write never leaves a truncated settings.json
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(s)
}

// Edits one key of the user's JSON; unknown keys and key order are kept.
fn with_change(text: &str, key: &str, change: Change) -> anyhow::Result<(String, Settings)> {
    use serde_json::Value;
    let mut obj: serde_json::Map<String, Value> = serde_json::from_str(text)?;
    let value = obj.get_mut(key).ok_or_else(|| anyhow::anyhow!("{key}: no such setting"))?;
    // "" never enters a list: a "" hide pattern would hide every file (starts_with)
    match (value, change) {
        (Value::Bool(b), Change::Toggle) => *b = !*b,
        (Value::Bool(b), Change::Set(on)) => *b = on,
        (Value::String(s), Change::Text(new)) => {
            if is_color(s) && !is_color(&new) {
                anyhow::bail!("{key}: expected #RRGGBB, got {new:?}");
            }
            *s = new;
        }
        (Value::Array(items), Change::ListSet(i, new)) if i < items.len() => {
            if new.is_empty() {
                items.remove(i);
            } else {
                items[i] = Value::String(new);
            }
        }
        (Value::Array(items), Change::ListAdd(new)) => {
            if !new.is_empty() {
                items.push(Value::String(new));
            }
        }
        (Value::Array(items), Change::ListRemove(i)) if i < items.len() => {
            items.remove(i);
        }
        _ => anyhow::bail!("{key}: unsupported change"),
    }
    let obj = Value::Object(obj);
    let s = serde_json::from_value::<Settings>(obj.clone())?;
    Ok((serde_json::to_string_pretty(&obj)?, s))
}

/// Exactly "#RRGGBB"; the menu shows a color swatch for these.
pub fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// "#RRGGBB" (leading '#' optional); anything else → black.
pub fn parse_color(s: &str) -> Color {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            return Color::from_argb_encoded(0xFF00_0000 | v);
        }
    }
    Color::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_includes_semantic() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""semantic":false"#));
    }

    #[test]
    fn missing_fields_are_added_and_user_values_kept() {
        let text = r##"{"bg": "#111111", "surface": "#000000", "hover": "#000000", "selected": "#000000",
            "border": "#000000", "accent": "#000000", "text": "#000000", "text_dim": "#000000",
            "semantic": true, "custom": 1}"##;
        let s: Settings = serde_json::from_str(text).unwrap();
        let json: serde_json::Value = serde_json::from_str(&with_missing_fields(text, &s).unwrap()).unwrap();
        assert_eq!(json["ocr"], false);
        assert_eq!(json["currency"], true);
        assert_eq!(json["semantic"], true);
        assert_eq!(json["bg"], "#111111");
        assert_eq!(json["custom"], 1);
        // complete file -> no rewrite
        let full = serde_json::to_string(&s).unwrap();
        assert!(with_missing_fields(&full, &s).is_none());
    }

    fn template() -> String {
        serde_json::to_string_pretty(&Settings::default()).unwrap()
    }

    fn change(text: &str, key: &str, c: Change) -> serde_json::Value {
        serde_json::from_str(&with_change(text, key, c).unwrap().0).unwrap()
    }

    #[test]
    fn change_toggles_bool() {
        let json = change(&template(), "semantic", Change::Toggle);
        assert_eq!(json["semantic"], true);
    }

    #[test]
    fn change_set_is_idempotent() {
        let on = change(&template(), "agent_api", Change::Set(true));
        assert_eq!(on["agent_api"], true);
        let twice = change(&serde_json::to_string(&on).unwrap(), "agent_api", Change::Set(true));
        assert_eq!(twice["agent_api"], true);
    }

    #[test]
    fn change_with_wrong_type_is_rejected() {
        assert!(with_change(&template(), "semantic", Change::Text("yes".into())).is_err());
        assert!(with_change(&template(), "missing", Change::Toggle).is_err());
        // a field that fails Settings validation blocks the write
        let broken = template().replace(r#""currency": true"#, r#""currency": "yes""#);
        assert!(with_change(&broken, "ocr", Change::Toggle).is_err());
    }

    #[test]
    fn change_keeps_unknown_keys_and_order() {
        let text = template().replacen('{', r#"{"zz_custom": [1, 2],"#, 1);
        let out = with_change(&text, "accent", Change::Text("#123456".into())).unwrap().0;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["zz_custom"], serde_json::json!([1, 2]));
        assert_eq!(json["accent"], "#123456");
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(keys[0], "zz_custom");
        assert_eq!(keys[1], "bg");
    }

    #[test]
    fn change_edits_lists() {
        let t = template();
        let json = change(&t, "hide_patterns", Change::ListAdd("node_modules".into()));
        assert_eq!(json["hide_patterns"], serde_json::json!([".", "node_modules"]));
        let json = change(&t, "hide_patterns", Change::ListSet(0, "_".into()));
        assert_eq!(json["hide_patterns"], serde_json::json!(["_"]));
        let json = change(&t, "search_dirs", Change::ListRemove(1));
        assert_eq!(json["search_dirs"], serde_json::json!(["~/Documents", "~/Desktop"]));
        assert!(with_change(&t, "search_dirs", Change::ListRemove(9)).is_err());
    }

    #[test]
    fn empty_list_item_is_removed_not_stored() {
        let t = template();
        let json = change(&t, "hide_patterns", Change::ListSet(0, "".into()));
        assert_eq!(json["hide_patterns"], serde_json::json!([]));
        let json = change(&t, "hide_patterns", Change::ListAdd("".into()));
        assert_eq!(json["hide_patterns"], serde_json::json!(["."]));
    }

    #[test]
    fn bad_color_is_rejected() {
        let t = template();
        for bad in ["#zzz", "#12345", "123456", "#1234567", ""] {
            assert!(with_change(&t, "accent", Change::Text(bad.into())).is_err(), "{bad}");
        }
        assert_eq!(change(&t, "accent", Change::Text("#a1B2c3".into()))["accent"], "#a1B2c3");
    }

    #[test]
    fn template_includes_ocr() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""ocr":false"#));
    }
}
