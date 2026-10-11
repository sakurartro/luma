use crate::backend::apps::db_init::db_path;
use super::db_service::{del_entry, insert_batch};
use super::files_search::{expand_home, FileEntry};
use notify::{EventKind, RecursiveMode, Watcher};
use rusqlite::Connection;
use std::sync::mpsc;
use std::thread;

pub fn start_files_watcher(dirs: &[String]) {
    let dirs = dirs.to_vec();
    thread::spawn(move || {
        let mut conn =
            Connection::open(db_path()).expect("Failed to init db conn by files watcher");
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx).expect("failed to create files watcher");

        for dir in &dirs {
            let path = expand_home(dir);
            if path.is_dir() {
                let _ = watcher.watch(&path, RecursiveMode::Recursive);
            }
        }

        for event in rx {
            let Ok(event) = event else {
                continue;
            };

            match event.kind {
                EventKind::Create(_) => {
                    for path in event.paths {
                        if path.is_dir() || path.is_file() {
                            // shortcut: contents of a moved-in tree stay unindexed
                            // until the next startup rescan; inotify only sees
                            // the moved dir itself
                            let entry = FileEntry {
                                name: path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                path: path.to_string_lossy().into_owned(),
                                is_dir: path.is_dir(),
                            };
                            let _ = insert_batch(&[entry], &mut conn);
                        }
                    }
                }

                EventKind::Remove(_) => {
                    for path in event.paths {
                        let _ = del_entry(&mut conn, path.to_string_lossy().to_string());
                    }
                }
                _ => {}
            }
        }
    });
}