import asyncio
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import datetime

import aiosqlite

from database.init import get_db_path


ApplicationRecord = dict[str, object]


@asynccontextmanager
async def _connect() -> AsyncIterator[aiosqlite.Connection]:
    conn = await aiosqlite.connect(get_db_path())
    conn.row_factory = aiosqlite.Row
    try:
        await conn.execute("PRAGMA foreign_keys = ON")
        yield conn
    finally:
        await conn.close()


def _now() -> str:
    return datetime.now().strftime("%Y-%m-%d %H:%M:%S")


def _normalize_categories(categories: list[str]) -> list[str]:
    result: list[str] = []
    seen: set[str] = set()
    for value in categories:
        category = value.strip()
        if category and category not in seen:
            seen.add(category)
            result.append(category)
    return result


async def _replace_categories(
    conn: aiosqlite.Connection,
    application_id: int,
    categories: list[str],
) -> None:
    await conn.execute(
        "DELETE FROM application_categories WHERE application_id = ?",
        (application_id,),
    )
    normalized = _normalize_categories(categories)
    if normalized:
        await conn.executemany("""
            INSERT INTO application_categories (application_id, category)
            VALUES (?, ?)
        """, ((application_id, category) for category in normalized))


async def _upsert_application(
    conn: aiosqlite.Connection,
    *,
    name: str,
    categories: list[str],
    app_path: str,
    icon_path: str | None,
    command: str | None,
    last_used: str,
    last_checked: str,
) -> None:
    cursor = await conn.execute("""
        INSERT INTO applications (
            name, app_path, icon_path, command, last_used, last_checked
        ) VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(app_path) DO UPDATE SET
            name = excluded.name,
            icon_path = excluded.icon_path,
            command = excluded.command,
            last_checked = excluded.last_checked
        RETURNING id
    """, (name, app_path, icon_path, command, last_used, last_checked))
    application_id = (await cursor.fetchone())[0]
    await _replace_categories(conn, application_id, categories)


async def add_data(
    name: str,
    categories: list[str],
    app_path: str,
    icon_path: str | None,
    command: str | None,
) -> None:
    now = _now()
    async with _connect() as conn:
        await _upsert_application(
            conn,
            name=name,
            categories=categories,
            app_path=app_path,
            icon_path=icon_path,
            command=command,
            last_used=now,
            last_checked=now,
        )
        await conn.commit()


async def add_batch_of_data(items: list[ApplicationRecord]) -> None:
    """Upsert mappings with name, categories, app_path, icon_path and command."""
    now = _now()
    async with _connect() as conn:
        for item in items:
            await _upsert_application(
                conn,
                name=item["name"],
                categories=item["categories"],
                app_path=item["app_path"],
                icon_path=item.get("icon_path"),
                command=item.get("command"),
                last_used=item.get("last_used") or now,
                last_checked=item.get("last_checked") or now,
            )
        await conn.commit()


async def delete_data(app_path: str) -> None:
    async with _connect() as conn:
        await conn.execute("DELETE FROM applications WHERE app_path = ?", (app_path,))
        await conn.commit()


async def update_datetime(app_path: str) -> None:
    async with _connect() as conn:
        await conn.execute(
            "UPDATE applications SET last_used = ? WHERE app_path = ?",
            (_now(), app_path),
        )
        await conn.commit()


def _group_rows(rows: list[aiosqlite.Row]) -> list[ApplicationRecord]:
    applications: dict[int, ApplicationRecord] = {}
    for row in rows:
        application_id = row["id"]
        application = applications.setdefault(application_id, {
            "id": application_id,
            "name": row["name"],
            "app_path": row["app_path"],
            "icon_path": row["icon_path"],
            "command": row["command"],
            "last_used": row["last_used"],
            "last_checked": row["last_checked"],
            "categories": [],
        })
        if row["category"] is not None:
            application["categories"].append(row["category"])
    return list(applications.values())


async def _fetch_applications(
    conn: aiosqlite.Connection,
    category: str | None = None,
) -> list[ApplicationRecord]:
    category_join = ""
    params: tuple[str, ...] = ()
    if category is not None:
        category_join = """
            JOIN application_categories AS filter_categories
              ON filter_categories.application_id = a.id
             AND filter_categories.category = ?
        """
        params = (category.strip(),)

    cursor = await conn.execute(f"""
        SELECT
            a.id,
            a.name,
            a.app_path,
            a.icon_path,
            a.command,
            a.last_used,
            a.last_checked,
            ac.category
        FROM applications AS a
        {category_join}
        LEFT JOIN application_categories AS ac ON ac.application_id = a.id
        ORDER BY a.id, ac.category
    """, params)
    return _group_rows(await cursor.fetchall())


async def get_data() -> list[ApplicationRecord]:
    async with _connect() as conn:
        return await _fetch_applications(conn)


async def get_all_categories() -> list[str]:
    async with _connect() as conn:
        cursor = await conn.execute("""
            SELECT DISTINCT category
            FROM application_categories
            ORDER BY category
        """)
        return [row["category"] for row in await cursor.fetchall()]


async def get_apps_by_category(category: str) -> list[ApplicationRecord]:
    async with _connect() as conn:
        return await _fetch_applications(conn, category.strip())


if __name__ == "__main__":
    print(asyncio.run(get_all_categories()))
