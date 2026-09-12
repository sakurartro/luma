import sqlite3

def init_db():
    with sqlite3.connect("/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3") as conn:
        cur = conn.cursor()
        cur.execute("""
            CREATE TABLE IF NOT EXISTS applications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            app_path TEXT NOT NULL UNIQUE,
            icon_path TEXT,
            last_used TEXT DEFAULT CURRENT_TIMESTAMP
        );
        """)
        conn.commit()


init_db()
