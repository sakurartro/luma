use crate::backend::apps::apps_search::{exec_args, in_path, needs_terminal, terminal_argv, App, Apps};
use crate::backend::apps::db_init::{DbState, open_conn};
use crate::backend::apps::db_service::{get_category_scores, record_launch, set_pinned};
use crate::backend::apps::fuzzy;
use crate::{AppItem, MainWindow};
use slint::{ComponentHandle, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;

use super::Ctx;

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
        pinned: app.pinned,
    }
}

fn build_items(apps: &[App], cache: &RefCell<HashMap<String, slint::Image>>) -> Vec<AppItem> {
    apps.iter().map(|app| build_item(app, cache)).collect()
}

// fuzzy name match ∩ selected category; empty category = all
pub(super) fn filter_items(
    apps: &[App],
    query: &str,
    category: &str,
    cache: &RefCell<HashMap<String, slint::Image>>,
) -> Vec<AppItem> {
    fuzzy::fuzzy_search_by(apps, |app| app.name_lower.as_slice(), query, |app| {
        fuzzy::usage_bonus(app.score, app.pinned)
    })
        .into_iter()
        .filter(|app| category.is_empty() || app.categories.iter().any(|cat| cat == category))
        .map(|app| build_item(app, cache))
        .collect()
}

pub(super) fn wire(main_window: &MainWindow, ctx: &Ctx, mut db_state: DbState) {
    let Ctx { icon_cache, all_apps, .. } = ctx;
    let weak_window = main_window.as_weak();
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
                if let Ok(cats) = get_category_scores(&mut db_state.conn) {
                    window.set_categories(
                        Rc::new(VecModel::from(
                            cats.into_iter()
                                .map(|(name, _)| name.into())
                                .collect::<Vec<slint::SharedString>>(),
                        ))
                        .into(),
                    );
                }
                window.set_selected_category("".into());
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
                let items = filter_items(
                    &apps,
                    query.as_str(),
                    window.get_selected_category().as_str(),
                    &icon_cache,
                );
                window.set_apps(Rc::new(VecModel::from(items)).into());
            }
        }
    });
    let weak_category = main_window.as_weak();
    main_window.on_filter_category({
        let icon_cache = icon_cache.clone();
        let all_apps = all_apps.clone();
        move |category: slint::SharedString| {
            if let Some(window) = weak_category.upgrade() {
                let apps = all_apps.borrow();
                let items = filter_items(
                    &apps,
                    window.get_input_text().as_str(),
                    category.as_str(),
                    &icon_cache,
                );
                window.set_apps(Rc::new(VecModel::from(items)).into());
            }
        }
    });
    main_window.on_launch_app(|app| {
        let mut args = exec_args(&app.command);
        if needs_terminal(&app.path) {
            let terminal = std::env::var("TERMINAL").ok();
            let Some(wrapped) = terminal_argv(args, terminal.as_deref(), in_path) else {
                eprintln!("Failed to launch {}, no terminal found", app.command);
                return;
            };
            args = wrapped;
        }
        let Some((program, rest)) = args.split_first() else {
            return;
        };

        if let Err(err) = Command::new(program)
            .args(rest)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
        {
            eprintln!("Failed to launch {program}, {err}");
            return;
        }

        // Own connection (same pattern as watcher.rs); launches are rare.
        match open_conn() {
            Ok(mut conn) => {
                if let Err(err) = record_launch(&mut conn, &app.path) {
                    eprintln!("failed to record launch of {program}: {err}");
                }
            }
            Err(err) => eprintln!("failed to open db for launch record: {err}"),
        }
    });

    main_window.on_toggle_pin({
        let icon_cache = icon_cache.clone();
        let all_apps = all_apps.clone();
        let weak = main_window.as_weak();
        move |app: AppItem| {
            let pinned = match open_conn() {
                Ok(mut conn) => match set_pinned(&mut conn, app.path.as_str()) {
                    Ok(pinned) => pinned,
                    Err(err) => {
                        eprintln!("failed to toggle pin of {}: {err}", app.path);
                        return;
                    }
                },
                Err(err) => {
                    eprintln!("failed to open db for pin toggle: {err}");
                    return;
                }
            };

            // Keep the in-memory copy in sync and re-sort, same ordering
            // as get_data (pinned first, then score).
            {
                let mut apps = all_apps.borrow_mut();
                for a in apps.iter_mut() {
                    if a.path == app.path.as_str() {
                        a.pinned = pinned;
                    }
                }
                apps.sort_by(|a, b| {
                    b.pinned
                        .cmp(&a.pinned)
                        .then_with(|| b.score.cmp(&a.score))
                });
            }

            // Re-render the apps view with the current query/category.
            if let Some(window) = weak.upgrade() {
                let apps = all_apps.borrow();
                let items = filter_items(
                    &apps,
                    window.get_input_text().as_str(),
                    window.get_selected_category().as_str(),
                    &icon_cache,
                );
                window.set_apps(Rc::new(VecModel::from(items)).into());
            }
        }
    });
}
