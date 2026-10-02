slint::include_modules!();
mod db_init;
mod db_service;
mod toggle;
mod watcher;
use apps_search::Apps;
use db_init::DbState;
use slint::VecModel;
use slint::winit_030::{WinitWindowAccessor, invoke_from_active_event_loop};
use std::collections::HashMap;
use std::rc::Rc;
use watcher::start_watcher;
mod apps_search;
use crate::apps_search::App;
use anyhow::Result;
use std::cell::RefCell;
use std::path::Path;
use std::process::Command;

fn build_items(apps: Vec<App>, cache: &RefCell<HashMap<String, slint::Image>>) -> Vec<AppItem> {
    apps.into_iter()
        .map(|app| {
            let icon = cache
                .borrow_mut()
                .entry(app.icon_path.clone())
                .or_insert_with(|| {
                    slint::Image::load_from_path(Path::new(&app.icon_path)).unwrap_or_default()
                })
                .clone();
            AppItem {
                name: app.app_name.into(),
                path: app.path.into(),
                command: app.command.into(),
                icon,
            }
        })
        .collect()
}

fn toggle_window(window: &MainWindow) {
    if window.window().is_visible() {
        let _ = window.hide();
    } else {
        let _ = window.show();
        window.window().with_winit_window(|w| w.focus_window());
    }
}

fn main() -> Result<()> {
    // --toggle cold start goes to the background hidden; a plain launch shows the window.
    let toggle_arg = std::env::args().any(|arg| arg == "--toggle");
    if toggle_arg && toggle::notify_running() {
        return Ok(());
    }

    let icons = Apps::initial_icons_scan()?;
    let mut db_state = DbState::init()?;
    let apps_obj = Apps::find_apps(&mut db_state.conn, &icons)?;
    start_watcher(icons);
    let apps = apps_obj.apps;
    let main_window = MainWindow::new()?;
    let weak_window = main_window.as_weak();
    slint::set_xdg_app_id("luma-rust")?;
    let icon_cache = Rc::new(RefCell::new(HashMap::new()));
    let slint_apps = build_items(apps, &icon_cache);
    let model = Rc::new(VecModel::from(slint_apps));
    main_window.on_show_apps({
        let icon_cache = icon_cache.clone();
        move || {
            if let Some(window) = weak_window.upgrade() {
                let apps_refresh_obj = match Apps::refresh(&mut db_state.conn) {
                    Ok(apps) => apps,
                    Err(err) => {
                        eprintln!("refresh error, {}", err);
                        return;
                    }
                };
                let refreshed_apps = apps_refresh_obj.apps;
                let slint_apps = build_items(refreshed_apps, &icon_cache);

                window.set_apps(Rc::new(VecModel::from(slint_apps)).into());
            }
        }
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

    let weak_window = main_window.as_weak();
    toggle::start_listener(move || {
        let weak_window = weak_window.clone();
        let _ = invoke_from_active_event_loop(move |_| {
            if let Some(window) = weak_window.upgrade() {
                toggle_window(&window);
            }
        });
    });

    // Window starts hidden (when started via --toggle) and stays alive in the background;
    // the event loop must not quit when the last window is hidden, hence _until_quit.
    if !toggle_arg {
        let _ = main_window.show();
    }
    slint::run_event_loop_until_quit()?;
    Ok(())
}
