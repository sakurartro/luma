use super::apps_search::Apps;
use super::db_service::del_app;
use notify::{EventKind, RecursiveMode, Watcher};
use rusqlite::Connection;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// Package installs write several .desktop files in a burst; one background
// refresh per burst is enough, no need to rescan icons per event.
const REFRESH_DEBOUNCE: Duration = Duration::from_secs(2);

pub fn start_watcher() {
    thread::spawn(move || {
        let mut conn =
            Connection::open(super::db_init::db_path()).expect("Failed to init db conn by watcher");
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx).expect("failed to create watcher");

        watcher
            .watch(
                Path::new("/usr/share/applications"),
                RecursiveMode::Recursive,
            )
            .expect("failed to watch directory");

        let mut last_refresh = Instant::now() - REFRESH_DEBOUNCE;

        for event in rx {
            let Ok(event) = event else {
                continue;
            };

            match event.kind {
                EventKind::Create(_) => {
                    let new_desktops = event
                        .paths
                        .iter()
                        .filter(|path| {
                            path.is_file()
                                && path.extension().and_then(|ext| ext.to_str()) == Some("desktop")
                        })
                        .count();

                    if new_desktops > 0 && last_refresh.elapsed() >= REFRESH_DEBOUNCE {
                        last_refresh = Instant::now();
                        // Full refresh in this background thread: icon cache
                        // (new packages bring new icons) plus a re-parse of all
                        // .desktop files, whose upsert also refreshes icon_path
                        // and keeps launch stats.
                        match Apps::refresh_icons(&mut conn)
                            .and_then(|icons| Apps::find_apps(&mut conn, &icons))
                        {
                            Ok(_) => {}
                            Err(err) => eprintln!("watcher refresh error: {err}"),
                        }
                    }
                }

                EventKind::Remove(_) => {
                    for path in event.paths {
                        if path.extension().and_then(|ext| ext.to_str()) == Some("desktop") {
                            let _ = del_app(&mut conn, path.to_string_lossy().to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    });
}