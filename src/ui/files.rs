use crate::backend;
use crate::backend::apps::db_init::open_conn;
use crate::backend::apps::fuzzy;
use crate::backend::files::clip;
use crate::backend::files::db_service as files_db;
use crate::backend::files::files_search::{self, FileEntry};
use crate::backend::files::ocr;
use crate::{FileItem, MainWindow};
use slint::{ComponentHandle, Model, VecModel};
use std::cell::{Cell, RefCell};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::Ctx;

// Finished background files reload, picked up by load_files_if_stale on the
// UI thread (same handoff pattern as INIT_DB; no cross-thread slint invokes).
static FILES_RELOAD: Mutex<Option<(usize, Vec<FileEntry>)>> = Mutex::new(None);
// Coalescing: one background reload at a time; extra presses reuse the
// in-memory copy instead of stacking reader threads.
static FILES_RELOAD_RUNNING: AtomicBool = AtomicBool::new(false);

// Thumbnail jobs (row index, path) of the current files model, popped from the
// back, + whether the single worker thread is running. Both under one lock so
// a new queue can't slip in between "queue empty" and the worker exiting.
static THUMB_JOBS: Mutex<(Vec<(usize, String)>, bool)> = Mutex::new((Vec::new(), false));

// Render cap: thousands of slint cards kill layout/memory; results are
// ranked, the tail is what nobody scrolls to
const MAX_RENDERED: usize = 200;

// Reload the in-memory file index when the watcher bumped DB_VERSION; keeps
// per-call cost off repeated searches. The heavy read+sort of ~200k rows runs
// in a background thread (a synchronous reload here froze the UI for seconds
// per Alt+2 press), result is applied through FILES_RELOAD by the
// refresh_view the thread triggers when done (otherwise the tab stays empty
// until the next call). Reloads are throttled and coalesced: the files watcher bumps
// DB_VERSION on every fs event, so presses come in faster than reloads.
const FILES_RELOAD_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

pub(super) fn load_files_if_stale(
    all_files: &RefCell<Vec<FileEntry>>,
    files_db_version: &Cell<usize>,
    last_load: &Cell<Option<std::time::Instant>>,
    weak: slint::Weak<MainWindow>,
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
            Ok(entries) => {
                *FILES_RELOAD.lock().unwrap() = Some((version, entries));
                // Re-run the current view: it picks the result up and redraws.
                let _ = weak.upgrade_in_event_loop(|window| window.invoke_refresh_view());
            }
            Err(err) => eprintln!("files refresh error, {err}"),
        }
        FILES_RELOAD_RUNNING.store(false, Ordering::Relaxed);
    });
}

pub(super) fn build_file_item(file: &FileEntry, match_kind: i32, match_score: i32) -> FileItem {
    FileItem {
        name: file.name.as_str().into(),
        path: file.path.as_str().into(),
        is_dir: file.is_dir,
        kind: files_search::file_kind(&file.path, file.is_dir),
        thumb: slint::Image::default(),
        match_kind,
        match_score,
    }
}

// Every files model goes through here. In the files grid (modes 2 and 4) it
// replaces the thumbnail queue with this model's images; elsewhere it just
// drops the old queue.
pub(super) fn set_files(window: &MainWindow, items: Vec<FileItem>) {
    let grid = matches!(window.get_mode(), 2 | 4);
    let mut jobs: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .filter(|(_, f)| grid && !f.is_dir && files_search::is_image(&f.path))
        // rows carried over into a new model may already have one
        .filter(|(_, f)| f.thumb.size().width == 0)
        .map(|(i, f)| (i, f.path.to_string()))
        .collect();
    jobs.reverse(); // popped from the back: top of the grid first
    window.set_files(Rc::new(VecModel::from(items)).into());

    let mut queue = THUMB_JOBS.lock().unwrap();
    queue.0 = jobs;
    if queue.0.is_empty() || queue.1 {
        return;
    }
    queue.1 = true;
    let weak = window.as_weak();
    // One worker at a time (low-RAM machines): decodes one image, hands the
    // pixels to the UI thread, repeats until the queue runs dry.
    std::thread::spawn(move || {
        let dir = backend::files::thumbs::cache_dir();
        loop {
            let Some((index, path)) = ({
                let mut queue = THUMB_JOBS.lock().unwrap();
                let job = queue.0.pop();
                queue.1 = job.is_some();
                job
            }) else {
                return;
            };
            let Some(out) = backend::files::thumbs::thumbnail(Path::new(&path), &dir) else { continue };
            let Ok(img) = image::open(&out) else { continue };
            let rgba = img.into_rgba8();
            let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
            );
            let _ = weak.upgrade_in_event_loop(move |window| {
                // The model may have been replaced since the job was queued.
                let files = window.get_files();
                if let Some(mut item) = files.row_data(index)
                    && item.path == path.as_str()
                {
                    item.thumb = slint::Image::from_rgba8(pixels);
                    files.set_row_data(index, item);
                }
            });
        }
    });
}

