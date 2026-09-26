use std::{
    path::Path,
    sync::mpsc,
    thread,
};
use notify::{RecursiveMode, Watcher, EventKind};
use rusqlite::{Connection};
use crate::apps_search::Apps;
use crate::db_service::del_app;

pub fn start_watcher() {
    thread::spawn(move || {
        let mut conn = Connection::open("main.sqlite3")
            .expect("Failed to init db conn by watcher");
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx)
            .expect("failed to create watcher");

        watcher
            .watch (
                Path::new("/usr/share/applications"),
                RecursiveMode::Recursive,
            )
            .expect("failed to watch directory");

        for event in rx {
            let Ok(event) = event else {
                continue;
            };

            match event.kind {
                EventKind::Create(_) => {
                    for path in event.paths {
                        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("desktop") {
                            let _ = Apps::parse_config(&mut conn, vec![path]);
                        }
                    }
                }

                EventKind::Remove(_) => {
                    for path in event.paths {
                        if path.extension().and_then(|ext| ext.to_str()) == Some("desktop") {
                            del_app(&mut conn, path.to_string_lossy().to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    });
}
