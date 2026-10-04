slint::include_modules!();
mod backend;
mod toggle;
use anyhow::Result;
use backend::apps::apps_search::{App, Apps};
use backend::apps::db_init::DbState;
use backend::apps::fuzzy;
use backend::apps::watcher::start_watcher;
use backend::general_features::engine;
use slint::VecModel;
use slint::winit_030::{WinitWindowAccessor, invoke_from_active_event_loop};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

// Toggle requests from the keybind socket; drained by a timer on the UI thread.
static TOGGLES: AtomicUsize = AtomicUsize::new(0);

fn build_item(app: &App, cache: &RefCell<HashMap<String, slint::Image>>) -> AppItem {
    let icon = cache
        .borrow_mut()
        .entry(app.icon_path.clone())
        .or_insert_with(|| {
            slint::Image::load_from_path(Path::new(&app.icon_path)).unwrap_or_default()
        })
        .clone();
    AppItem {
        name: app.app_name.as_str().into(),
        path: app.path.as_str().into(),
        command: app.command.as_str().into(),
        icon,
    }
}

fn build_items(apps: &[App], cache: &RefCell<HashMap<String, slint::Image>>) -> Vec<AppItem> {
    apps.iter().map(|app| build_item(app, cache)).collect()
}

// Deferred window creation (slint waits for the xdg-desktop-portal appearance
// query) needs the event loop to wake up; firing timers does that. Poke until
// the winit window materializes or give up.
fn poke_window_materialization(weak_window: slint::Weak<MainWindow>, tries: u32) {
    let Some(window) = weak_window.upgrade() else { return };
    if window.window().has_winit_window() || tries == 0 {
        return;
    }
    slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
        poke_window_materialization(weak_window, tries - 1);
    });
}

fn show_window(window: &MainWindow, visible: &Cell<bool>) {
    // app lives in background, clear last query for a clean reopen
    window.set_query_result("".into());
    window.set_input_text("".into());
    window.invoke_reset_view();
    let _ = window.show();
    visible.set(true);
    window.invoke_focus_input();
    if !window.window().has_winit_window() {
        poke_window_materialization(window.as_weak(), 100);
    }
    window.window().with_winit_window(|w| w.focus_window());
}

fn hide_window(window: &MainWindow, visible: &Cell<bool>) {
    let _ = window.hide();
    visible.set(false);
}

