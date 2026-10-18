slint::include_modules!();
mod backend;
mod toggle;
use anyhow::Result;
use backend::apps::apps_search::{App, Apps};
use backend::apps::db_init::{DbState, open_conn};
use backend::apps::db_service::{get_category_scores, record_launch, set_pinned};
use backend::apps::fuzzy;
use backend::apps::watcher::start_watcher;
use backend::files::clip;
use backend::files::db_service as files_db;
use backend::files::files_search::{self, FileEntry};
use backend::files::watcher::start_files_watcher;
use backend::general_features::engine;
use backend::settings;
use slint::VecModel;
use slint::winit_030::WinitWindowAccessor;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

// Result of the background init, picked up by the UI-thread drain timer.
// (No cross-thread invoke: slint can silently drop those around loop start.)
static INIT_DB: Mutex<Option<DbState>> = Mutex::new(None);

// Toggle requests from the keybind socket; drained by a timer on the UI thread.
static TOGGLES: AtomicUsize = AtomicUsize::new(0);

// Finished background files reload, picked up by load_files_if_stale on the
// UI thread (same handoff pattern as INIT_DB; no cross-thread slint invokes).
static FILES_RELOAD: Mutex<Option<(usize, Vec<FileEntry>)>> = Mutex::new(None);
// Coalescing: one background reload at a time; extra presses reuse the
// in-memory copy instead of stacking reader threads.
static FILES_RELOAD_RUNNING: AtomicBool = AtomicBool::new(false);

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

// Render cap: thousands of slint cards kill layout/memory; results are
// ranked, the tail is what nobody scrolls to
const MAX_RENDERED: usize = 200;

// Per-section cap for the unified (main view) search
const MAX_MIXED: usize = 20;

// Reload the in-memory file index when the watcher bumped DB_VERSION; keeps
// per-call cost off repeated searches. The heavy read+sort of ~200k rows runs
// in a background thread (a synchronous reload here froze the UI for seconds
// per Alt+2 press), result is applied on the next UI-thread call through
// FILES_RELOAD. Reloads are throttled and coalesced: the files watcher bumps
// DB_VERSION on every fs event, so presses come in faster than reloads.
const FILES_RELOAD_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

fn load_files_if_stale(
    all_files: &RefCell<Vec<FileEntry>>,
    files_db_version: &Cell<usize>,
    last_load: &Cell<Option<std::time::Instant>>,
) {
    // Apply a finished background reload.
    if let Some((version, entries)) = FILES_RELOAD.lock().unwrap().take() {
        *all_files.borrow_mut() = entries;
        files_db_version.set(version);
    }
    let version = files_search::DB_VERSION.load(Ordering::Relaxed);
    if version == files_db_version.get() && !all_files.borrow().is_empty() {
        return;
    }
    if last_load.get().is_some_and(|t| t.elapsed() < FILES_RELOAD_MIN_INTERVAL) {
        return;
    }
    if FILES_RELOAD_RUNNING.swap(true, Ordering::Relaxed) {
        return;
    }
    last_load.set(Some(std::time::Instant::now()));
    // Own connection (same pattern as on_launch_app); reloads are rare.
    std::thread::spawn(move || {
        match open_conn().and_then(|conn| files_db::get_files(&conn).map_err(anyhow::Error::from)) {
            Ok(entries) => *FILES_RELOAD.lock().unwrap() = Some((version, entries)),
            Err(err) => eprintln!("files refresh error, {err}"),
        }
        FILES_RELOAD_RUNNING.store(false, Ordering::Relaxed);
    });
}

fn build_file_item(file: &FileEntry, match_kind: i32, match_score: i32) -> FileItem {
    FileItem {
        name: file.name.as_str().into(),
        path: file.path.as_str().into(),
        is_dir: file.is_dir,
        match_kind,
        match_score,
    }
}

