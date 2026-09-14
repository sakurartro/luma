import sqlite3
from datetime import datetime


def add_data(name: str, app_path: str, icon_path: str) -> None:
    now = datetime.now()
    now = now.strftime("%Y-%m-%d %H:%M:%S")

    with sqlite3.connect("/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3") as conn:
        cur = conn.cursor()
        cur.execute("""
            INSERT OR IGNORE INTO applications (name, app_path, icon_path, last_used) VALUES (?, ?, ?, ?)
        """, (name, app_path, icon_path, now))

def delete_data(filename: str) -> None:
    with sqlite3.connect("/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3") as conn:
        cur = conn.cursor()
        cur.execute("""
            DELETE FROM applications WHERE name = ?
        """, (filename,))
        conn.commit()

def update_datetime(name: str):
    now = datetime.now()
    now = now.strftime("%Y-%m-%d %H:%M:%S")

    with sqlite3.connect("/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3") as conn:
        cur = conn.cursor()
        conn.execute("""
            UPDATE applications SET last_used = ? WHERE name = ?
        """, (now, name))
        conn.commit()



def get_data():
    with sqlite3.connect("/run/media/sakura/1a177757-dd80-4b2f-8b81-d3db963ca160/projects/luma/src/database/main.sqlite3") as conn:
        cur = conn.cursor()
        cur.execute(
            "SELECT * FROM applications",
        )
        result = cur.fetchall()
        return result



if __name__ == "__main__":
    update_datetime("org.telegram")

