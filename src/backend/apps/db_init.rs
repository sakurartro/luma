use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

pub struct DbState {
    pub conn: Connection,
}

/// $XDG_CONFIG_HOME/luma/main.sqlite3 (defaults to ~/.config).
pub fn db_path() -> PathBuf {
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".config"),
    };
    let dir = dir.join("luma");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("main.sqlite3")
}

impl DbState {
    pub fn init() -> Result<Self> {
        let conn = Connection::open(db_path())?;

        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS applications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                app_path TEXT NOT NULL UNIQUE,
                icon_path TEXT,
                command TEXT,
                times_launched INTEGER NOT NULL DEFAULT 0,
                last_launched INTEGER
            );

            CREATE TABLE IF NOT EXISTS application_categories (
                application_id INTEGER NOT NULL
                    REFERENCES applications(id) ON DELETE CASCADE,
                category TEXT NOT NULL
                    CHECK (category = trim(category) AND category <> ''),
                times_launched INTEGER NOT NULL DEFAULT 0,
                last_launched INTEGER,
                PRIMARY KEY (application_id, category)
            );

            CREATE INDEX IF NOT EXISTS application_categories_by_category
            ON application_categories(category, application_id);
            "#,
        )?;

        Ok(Self { conn })
    }
}