// Folders stay in front in every sort mode; ties broken by name.
// Sort modes: 1 = newest first, 2 = oldest first, 0 = untouched.
fn sort_entries(entries: &mut [&FileEntry], sort: i32) {
    match sort {
        1 => entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| b.mtime.cmp(&a.mtime))
                .then_with(|| b.name_lower.cmp(&a.name_lower))
        }),
        2 => entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.mtime.cmp(&b.mtime))
                .then_with(|| a.name_lower.cmp(&b.name_lower))
        }),
        _ => {}
    }
}

// Filtered + sorted files tab content. Sort modes: 0 = name (the order
// get_files returns), 1 = newest first, 2 = oldest first.
fn files_view(files: &[FileEntry], hide_patterns: &[String], sort: i32) -> Vec<FileItem> {
    let mut filtered: Vec<&FileEntry> = files
        .iter()
        .filter(|f| !hide_patterns.iter().any(|p| f.name.starts_with(p.as_str())))
        .collect();
    sort_entries(&mut filtered, sort);
    filtered
        .into_iter()
        .take(MAX_RENDERED)
        .map(|f| build_file_item(f, clip::MATCH_NAME, 0))
        .collect()
}

// Push file-index progress into the UI; called from the 30ms drain timer.
fn update_index_progress(window: &MainWindow) {
    let done = files_search::INDEX_DONE.load(Ordering::Relaxed);
    if window.get_indexing() != !done {
        window.set_indexing(!done);
    }
    let percent = if done {
        100
    } else {
        let total = files_search::INDEX_TOTAL.load(Ordering::Relaxed);
        let indexed = files_search::INDEXED.load(Ordering::Relaxed);
        (indexed.min(total) * 100 / total.max(1)) as i32
    };
    if window.get_index_percent() != percent {
        window.set_index_percent(percent);
    }
    update_clip_progress(window);
}

// Model download / photo indexing banner, same polling style as above.
fn update_clip_progress(window: &MainWindow) {
    let dl_total = clip::DL_TOTAL.load(Ordering::Relaxed);
    let dl_done = clip::DL_DONE.load(Ordering::Relaxed);
    let total = clip::EMB_TOTAL.load(Ordering::Relaxed);
    let done = clip::EMB_DONE.load(Ordering::Relaxed);
    let (busy, label, percent) = if dl_total > 0 && dl_done < dl_total {
        (true, "Downloading models...", (dl_done.min(dl_total) * 100 / dl_total) as i32)
    } else if total > 0 && done < total {
        (true, "Indexing photos...", (done.min(total) * 100 / total) as i32)
    } else {
        (false, "", 0)
    };
    if window.get_clip_busy() != busy {
        window.set_clip_busy(busy);
    }
    if window.get_clip_percent() != percent {
        window.set_clip_percent(percent);
    }
    if window.get_clip_label() != label {
        window.set_clip_label(label.into());
    }
}

