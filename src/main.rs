slint::include_modules!();
mod db_init;
mod db_service;
mod watcher;
use watcher::start_watcher;
use apps_search::Apps;
use db_init::DbState;
use slint::VecModel;
use std::rc::Rc;
mod apps_search;
use std::path::Path;
use std::process::Command;
use anyhow::Result;

fn main() -> Result<()> {
    start_watcher();
    let mut db_state = DbState::init()?;
    let apps_obj = Apps::find_apps(&mut db_state.conn)?;
    let apps = apps_obj.apps;
    let main_window = MainWindow::new()?;
    let weak_window = main_window.as_weak();
    slint::set_xdg_app_id("luma-rust")?;
    let slint_apps: Vec<AppItem> = apps
        .into_iter()
        .map(|app| {
            let icon = slint::Image::load_from_path(
                Path::new(&app.icon_path)
            ).unwrap_or_default();
            AppItem{
                name: app.app_name.into(),
                path: app.path.into(),
                command: app.command.into(),
                icon,
            }
        })
        .collect();

    let model = Rc::new(VecModel::from(slint_apps));
    main_window.on_show_apps({
        // let model = model.clone();
        move || {
        if let Some(window) = weak_window.upgrade(){
            let apps_refresh_obj = match Apps::refresh(&mut db_state.conn) {
                Ok(apps) => apps,
                Err(err) => {
                    eprintln!("refresh error, {}", err);
                    return;
                }
            };
            let refreshed_apps = apps_refresh_obj.apps;
            let slint_apps: Vec<AppItem> = refreshed_apps
                .into_iter()
                .map(|app| {
                    let icon = slint::Image::load_from_path(
                        Path::new(&app.icon_path)
                    ).unwrap_or_default();
                    AppItem{
                        name: app.app_name.into(),
                        path: app.path.into(),
                        command: app.command.into(),
                        icon,
                    }
                })
                .collect();

            let model = Rc::new(VecModel::from(slint_apps));

            window.set_apps(model.into());

        }
        }
    });
    main_window.on_launch_app( |command| {
        let mut parts = command.split_whitespace();

        let Some(program) = parts.next() else {
            return;
        };

        if let Err(err) = Command::new(program)
            .args(parts)
            .spawn()
        {
            eprintln!("Failed to launch {program}, {err}");
        }
    });
    main_window.run()?;
    Ok(())
}
