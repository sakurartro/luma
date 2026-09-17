import aiosqlite
from datetime import datetime
import asyncio
from database.init import get_db_path

db_path = get_db_path()

async def add_data(name: str, categories: str | None, app_path: str, icon_path: str | None, command: str | None) -> None:
    now = datetime.now()
    now = now.strftime("%Y-%m-%d %H:%M:%S")

    async with aiosqlite.connect(db_path) as conn:
        await conn.execute("""
            INSERT OR IGNORE INTO applications (name, categories, app_path, icon_path, command, last_used, last_checked) VALUES (?, ?, ?, ?, ?, ?, ?)
        """, (name, categories, app_path, icon_path, command, now, now))
        await conn.commit()


async def add_batch_of_data(items: list[tuple]) -> None:
    async with aiosqlite.connect(db_path) as conn:
        await conn.executemany("""
            INSERT OR IGNORE INTO applications (name, categories, app_path, icon_path, command, last_used, last_checked) VALUES (?, ?, ?, ?, ?, ?, ?)
        """, items)
        await conn.commit()
async def delete_data(app_path: str) -> None:
    async with aiosqlite.connect(db_path) as conn:
        await conn.execute("""
            DELETE FROM applications WHERE app_path = ?
        """, (app_path,))
        await conn.commit()

async def update_datetime(app_path: str):
    now = datetime.now()
    now = now.strftime("%Y-%m-%d %H:%M:%S")

    async with aiosqlite.connect(db_path) as conn:
        await conn.execute("""
            UPDATE applications SET last_used = ? WHERE app_path = ?
        """, (now, app_path))
        await conn.commit()

async def get_data():
    async with aiosqlite.connect(db_path) as conn:
        cursor = await conn.execute(
            "SELECT * FROM applications",
        )
        result = await cursor.fetchall()
        return result

async def get_all_categories():
    async with aiosqlite.connect(db_path) as conn:
        cursor = await conn.execute(
            "SELECT categories FROM applications",
        )
        result = await cursor.fetchall()
        return result

async def get_apps_by_category(category: str):
    async with aiosqlite.connect(db_path) as conn:
        cursor = await conn.execute("""
            SELECT *
            FROM applications
            WHERE ';' || categories || ';' LIKE
            '%;' || ? || ';%'
        """, (category,))
        result = await cursor.fetchall()
        return result

if __name__ == "__main__":
    print(asyncio.run(get_all_categories()))