// fuzzy name match ∩ selected category; empty category = all
fn filter_items(
    apps: &[App],
    query: &str,
    category: &str,
    cache: &RefCell<HashMap<String, slint::Image>>,
) -> Vec<AppItem> {
    fuzzy::fuzzy_search(apps, |app| app.name_lower.as_slice(), query)
        .into_iter()
        .filter(|app| category.is_empty() || app.categories.iter().any(|cat| cat == category))
        .map(|app| build_item(app, cache))
        .collect()
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

    // On failure db_state comes back so the next toggle can retry the whole
    // init instead of dropping the connection for good.
    fn create_ui(mut db_state: DbState) -> Result<UiState, (anyhow::Error, DbState)> {
        let main_window = match MainWindow::new() {
            Ok(window) => window,
            Err(err) => return Err((anyhow::Error::from(err), db_state)),
        };
        if let Err(err) = slint::set_xdg_app_id("luma-rust") {
            return Err((anyhow::Error::from(err), db_state));
        }

        let st = settings::load();
        let hide_patterns = st.hide_patterns.clone();
        main_window.set_bg(settings::parse_color(&st.bg));
        main_window.set_surface(settings::parse_color(&st.surface));
        main_window.set_hover(settings::parse_color(&st.hover));
        main_window.set_selected(settings::parse_color(&st.selected));
        main_window.set_border(settings::parse_color(&st.border));
        main_window.set_accent(settings::parse_color(&st.accent));
        main_window.set_text(settings::parse_color(&st.text));
        main_window.set_text_dim(settings::parse_color(&st.text_dim));

        let weak_window = main_window.as_weak();
        let icon_cache = Rc::new(RefCell::new(HashMap::new()));
        let all_apps: Rc<RefCell<Vec<App>>> = Rc::new(RefCell::new(Vec::new()));
        let all_files: Rc<RefCell<Vec<FileEntry>>> = Rc::new(RefCell::new(Vec::new()));
        // DB_VERSION at the time all_files was loaded; 0 entries force a load
        let files_db_version: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        // Throttle clock for the stale reload above
        let last_files_load: Rc<Cell<Option<std::time::Instant>>> = Rc::new(Cell::new(None));

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
        main_window.on_show_files({
            let all_files = all_files.clone();
            let files_db_version = files_db_version.clone();
            let last_files_load = last_files_load.clone();
            let hide_patterns = hide_patterns.clone();
            let weak = main_window.as_weak();
            move || {
                if let Some(window) = weak.upgrade() {
                    // Reload from the db only when it actually changed since the
                    // last load (watcher bumps DB_VERSION); otherwise the
                    // in-memory copy is reused — reading all 180k rows per tab
                    // open froze the UI for up to a second.
                    load_files_if_stale(&all_files, &files_db_version, &last_files_load);
                    window.set_files(
                        Rc::new(VecModel::from(files_view(
                            &all_files.borrow(),
                            &hide_patterns,
                            window.get_files_sort(),
                        )))
                        .into(),
                    );
                }
            }
        });
        main_window.on_search_files({
            let all_files = all_files.clone();
            let weak = main_window.as_weak();
            move |query: slint::SharedString| {
                if let Some(window) = weak.upgrade() {
                    let files = all_files.borrow();
                    let mut hits =
                        fuzzy::fuzzy_search(&files, |file| file.name_lower.as_slice(), query.as_str());
                    sort_entries(&mut hits, window.get_files_sort());
                    let mut items: Vec<FileItem> = hits
                        .iter()
                        .take(MAX_RENDERED)
                        .map(|f| build_file_item(f, clip::MATCH_NAME, 0))
                        .collect();
                    // Semantic hits only in relevance order; mtime sorts would
                    // jumble them (same limitation as fuzzy ranking there).
                    if window.get_files_sort() == 0 && !query.is_empty() {
                        let fuzzy_paths: std::collections::HashSet<String> =
                            hits.iter().map(|f| f.path.clone()).collect();
                        // CLIP inference (~50-300ms per keystroke) must not
                        // run on the UI thread: append hits to the already
                        // shown fuzzy results from a worker thread.
                        let weak = weak.clone();
                        let query = query.clone();
                        let items = items.clone();
                        let base_len = items.len();
                        std::thread::spawn(move || {
                            let clip_items: Vec<FileItem> = clip::clip_search(query.as_str())
                                .into_iter()
                                .filter(|(f, _)| !fuzzy_paths.contains(f.path.as_str()))
                                .take(MAX_RENDERED - base_len.min(MAX_RENDERED))
                                .map(|(f, score)| build_file_item(&f, clip::MATCH_SEMANTIC, score))
                                .collect();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = weak.upgrade() {
                                    // A newer keystroke already replaced the
                                    // model through the sync path — discard.
                                    if window.get_input_text() == query {
                                        let mut all = items;
                                        all.extend(clip_items);
                                        window.set_files(Rc::new(VecModel::from(all)).into());
                                    }
                                }
                            });
                        });
                    }
                    window.set_files(Rc::new(VecModel::from(items)).into());
                }
            }
        });
        main_window.on_search_all({
            let icon_cache = icon_cache.clone();
            let all_apps = all_apps.clone();
            let all_files = all_files.clone();
            let files_db_version = files_db_version.clone();
            let last_files_load = last_files_load.clone();
            let hide_patterns = hide_patterns.clone();
            let weak = main_window.as_weak();
            move |query: slint::SharedString| {
                if let Some(window) = weak.upgrade() {
                    // First unified search lazy-loads the app list (own
                    // connection, same pattern as on_launch_app).
                    if all_apps.borrow().is_empty() {
                        match open_conn() {
                            Ok(mut conn) => match Apps::refresh(&mut conn) {
                                Ok(apps_refresh_obj) => {
                                    *all_apps.borrow_mut() = apps_refresh_obj.apps;
                                }
                                Err(err) => eprintln!("apps refresh error, {err}"),
                            },
                            Err(err) => eprintln!("failed to open db for apps refresh: {err}"),
                        }
                    }
                    load_files_if_stale(&all_files, &files_db_version, &last_files_load);
                    let apps = all_apps.borrow();
                    let app_items = filter_items(&apps, query.as_str(), "", &icon_cache)
                        .into_iter()
                        .take(MAX_MIXED)
                        .collect::<Vec<AppItem>>();
                    window.set_apps(Rc::new(VecModel::from(app_items)).into());
                    let files = all_files.borrow();
                    let fuzzy_hits = fuzzy::fuzzy_search(
                        &files,
                        |file| file.name_lower.as_slice(),
                        query.as_str(),
                    );
                    let mut file_items: Vec<FileItem> = fuzzy_hits
                        .iter()
                        .filter(|file| {
                            !hide_patterns.iter().any(|p| file.name.starts_with(p.as_str()))
                        })
                        .take(MAX_MIXED)
                        .map(|f| build_file_item(f, clip::MATCH_NAME, 0))
                        .collect();
                    if !query.is_empty() {
                        let fuzzy_paths: std::collections::HashSet<String> =
                            fuzzy_hits.iter().map(|f| f.path.clone()).collect();
                        // CLIP inference off the UI thread, same as
                        // on_search_files: append to the shown fuzzy results.
                        let weak = weak.clone();
                        let query = query.clone();
                        let hide_patterns = hide_patterns.clone();
                        let file_items = file_items.clone();
                        let base_len = file_items.len();
                        std::thread::spawn(move || {
                            let clip_items: Vec<FileItem> = clip::clip_search(query.as_str())
                                .into_iter()
                                .filter(|(f, _)| !fuzzy_paths.contains(f.path.as_str()))
                                .filter(|(f, _)| {
                                    !hide_patterns.iter().any(|p| f.name.starts_with(p.as_str()))
                                })
                                .take(MAX_MIXED - base_len.min(MAX_MIXED))
                                .map(|(f, score)| build_file_item(&f, clip::MATCH_SEMANTIC, score))
                                .collect();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = weak.upgrade() {
                                    // A newer keystroke already replaced the
                                    // model through the sync path — discard.
                                    if window.get_input_text() == query {
                                        let mut all = file_items;
                                        all.extend(clip_items);
                                        window.set_files(Rc::new(VecModel::from(all)).into());
                                    }
                                }
                            });
                        });
                    }
                    window.set_files(Rc::new(VecModel::from(file_items)).into());
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
        main_window.on_launch_app(|app| {
            let mut parts = app.command.split_whitespace();

            let Some(program) = parts.next() else {
                return;
            };

            if let Err(err) = Command::new(program)
                .args(parts)
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

        main_window.on_open_file(|file| {
            if let Err(err) = Command::new("xdg-open")
                .arg(file.path.as_str())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
            {
                eprintln!("Failed to open {}, {err}", file.path);
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
        update_index_progress(&main_window);
        Ok(UiState::Live {
            window: main_window,
            visible,
        })
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
                        match create_ui(db_state) {
                            Ok(live) => {
                                *s = live;
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
                update_index_progress(window);
                if TOGGLES.swap(0, Ordering::Relaxed) > 0 {
                    if visible.get() {
                        hide_window(window, visible);
                    } else {
                        show_window(window, visible);
                    }
                }
            }
            }
        });
    }

    // Heavy init in the background so the event loop (and the toggle socket)
    // is up in milliseconds; the drain timer picks up the result. Apps scan
    // first, then the state is handed to the UI — file indexing keeps going
    // in the background with progress reported via files_search::INDEX_*.
    std::thread::spawn(|| {
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
                let st = settings::load();
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
