import aiosqlite

async def init_db():
    async with aiosqlite.connect("/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3") as conn:
        await conn.execute("""
            CREATE TABLE IF NOT EXISTS applications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            categories TEXT,
            app_path TEXT NOT NULL UNIQUE,
            icon_path TEXT,
            command TEXT,
            last_used TEXT DEFAULT CURRENT_TIMESTAMP,
            last_checked TEXT
        );
        """)
        await conn.commit()
