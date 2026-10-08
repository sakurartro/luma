use rusqlite::{params, Connection};
use anyhow::Result;
use super::apps_search::App;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ponytail: score = count * 0.5^(days/45), counts stored as integers;
// switch to a launch-history table if per-launch decay is ever needed
fn decayed_count(count: i64, last: i64, now: i64) -> i64 {
    let days = ((now - last) / 86_400) as f64;
    (count as f64 * 0.5f64.powf(days / 45.0)).round() as i64
}

pub fn insert_batch(batch: &[App], conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    {
        let mut apps_stmt = tx.prepare(
            r#"
            INSERT INTO applications (
                name,
                app_path,
                icon_path,
                command
            )
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT (app_path) DO UPDATE SET
                name=excluded.name,
                icon_path=excluded.icon_path,
                command=excluded.command
            RETURNING ID
            "#,
        )?;
        let mut del_stale = tx.prepare(
            r#"
            DELETE FROM application_categories
            WHERE application_id = ?1 AND category = ?2
            "#,
        )?;
        let mut existing_stmt = tx.prepare(
            r#"
            SELECT category FROM application_categories WHERE application_id = ?1
            "#,
        )?;
        let mut categories_stmt = tx.prepare(
            r#"
            INSERT OR IGNORE INTO application_categories (
                application_id,
                category
            )
            VALUES (?1, ?2)
            "#,
        )?;

        for app in batch {
            let application_id: i64 = apps_stmt.query_row(params![
                &app.app_name,
                &app.path,
                &app.icon_path,
                &app.command,
            ],
            |row| row.get(0)
            )?;

            // Sync categories without wiping launch stats: INSERT OR IGNORE
            // keeps existing rows' times_launched/last_launched, stale rows
            // (category gone from the .desktop) are dropped.
            let mut existing: HashSet<String> = existing_stmt
                .query_map([application_id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;

            for category in &app.categories {
                existing.remove(category);
                categories_stmt.execute(params![application_id, category])?;
            }

            for stale in existing {
                del_stale.execute(params![application_id, stale])?;
            }


        }
    }

    tx.commit()?;

    Ok(())
}

pub fn record_launch(conn: &mut Connection, path: &str) -> Result<()> {
    let now = now_secs();
    let (id, count, last): (i64, i64, Option<i64>) = conn
        .query_row(
            "SELECT id, times_launched, last_launched FROM applications WHERE app_path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|err| anyhow::anyhow!("record_launch: app not found: {path}: {err}"))?;

    // half-life decay on long breaks (variant 1); short breaks don't bother
    let count = match last {
        Some(last) if now - last > 3 * 86_400 => decayed_count(count, last, now),
        _ => count,
    };

    conn.execute(
        "UPDATE applications SET times_launched = ?1, last_launched = ?2 WHERE app_path = ?3",
        params![count + 1, now, path],
    )?;

    // bump the app's categories with the same decay
    let mut stmt = conn.prepare(
        "SELECT category, times_launched, COALESCE(last_launched, 0) FROM application_categories WHERE application_id = ?1",
    )?;
    let cats = stmt
        .query_map([id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (cat, count, last) in cats {
        conn.execute(
            "UPDATE application_categories SET times_launched = ?1, last_launched = ?2
             WHERE application_id = ?3 AND category = ?4",
            params![decayed_count(count, last, now) + 1, now, id, cat],
        )?;
    }

    Ok(())
}


pub fn del_app(conn: &mut Connection, path: String) -> Result<()> {
    conn.execute("DELETE FROM applications WHERE app_path = ?", [path],)?;
    Ok(())
}

pub fn get_data(conn: &mut Connection) -> rusqlite::Result<Vec<App>> {
    let now = now_secs();
    let mut stmt = conn.prepare(
        "SELECT id, name, app_path, icon_path, command, times_launched, COALESCE(last_launched, 0) FROM applications",
    )?;

    let mut apps: Vec<(i64, App)> = stmt
        .query_map([], |row| {
            let count: i64 = row.get(5)?;
            let last: i64 = row.get(6)?;
            Ok((
                row.get(0)?,
                App {
                    app_name: row.get(1)?,
                    path: row.get(2)?,
                    icon_path: row.get(3)?,
                    command: row.get(4)?,
                    categories: Vec::new(),
                    time_launched: count,
                    last_launched: last,
                    score: decayed_count(count, last, now),
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for (id, app) in &mut apps {
        app.categories = get_categories(conn, *id)?;
    }

    let mut apps: Vec<App> = apps.into_iter().map(|(_, app)| app).collect();
    apps.sort_by(|a, b| b.score.cmp(&a.score));
    Ok(apps)
}

pub fn get_categories(conn: &Connection, id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT category FROM application_categories WHERE application_id = ?1"
    )?;

    let categories = stmt
        .query_map([id], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<String>>>()?;

    Ok(categories)
}

// Category pills ranked by the same decayed score; per-category score is the
// sum of its per-app rows. Ties sort alphabetically.
pub fn get_category_scores(conn: &mut Connection) -> rusqlite::Result<Vec<(String, i64)>> {
    let now = now_secs();
    let mut stmt = conn.prepare(
        "SELECT category, times_launched, COALESCE(last_launched, 0) FROM application_categories",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut scores: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for (cat, count, last) in rows {
        *scores.entry(cat).or_insert(0) += decayed_count(count, last, now);
    }

    let mut out: Vec<(String, i64)> = scores.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(out)
}
