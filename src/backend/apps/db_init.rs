use anyhow::Result;
use rusqlite::Connection;

use std::path::PathBuf;

pub struct DbState {
    pub conn: Connection,
}

/// $XDG_CONFIG_HOME/luma (defaults to ~/.config).
pub fn config_dir() -> PathBuf {
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME").expect("HOME not set")).join(".config"),
    };
    let dir = dir.join("luma");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// $XDG_CONFIG_HOME/luma/main.sqlite3 (defaults to ~/.config).
pub fn db_path() -> PathBuf {
    config_dir().join("main.sqlite3")
}

// All connections to the db go through this: WAL (set persistently in
// DbState::init) stops readers/writers blocking each other during the file
// scan; busy_timeout + synchronous=NORMAL keep UI-thread queries responsive.
pub fn open_conn() -> Result<Connection> {
    let conn = Connection::open(db_path())?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(conn)
}

impl DbState {
    pub fn init() -> Result<Self> {
        let conn = Connection::open(db_path())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;

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
                last_launched INTEGER,
                pinned INTEGER NOT NULL DEFAULT 0
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

            CREATE TABLE IF NOT EXISTS icons (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE
            );

            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE
            );
            "#,
        )?;

        Ok(Self { conn })
    }
}
