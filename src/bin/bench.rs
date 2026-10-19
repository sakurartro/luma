// temporary benchmark harness, delete after measuring
// includes whole backend but only benchmarks part of it: dead-code warnings here are noise
#![allow(dead_code, unused_imports)]
#[path = "../backend/mod.rs"]
mod backend;

use backend::apps::apps_search::Apps;
use backend::apps::db_init::DbState;
use backend::apps::db_service::{get_category_scores, get_data};
use backend::apps::fuzzy::fuzzy_search;
use backend::files::db_service::get_files;
use backend::files::files_search::scan_dirs;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() -> anyhow::Result<()> {
    // db_path() honors XDG_CONFIG_HOME, so this redirects DbState to a temp db
    unsafe { std::env::set_var("XDG_CONFIG_HOME", "/tmp/opencode/luma_fakeconf") };
    let _ = std::fs::remove_dir_all("/tmp/opencode/luma_fakeconf");
    let mut conn_db = DbState::init()?;

    let icons = {
        let t = Instant::now();
        let icons = Apps::initial_icons_scan(&mut conn_db.conn)?;
        println!("icons scan ({} icons): {:.1} ms", icons.len(), ms(t));
        icons
    };

    let t = Instant::now();
    let apps_obj = Apps::find_apps(&mut conn_db.conn, &icons)?;
    println!("apps index (parse {} .desktop + insert): {:.1} ms", apps_obj.apps.len(), ms(t));

    let dirs = vec!["/home/sakura".to_string()];
    let t = Instant::now();
    scan_dirs(&mut conn_db.conn, &dirs)?;
    println!("files index (walk home + insert): {:.1} ms", ms(t));

    let t = Instant::now();
    let apps = get_data(&mut conn_db.conn)?;
    println!("apps refresh (get_data, {} apps): {:.2} ms", apps.len(), ms(t));
    let t = Instant::now();
    let cats = get_category_scores(&mut conn_db.conn)?;
    println!("category scores ({} cats): {:.2} ms", cats.len(), ms(t));

    let real = rusqlite::Connection::open("/home/sakura/.config/luma/main.sqlite3")?;
    let t = Instant::now();
    let files = get_files(&real)?;
    println!("files refresh (get_files, {} entries): {:.2} ms", files.len(), ms(t));

    for q in ["", "fi", "fire", "term", "zzzz"] {
        let t = Instant::now();
        let hits = fuzzy_search(&apps, |a| a.name_lower.as_slice(), q);
        println!("apps fuzzy '{q}' -> {} hits: {:.3} ms", hits.len(), ms(t));
    }
    for q in ["", "no", "proj", "luma", "zzzz"] {
        let t = Instant::now();
        let hits = fuzzy_search(&files, |f| f.name_lower.as_slice(), q);
        println!("files fuzzy '{q}' -> {} hits: {:.2} ms", hits.len(), ms(t));
    }

    let _ = std::fs::remove_dir_all("/tmp/opencode/luma_fakeconf");
    Ok(())
}
