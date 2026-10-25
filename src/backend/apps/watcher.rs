use super::apps_search::{app_dirs, Apps};
use super::db_init::open_conn;
use notify::{EventKind, RecursiveMode, Watcher};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

// Package installs write several .desktop files in a burst: wait until the
// dirs are quiet this long, then refresh once with every file complete.
const REFRESH_DEBOUNCE: Duration = Duration::from_secs(2);

pub fn start_watcher() {
    thread::spawn(move || {
        let mut conn = open_conn().expect("Failed to init db conn by watcher");
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx).expect("failed to create watcher");

        // shortcut: dirs created after startup (first flatpak install) aren't watched until restart
        for dir in app_dirs() {
            if let Err(err) = watcher.watch(&dir, RecursiveMode::Recursive) {
                eprintln!("failed to watch {}: {err}", dir.display());
            }
        }

        let mut pending = false;
        loop {
            // Duration::MAX = plain blocking recv while nothing is pending
            let timeout = if pending { REFRESH_DEBOUNCE } else { Duration::MAX };
            match rx.recv_timeout(timeout) {
                Ok(Ok(event)) => {
                    // Modify covers in-place edits and rename-into-place (atomic save)
                    pending |= matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) && event
                        .paths
                        .iter()
                        .any(|path| path.extension().is_some_and(|ext| ext == "desktop"));
                }
                Ok(Err(_)) => {}
                Err(RecvTimeoutError::Timeout) => {
                    pending = false;
                    // Full refresh in this background thread: icon cache (new
                    // packages bring new icons) plus a re-parse of all .desktop
                    // files. find_apps also drops removed/hidden apps and brings
                    // back entries a deleted override was shadowing.
                    if let Err(err) = Apps::refresh_icons(&mut conn)
                        .and_then(|icons| Apps::find_apps(&mut conn, &icons))
                    {
                        eprintln!("watcher refresh error: {err}");
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}
