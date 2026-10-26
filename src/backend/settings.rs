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

    #[test]
    fn template_includes_ocr() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""ocr":false"#));
    }
}