fn main() -> Result<()> {
    // --toggle cold start goes to the background hidden; a plain launch shows the window.
    let toggle_arg = std::env::args().any(|arg| arg == "--toggle");
    if toggle_arg && toggle::notify_running() {
        return Ok(());
    }

    // Bind the toggle socket before anything heavy: keybinds pressed while we
    // initialize then reach us instead of spawning duplicate instances.
    let listener = toggle::bind_socket();

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
            icon_cache: Rc<RefCell<HashMap<String, slint::Image>>>,
        },
    }

    thread_local! {
        static UI: RefCell<UiState> = RefCell::new(UiState::NotReady);
    }

    fn create_ui(mut db_state: DbState) -> Result<UiState> {
        let main_window = MainWindow::new()?;
        slint::set_xdg_app_id("luma-rust")?;

        let weak_window = main_window.as_weak();
        let icon_cache = Rc::new(RefCell::new(HashMap::new()));
        let all_apps: Rc<RefCell<Vec<App>>> = Rc::new(RefCell::new(Vec::new()));

        main_window.on_show_apps({
            let icon_cache = icon_cache.clone();
            let all_apps = all_apps.clone();
            move || {
                if let Some(window) = weak_window.upgrade() {
                    let apps_refresh_obj = match Apps::refresh(&mut db_state.conn) {
                        Ok(apps) => apps,
                        Err(err) => {
                            eprintln!("refresh error, {}", err);
                            return;
                        }
                    };
                    *all_apps.borrow_mut() = apps_refresh_obj.apps;
                    window.set_apps(
                        Rc::new(VecModel::from(build_items(&all_apps.borrow(), &icon_cache)))
                            .into(),
                    );
                }
            }
        });
        main_window.on_search_apps({
            let icon_cache = icon_cache.clone();
            let all_apps = all_apps.clone();
            let weak = main_window.as_weak();
            move |query: slint::SharedString| {
                if let Some(window) = weak.upgrade() {
                    let apps = all_apps.borrow();
                    let filtered = fuzzy::fuzzy_search(&apps, query.as_str());
                    let items: Vec<AppItem> = filtered
                        .iter()
                        .map(|app| build_item(app, &icon_cache))
                        .collect();
                    window.set_apps(Rc::new(VecModel::from(items)).into());
                }
            }
        });
        let weak_query = main_window.as_weak();
        main_window.on_run_query(move |text| {
            let weak = weak_query.clone();
            let text = text.to_string();
            std::thread::spawn(move || {
                let msg = match engine::general_engine(text) {
                    Ok(out) => out,
                    Err(err) => format!("Error: {err}"),
                };
                let _ = weak.upgrade_in_event_loop(move |window| {
                    window.set_query_result(msg.into());
                });
            });
        });
        main_window.on_launch_app(|command| {
            let mut parts = command.split_whitespace();

            let Some(program) = parts.next() else {
                return;
            };

            if let Err(err) = Command::new(program).args(parts).spawn() {
                eprintln!("Failed to launch {program}, {err}");
            }
        });

        let visible = Rc::new(Cell::new(false));
        let weak_hide = main_window.as_weak();
        let visible_hide = visible.clone();
        main_window.on_hide_window(move || {
            // Esc goes through here too, so the visibility flag stays in sync
            if let Some(window) = weak_hide.upgrade() {
                hide_window(&window, &visible_hide);
            }
        });

        show_window(&main_window, &visible);
        Ok(UiState::Live {
            window: main_window,
            visible,
            icon_cache,
        })
    }

    fn drain_toggles() {
        UI.with_borrow_mut(|s| match s {
            UiState::NotReady => {} // init still running; counter keeps accumulating
            UiState::Fresh { .. } => {
                if TOGGLES.swap(0, Ordering::Relaxed) > 0 {
                    let old = std::mem::replace(s, UiState::NotReady);
                    if let UiState::Fresh { db_state } = old {
                        match create_ui(db_state) {
                            Ok(live) => *s = live,
                            Err(err) => eprintln!("failed to create window: {err}"),
                        }
                    }
                }
            }
            UiState::Live { window, visible, .. } => {
                if TOGGLES.swap(0, Ordering::Relaxed) > 0 {
                    if visible.get() {
                        hide_window(window, visible);
                    } else {
                        show_window(window, visible);
                    }
                }
            }
        });
    }

    // Heavy init in the background so the event loop (and the toggle socket)
    // is up in milliseconds; the result is handed over to the UI thread.
    std::thread::spawn(move || {
        let init = (|| -> Result<DbState> {
            let icons = Apps::initial_icons_scan()?;
            let mut db_state = DbState::init()?;
            let apps_obj = Apps::find_apps(&mut db_state.conn, &icons)?;
            start_watcher(icons);
            let _ = apps_obj.apps;
            Ok(db_state)
        })();
        let _ = invoke_from_active_event_loop(move |_| {
            UI.with_borrow_mut(|s| {
                if let UiState::NotReady = s {
                    match init {
                        Ok(db_state) => *s = UiState::Fresh { db_state },
                        Err(err) => eprintln!("init error: {err}"),
                    }
                }
            });
        });
    });

    // Open right away unless we were started by the service (systemd sets
    // INVOCATION_ID): a keybind spawn that didn't find a running instance
    // became the instance itself, so that press must still open the window.
    if !toggle_arg || std::env::var_os("INVOCATION_ID").is_none() {
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
