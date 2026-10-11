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
        }
    }
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// Missing file → defaults + write a template to edit; broken file → defaults, file kept.
pub fn load() -> Settings {
    let path = settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        let s = Settings::default();
        if let Ok(json) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(&path, json);
        }
        return s;
    };
    serde_json::from_str(&text).unwrap_or_else(|err| {
        eprintln!("settings.json parse error, using defaults: {err}");
        Settings::default()
    })
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
