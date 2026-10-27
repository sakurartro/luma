mod apps;
mod files;
mod power;
mod search;
mod settings;

use crate::backend::apps::apps_search::App;
use crate::backend::apps::db_init::DbState;
use crate::backend::files::files_search::FileEntry;
use crate::MainWindow;
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub use search::update_index_progress;
pub use settings::remember_startup;

// State shared by the callbacks of create_ui; each wire() clones what it needs.
#[derive(Clone)]
struct Ctx {
    icon_cache: Rc<RefCell<HashMap<String, slint::Image>>>,
    all_apps: Rc<RefCell<Vec<App>>>,
    all_files: Rc<RefCell<Vec<FileEntry>>>,
    // DB_VERSION at the time all_files was loaded; 0 entries force a load
    files_db_version: Rc<Cell<usize>>,
    // Throttle clock for files::load_files_if_stale
    last_files_load: Rc<Cell<Option<std::time::Instant>>>,
    // Live settings, replaced by the settings menu
    hide_patterns: Rc<RefCell<Vec<String>>>,
    currency: Rc<Cell<bool>>,
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

pub fn show_window(window: &MainWindow, visible: &Cell<bool>) {
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

pub fn hide_window(window: &MainWindow, visible: &Cell<bool>) {
    let _ = window.hide();
    visible.set(false);
}

// On failure db_state comes back so the next toggle can retry the whole
// init instead of dropping the connection for good.
pub fn create_ui(db_state: DbState, visible: Rc<Cell<bool>>) -> Result<MainWindow, (anyhow::Error, DbState)> {
    let main_window = match MainWindow::new() {
        Ok(window) => window,
        Err(err) => return Err((anyhow::Error::from(err), db_state)),
    };
    if let Err(err) = slint::set_xdg_app_id("luma-rust") {
        return Err((anyhow::Error::from(err), db_state));
    }

    let ctx = Ctx {
        icon_cache: Rc::new(RefCell::new(HashMap::new())),
        all_apps: Rc::new(RefCell::new(Vec::new())),
        all_files: Rc::new(RefCell::new(Vec::new())),
        files_db_version: Rc::new(Cell::new(0)),
        last_files_load: Rc::new(Cell::new(None)),
        hide_patterns: Rc::new(RefCell::new(Vec::new())),
        currency: Rc::new(Cell::new(true)),
    };
    settings::apply_settings(&main_window, &crate::backend::settings::load(), &ctx);

    power::wire(&main_window);
    apps::wire(&main_window, &ctx, db_state);
    files::wire(&main_window, &ctx);
    search::wire(&main_window, &ctx);
    settings::wire(&main_window, &ctx);

    let weak_hide = main_window.as_weak();
    let visible_hide = visible.clone();
    main_window.on_hide_window(move || {
        // Esc goes through here too, so the visibility flag stays in sync
        if let Some(window) = weak_hide.upgrade() {
            hide_window(&window, &visible_hide);
        }
    });

    show_window(&main_window, &visible);
    update_index_progress(&main_window);
    Ok(main_window)
}