// Files found by the text on them (OCR) and by meaning (CLIP), in that order:
// text on the image is the stronger signal. Paths already shown by the fuzzy
// name search, hidden names and repeats between the two are skipped.
// Runs on worker threads, so it returns (entry, match kind, score) and the UI
// thread builds the FileItems (slint::Image in them isn't Send).
pub(super) fn extra_file_items(
    query: &str,
    shown: &std::collections::HashSet<String>,
    hide_patterns: &[String],
    limit: usize,
) -> Vec<(FileEntry, i32, i32)> {
    let ocr_hits = ocr::ocr_search(query).into_iter().map(|(f, score)| (f, ocr::MATCH_OCR, score));
    let clip_hits =
        clip::clip_search(query).into_iter().map(|(f, score)| (f, clip::MATCH_SEMANTIC, score));
    let mut taken = std::collections::HashSet::new();
    ocr_hits
        .chain(clip_hits)
        .filter(|(f, _, _)| {
            !shown.contains(f.path.as_str())
                && !hide_patterns.iter().any(|p| f.name.starts_with(p.as_str()))
                && taken.insert(f.path.clone())
        })
        .take(limit)
        .collect()
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

// Image files only, for the Photos view.
fn is_photo(file: &FileEntry) -> bool {
    !file.is_dir && files_search::is_image(&file.path)
}

// Filtered + sorted files tab content. Sort modes: 0 = name (the order
// get_files returns), 1 = newest first, 2 = oldest first.
fn files_view(files: &[FileEntry], hide_patterns: &[String], sort: i32, photos: bool) -> Vec<FileItem> {
    let mut filtered: Vec<&FileEntry> = files
        .iter()
        .filter(|f| !hide_patterns.iter().any(|p| f.name.starts_with(p.as_str())))
        .filter(|f| !photos || is_photo(f))
        .collect();
    sort_entries(&mut filtered, sort);
    filtered
        .into_iter()
        .take(MAX_RENDERED)
        .map(|f| build_file_item(f, clip::MATCH_NAME, 0))
        .collect()
}

pub(super) fn wire(main_window: &MainWindow, ctx: &Ctx) {
    let Ctx { all_files, files_db_version, last_files_load, hide_patterns, .. } = ctx;
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
                load_files_if_stale(&all_files, &files_db_version, &last_files_load, weak.clone());
                // mode 4 (Photos) shares the files tab, images only
                set_files(
                    &window,
                    files_view(
                        &all_files.borrow(),
                        &hide_patterns.borrow(),
                        window.get_files_sort(),
                        window.get_mode() == 4,
                    ),
                );
            }
        }
    });
    main_window.on_search_files({
        let all_files = all_files.clone();
        let files_db_version = files_db_version.clone();
        let last_files_load = last_files_load.clone();
        let weak = main_window.as_weak();
        move |query: slint::SharedString| {
            if let Some(window) = weak.upgrade() {
                // refresh_view lands here after a background reload in
                // mode 2 with text — pick the finished result up.
                load_files_if_stale(&all_files, &files_db_version, &last_files_load, weak.clone());
                let files = all_files.borrow();
                let mut hits =
                    fuzzy::fuzzy_search(&files, |file| file.name_lower.as_slice(), query.as_str());
                // mode 4 (Photos) shares the files tab, images only
                let mode = window.get_mode();
                if mode == 4 {
                    hits.retain(|f| is_photo(f));
                }
                sort_entries(&mut hits, window.get_files_sort());
                let items: Vec<FileItem> = hits
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
                    // run on the UI thread: append OCR/CLIP hits to the
                    // already shown fuzzy results from a worker thread.
                    let weak = weak.clone();
                    let query = query.clone();
                    let base_len = items.len();
                    std::thread::spawn(move || {
                        let extra_items = extra_file_items(
                            query.as_str(),
                            &fuzzy_paths,
                            &[],
                            MAX_RENDERED - base_len.min(MAX_RENDERED),
                        );
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = weak.upgrade() {
                                // A newer keystroke or a files/photos
                                // switch already replaced the model — discard.
                                if window.get_input_text() == query && window.get_mode() == mode {
                                    // the shown fuzzy rows, with any thumbnails they got
                                    let mut all: Vec<FileItem> =
                                        window.get_files().iter().take(base_len).collect();
                                    all.extend(extra_items.into_iter().map(|(f, kind, score)| {
                                        build_file_item(&f, kind, score)
                                    }));
                                    set_files(&window, all);
                                }
                            }
                        });
                    });
                }
                set_files(&window, items);
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
}
