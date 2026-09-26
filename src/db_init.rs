use anyhow::Result;
use rusqlite::Connection;

pub struct DbState {
    pub conn: Connection,
}

impl DbState {

    pub fn init() -> Result<Self> {
        let conn = Connection::open("main.sqlite3")?;

        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS applications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                app_path TEXT NOT NULL UNIQUE,
                icon_path TEXT,
                command TEXT,
                launch_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS application_categories (
                application_id INTEGER NOT NULL
                    REFERENCES applications(id) ON DELETE CASCADE,
                category TEXT NOT NULL
                    CHECK (category = trim(category) AND category <> ''),
                PRIMARY KEY (application_id, category)
            );

            CREATE INDEX IF NOT EXISTS application_categories_by_category
            ON application_categories(category, application_id);
            "#,
        )?;

        Ok(Self { conn })
    }

}
