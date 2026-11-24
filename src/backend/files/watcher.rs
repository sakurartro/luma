use super::db_service::{del_path, insert_entries};
use super::files_search::{DB_VERSION, FileEntry, expand_home, is_excluded, walk_entries};
use crate::backend::apps::db_init::{config_dir, open_conn};
use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;

pub fn start_files_watcher(dirs: &[String], exclude: &[String]) {
    let dirs = dirs.to_vec();
    let mut excluded: Vec<PathBuf> = exclude.iter().map(|dir| expand_home(dir)).collect();
    // Indexing the database's own writes would feed the watcher indefinitely.
    excluded.push(config_dir());
    thread::spawn(move || {
        let mut conn = open_conn().expect("Failed to init db conn by files watcher");
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
            if let Err(err) = apply_event(&mut conn, &event, &excluded) {
                eprintln!("file watcher update error: {err}");
            }
        }
    });
}

fn apply_event(conn: &mut Connection, event: &Event, excluded: &[PathBuf]) -> anyhow::Result<()> {
    let (walk_subtree, invalidate, check_old_subtree) = match event.kind {
        // notify's inotify backend emits From and To before their combined event.
        #[cfg(target_os = "linux")]
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => return Ok(()),
        EventKind::Create(_) => (true, false, false),
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => (true, true, false),
        EventKind::Modify(ModifyKind::Name(_)) | EventKind::Remove(_) => (true, true, true),
        EventKind::Modify(_) => (false, true, false),
        _ => return Ok(()),
    };

    let paths: Vec<_> = event.paths.iter().filter(|path| !is_excluded(path, excluded)).collect();
    if paths.is_empty() {
        return Ok(());
    }

    let mut entries = Vec::new();
    for path in &paths {
        // Do not walk symlink targets. A symlink itself can still be indexed,
        // just as it is during the startup scan.
        if walk_subtree && path.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
            entries.extend(walk_entries(path, excluded).map(|entry| FileEntry::from_path(entry.path())));
        } else if path.is_dir() || path.is_file() {
            entries.push(FileEntry::from_path(path));
        }
    }

    let tx = conn.transaction()?;
    if invalidate {
        for path in paths {
            del_path(&tx, &path.to_string_lossy(), check_old_subtree)?;
        }
    }
    if !entries.is_empty() {
        insert_entries(&entries, &tx)?;
    }
    tx.commit()?;

    DB_VERSION.fetch_add(1, Ordering::Relaxed);
    schedule_content_indexing(&entries);
    Ok(())
}

fn schedule_content_indexing(entries: &[FileEntry]) {
    let mut photo = false;
    let mut video = false;
    let mut doc = false;
    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        photo |= super::files_search::is_image(&entry.path);
        video |= super::files_search::is_video(&entry.path);
        doc |= super::docs::is_doc(Path::new(&entry.path));
    }
    if photo || video {
        super::clip::schedule_reindex();
        super::ocr::schedule_reindex();
    }
    if video {
        super::video::speech_index::schedule_reindex();
    }
    if doc {
        super::docs::schedule_reindex();
    }
}
