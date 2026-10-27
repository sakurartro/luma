slint::include_modules!();
mod backend;
mod toggle;
mod ui;
use anyhow::Result;
use backend::apps::apps_search::Apps;
use backend::apps::db_init::{DbState, open_conn};
use backend::apps::watcher::start_watcher;
use backend::files::clip;
use backend::files::ocr;
use backend::files::files_search;
use backend::files::watcher::start_files_watcher;
use backend::settings;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

// Result of the background init, picked up by the UI-thread drain timer.
// (No cross-thread invoke: slint can silently drop those around loop start.)
static INIT_DB: Mutex<Option<DbState>> = Mutex::new(None);

// Toggle requests from the keybind socket; drained by a timer on the UI thread.
static TOGGLES: AtomicUsize = AtomicUsize::new(0);

fn main() -> Result<()> {
    // Any launch while an instance is live becomes a toggle request instead of
    // a second daemon.
    let toggle_arg = std::env::args().any(|arg| arg == "--toggle");
    if toggle::notify_running() {
        return Ok(());
    }

    // Bind the toggle socket before anything heavy: keybinds pressed while we
    // initialize then reach us instead of spawning duplicate instances.
    let listener = match toggle::bind_socket() {
        Some(listener) => listener,
        // A live instance appeared between notify_running() and the bind; it
        // owns the socket, so we step aside.
        None => return Ok(()),
    };

    // Open right away unless we were started by the service (systemd sets
    // LUMA_SERVICE=1 in the unit): a keybind spawn that didn't find a running
    // instance became the instance itself, so that press must open the window.
    let open_on_start = !toggle_arg || std::env::var_os("LUMA_SERVICE").is_none();

    // Tiny "Loading..." pill shown instantly on keybind while the background
    // init runs; hidden once the real window is up. Not shown for service start.
    // set_xdg_app_id goes into create_ui: it fails with NoPlatform before any
    // window exists, because it does not initialize the platform itself.
    let loading = if open_on_start {
        let w = LoadingWindow::new()?;
        let _ = w.show();
        Some(w)
    } else {
        None
    };

    enum UiState {
        // Background init (icons/db scan) still running; toggles wait in TOGGLES.
        NotReady,
        // Init done, window not created yet: slint/winit on Wayland force-create
        // and show the window at event-loop start, so for --toggle the window is
        // created lazily on the first toggle to keep the start in the background.
        Fresh { db_state: DbState },
        Live {
            window: MainWindow,
            visible: Rc<Cell<bool>>,
        },
    }

    thread_local! {
        static UI: RefCell<UiState> = RefCell::new(UiState::NotReady);
        static LOADING: RefCell<Option<LoadingWindow>> = const { RefCell::new(None) };
    }
    if let Some(l) = loading {
        LOADING.with_borrow_mut(|slot| *slot = Some(l));
    }

    fn drain_toggles() {
        UI.with_borrow_mut(|s| {
            if let UiState::NotReady = s {
                if let Some(db_state) = INIT_DB.lock().unwrap().take() {
                    *s = UiState::Fresh { db_state };
                }
            }
            match s {
            UiState::NotReady => {} // init still running; counter keeps accumulating
            UiState::Fresh { .. } => {
                if TOGGLES.swap(0, Ordering::Relaxed) > 0 {
                    let old = std::mem::replace(s, UiState::NotReady);
                    if let UiState::Fresh { db_state } = old {
                        let visible = Rc::new(Cell::new(false));
                        match ui::create_ui(db_state, visible.clone()) {
                            Ok(window) => {
                                *s = UiState::Live { window, visible };
                                LOADING.with_borrow_mut(|slot| {
                                    if let Some(lw) = slot.take() {
                                        let _ = lw.hide();
                                    }
                                });
                            }
                            // Keep the db_state for a retry on the next toggle;
                            // the Loading pill stays up as the retry indicator.
                            Err((err, db_state)) => {
                                eprintln!("failed to create window: {err}");
                                *s = UiState::Fresh { db_state };
                            }
                        }
                    }
                }
            }
            UiState::Live { window, visible } => {
                ui::update_index_progress(window);
                if TOGGLES.swap(0, Ordering::Relaxed) > 0 {
                    if visible.get() {
                        ui::hide_window(window, visible);
                    } else {
                        ui::show_window(window, visible);
                    }
                }
            }
            }
        });
    }

    // Read once on the main thread: the restart check of the settings menu
    // compares against exactly what the indexers below were started with.
    let st = settings::load();
    ui::remember_startup(&st);

    // Heavy init in the background so the event loop (and the toggle socket)
    // is up in milliseconds; the drain timer picks up the result. Apps scan
    // first, then the state is handed to the UI — file indexing keeps going
    // in the background with progress reported via files_search::INDEX_*.
    std::thread::spawn(move || {
        let init = (|| -> Result<DbState> {
            let mut db_state = DbState::init()?;
            let icons = Apps::initial_icons_scan(&mut db_state.conn)?;
            let apps_obj = Apps::find_apps(&mut db_state.conn, &icons)?;
            start_watcher();
            let _ = apps_obj.apps;
            Ok(db_state)
        })();
        match init {
            Ok(db_state) => {
                *INIT_DB.lock().unwrap() = Some(db_state);

                // Own connection: db_state.conn already moved to the UI side.
                let dirs = st.search_dirs;
                match open_conn() {
                    Ok(mut conn) => {
                        if let Err(err) = files_search::scan_dirs(&mut conn, &dirs) {
                            eprintln!("file scan error: {err}");
                        }
                    }
                    Err(err) => eprintln!("failed to open db for file scan: {err}"),
                }
                files_search::INDEX_DONE.store(true, Ordering::Relaxed);
                start_files_watcher(&dirs);
                // Own thread; no-op when the `semantic` setting is off.
                clip::start_indexing(st.semantic);
                // Waits for the clip worker, so keep it after the call above;
                // no-op when the `ocr` setting is off.
                ocr::start_indexing(st.ocr);
            }
            Err(err) => eprintln!("init error: {err}"),
        }
    });

    if open_on_start {
        TOGGLES.store(1, Ordering::Relaxed);
    }

    toggle::start_listener(listener, || {
        TOGGLES.fetch_add(1, Ordering::Relaxed);
    });

    // Poll toggle requests on the UI thread; the listener thread can't invoke
    // into the loop reliably before/around loop start, so no events are lost.
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(30),
        drain_toggles,
    );

    // The event loop must not quit when the last window is hidden, hence _until_quit.
    slint::run_event_loop_until_quit()?;
    Ok(())
}
