import asyncio
import os
from pathlib import Path

import aiosqlite


def get_db_path() -> Path:
    value = os.environ.get("XDG_DATA_HOME")
    base = Path(value) if value and Path(value).is_absolute() else Path.home() / ".local/share"
    return base / "luma" / "main.sqlite3"


async def init_db() -> None:
    db_path = get_db_path()
    db_path.parent.mkdir(parents=True, exist_ok=True)

    async with aiosqlite.connect(db_path) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await conn.execute("""
            CREATE TABLE IF NOT EXISTS applications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                app_path TEXT NOT NULL UNIQUE,
                icon_path TEXT,
                command TEXT,
                last_used TEXT DEFAULT CURRENT_TIMESTAMP,
                last_checked TEXT
            )
        """)
        await conn.execute("""
            CREATE TABLE IF NOT EXISTS application_categories (
                application_id INTEGER NOT NULL
                    REFERENCES applications(id) ON DELETE CASCADE,
                category TEXT NOT NULL
                    CHECK (category = trim(category) AND category <> ''),
                PRIMARY KEY (application_id, category)
            )
        """)
        await conn.execute("""
            CREATE INDEX IF NOT EXISTS application_categories_by_category
            ON application_categories(category, application_id)
        """)
        await conn.commit()


if __name__ == "__main__":
    asyncio.run(init_db())
