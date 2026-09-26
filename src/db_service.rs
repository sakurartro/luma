use rusqlite::{params, Connection};
use anyhow::Result;
use crate::apps_search::App;

pub fn insert_batch(batch: &[App], conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;

    {
        let mut apps_stmt = tx.prepare(
            r#"
            INSERT INTO applications (
                name,
                app_path,
                icon_path,
                command,
                launch_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (app_path) DO UPDATE SET
                name=excluded.name,
                app_path=excluded.app_path,
                icon_path=excluded.icon_path,
                command=excluded.command,
                launch_count=excluded.launch_count
            RETURNING ID
            "#,
        )?;
        let mut del_categories = tx.prepare(
            r#"
            DELETE FROM application_categories WHERE application_id = ?
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
                0,
            ],
            |row| row.get(0)
            )?;

            del_categories.execute(params![
                application_id,
            ])?;

            for category in &app.categories {
                categories_stmt.execute(params![
                    application_id,
                    category,
                ])?;
            }


        }
    }

    tx.commit()?;

    Ok(())
}


pub fn del_app(conn: &mut Connection, path: String) -> Result<()> {
    conn.execute("DELETE FROM applications WHERE app_path = ?", [path],)?;
    Ok(())
}

pub fn get_data(conn: &mut Connection) -> rusqlite::Result<Vec<App>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, app_path, icon_path, command FROM applications"
    )?;

    let apps = stmt
        .query_map([], |row| {
            Ok(App {
                app_name: row.get(1)?,
                path: row.get(2)?,
                icon_path: row.get(3)?,
                command: row.get(4)?,
                categories: get_categories(conn, row.get::<_, i64>(0)?)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<App>>>()?;
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
