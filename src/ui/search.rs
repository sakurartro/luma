use crate::backend::apps::apps_search::Apps;
use crate::backend::apps::db_init::open_conn;
use crate::backend::apps::fuzzy;
use crate::backend::apps::watcher::APPS_CHANGED;
use crate::backend::files::clip;
use crate::backend::files::files_search;
use crate::backend::files::{docs, ocr};
use crate::backend::general_features::engine;
use crate::{AppItem, FileItem, MainWindow};
use slint::{ComponentHandle, Model, VecModel};
use std::rc::Rc;
use std::sync::atomic::Ordering;

use super::Ctx;
use super::apps::filter_items;
use super::files::{build_file_item, doc_file_items, extra_file_items, load_files_if_stale, set_files};

// Per-section cap for the unified (main view) search
const MAX_MIXED: usize = 20;
// Document hits get their own slots: name matches can fill all of MAX_MIXED
const MAX_MIXED_DOCS: usize = 5;

// Push file-index progress into the UI; called from the 30ms drain timer.
pub fn update_index_progress(window: &MainWindow) {
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
    update_model_progress(window);
}

// Model download / photo / video / document indexing banner (clip, ocr, then docs), same polling
// style as above.
fn update_model_progress(window: &MainWindow) {
    let dl_total = clip::DL_TOTAL.load(Ordering::Relaxed);
    let dl_done = clip::DL_DONE.load(Ordering::Relaxed);
    let total = clip::EMB_TOTAL.load(Ordering::Relaxed);
    let done = clip::EMB_DONE.load(Ordering::Relaxed);
    let vid_total = clip::VID_TOTAL.load(Ordering::Relaxed);
    let vid_done = clip::VID_DONE.load(Ordering::Relaxed);
    let ocr_dl_total = ocr::OCR_DL_TOTAL.load(Ordering::Relaxed);
    let ocr_dl_done = ocr::OCR_DL_DONE.load(Ordering::Relaxed);
    let ocr_total = ocr::OCR_TOTAL.load(Ordering::Relaxed);
    let ocr_done = ocr::OCR_DONE.load(Ordering::Relaxed);
    let docs_total = docs::DOCS_TOTAL.load(Ordering::Relaxed);
    let docs_done = docs::DOCS_DONE.load(Ordering::Relaxed);
    let (busy, label, percent) = if dl_total > 0 && dl_done < dl_total {
        (true, "Downloading models...", (dl_done.min(dl_total) * 100 / dl_total) as i32)
    } else if total > 0 && done < total {
        (true, "Indexing photos...", (done.min(total) * 100 / total) as i32)
    } else if vid_total > 0 && vid_done < vid_total {
        (true, "Indexing videos...", (vid_done.min(vid_total) * 100 / vid_total) as i32)
    } else if ocr_dl_total > 0 && ocr_dl_done < ocr_dl_total {
        (true, "Downloading OCR models...", (ocr_dl_done.min(ocr_dl_total) * 100 / ocr_dl_total) as i32)
    } else if ocr_total > 0 && ocr_done < ocr_total {
        (true, "Recognizing text...", (ocr_done.min(ocr_total) * 100 / ocr_total) as i32)
    } else if docs_total > 0 && docs_done < docs_total {
        (true, "Indexing docs...", (docs_done.min(docs_total) * 100 / docs_total) as i32)
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

pub(super) fn wire(main_window: &MainWindow, ctx: &Ctx) {
    let Ctx { icon_cache, all_apps, all_files, files_db_version, last_files_load, hide_patterns, currency } = ctx;
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
                // connection, same pattern as on_launch_app); reload after
                // the watcher picked up installed/removed apps.
                if APPS_CHANGED.swap(false, Ordering::Relaxed) || all_apps.borrow().is_empty() {
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
                load_files_if_stale(&all_files, &files_db_version, &last_files_load, weak.clone());
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
                let file_items: Vec<FileItem> = fuzzy_hits
                    .iter()
                    .filter(|file| {
                        !hide_patterns.borrow().iter().any(|p| file.name.starts_with(p.as_str()))
                    })
                    .take(MAX_MIXED)
                    .map(|f| build_file_item(f, clip::MATCH_NAME, 0))
                    .collect();
                if !query.is_empty() {
                    let fuzzy_paths: std::collections::HashSet<String> =
                        fuzzy_hits.iter().map(|f| f.path.clone()).collect();
                    // OCR/CLIP search off the UI thread, same as
                    // on_search_files: append to the shown fuzzy results.
                    let weak = weak.clone();
                    let query = query.clone();
                    // owned copy: the Rc can't cross into the worker thread
                    let hide_patterns = hide_patterns.borrow().clone();
                    let base_len = file_items.len();
                    let generation = clip::next_query();
                    window.set_searching(true);
                    std::thread::spawn(move || {
                        // After the name matches, before the semantic guesses.
                        let room = MAX_MIXED - base_len.min(MAX_MIXED);
                        let doc_items = doc_file_items(query.as_str(), &fuzzy_paths, &hide_patterns, MAX_MIXED_DOCS);
                        let extra_items = extra_file_items(
                            query.as_str(),
                            generation,
                            &fuzzy_paths,
                            &hide_patterns,
                            room,
                        );
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = weak.upgrade() {
                                // A newer keystroke already replaced the
                                // model through the sync path — discard.
                                // Same-text re-runs (refresh_view) are told
                                // apart by the generation: a stale worker may
                                // have skipped CLIP and must not overwrite.
                                // Mode 0 only: a files/photos tab owns the
                                // files model once switched to.
                                if window.get_input_text() == query
                                    && window.get_mode() == 0
                                    && !clip::is_stale(generation)
                                {
                                    // the shown fuzzy rows
                                    // document hits go on top
                                    let mut all: Vec<FileItem> =
                                        doc_items.into_iter().map(|(f, snippet, score)| {
                                            let mut item = build_file_item(&f, docs::MATCH_DOC, score);
                                            item.snippet = snippet.into();
                                            item
                                        }).collect();
                                    all.extend(window.get_files().iter().take(base_len));
                                    all.extend(extra_items.into_iter().map(|(f, kind, score, snippet)| {
                                        let mut item = build_file_item(&f, kind, score);
                                        item.snippet = snippet.into();
                                        item
                                    }));
                                    set_files(&window, all);
                                    window.set_searching(false);
                                }
                            }
                        });
                    });
                } else {
                    window.set_searching(false);
                }
                set_files(&window, file_items);
            }
        }
    });
    let weak_query = main_window.as_weak();
    let currency = currency.clone();
    main_window.on_run_query(move |text| {
        let weak = weak_query.clone();
        let text = text.to_string();
        let currency = currency.get();
        std::thread::spawn(move || {
            // Runs on every keystroke: failures (offline, unknown code)
            // go to stderr, the plate just stays empty.
            let msg = match engine::general_engine(text.clone(), currency) {
                Ok(out) => out,
                Err(err) => {
                    eprintln!("query error: {err}");
                    String::new()
                }
            };
            let _ = weak.upgrade_in_event_loop(move |window| {
                // A newer keystroke already started its own query — discard.
                if window.get_input_text() == text.as_str() {
                    window.set_query_result(msg.into());
                }
            });
        });
    });
}
