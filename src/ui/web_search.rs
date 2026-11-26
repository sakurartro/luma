use crate::{MainWindow, SettingField};
use reqwest::Url;
use slint::{ComponentHandle, Model, VecModel};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::rc::Rc;

// Stable settings ID, display name, search page, query parameter.
const SITES: &[(&str, &str, &str, &str)] = &[
    ("google", "Google", "https://www.google.com/search", "q"),
    ("duckduckgo", "DuckDuckGo", "https://duckduckgo.com/", "q"),
    ("brave", "Brave Search", "https://search.brave.com/search", "q"),
    ("wikipedia", "Wikipedia", "https://en.wikipedia.org/w/index.php", "search"),
    ("github", "GitHub", "https://github.com/search", "q"),
    ("stackoverflow", "Stack Overflow", "https://stackoverflow.com/search", "q"),
    ("reddit", "Reddit", "https://www.reddit.com/search/", "q"),
    ("youtube", "YouTube", "https://www.youtube.com/results", "search_query"),
    ("archwiki", "ArchWiki", "https://wiki.archlinux.org/index.php", "search"),
    ("mdn", "MDN", "https://developer.mozilla.org/en-US/search", "q"),
];

pub(super) fn apply(window: &MainWindow, enabled: &[String]) {
    let rows: Vec<_> = SITES.iter().map(|&(id, name, _, _)| SettingField {
        key: id.into(),
        label: name.into(),
        on: enabled.iter().any(|selected| selected == id),
        ..Default::default()
    }).collect();
    let selected: Vec<_> = rows.iter().filter(|site| site.on).cloned().collect();
    let settings = window.get_web_sites();
    if settings.row_count() == rows.len() {
        for (i, site) in rows.into_iter().enumerate() {
            if !settings.row_data(i).is_some_and(|current| current.on == site.on) {
                settings.set_row_data(i, site);
            }
        }
    } else {
        window.set_web_sites(Rc::new(VecModel::from(rows)).into());
    }
    window.set_enabled_web_sites(Rc::new(VecModel::from(selected)).into());
}

fn search_url(id: &str, query: &str) -> Option<Url> {
    if query.trim().is_empty() {
        return None;
    }
    let &(_, _, page, param) = SITES.iter().find(|site| site.0 == id)?;
    let mut url = Url::parse(page).ok()?;
    url.query_pairs_mut().append_pair(param, query);
    Some(url)
}

pub(super) fn wire(window: &MainWindow) {
    let weak = window.as_weak();
    window.on_open_web_search(move |site, query| {
        let Some(window) = weak.upgrade() else { return false };
        if window.get_mode() != 0
            || window.get_has_paste_thumb()
            || window.get_input_text() != query
            || !window.get_enabled_web_sites().iter().any(|row| row.key == site)
        {
            return false;
        }
        let Some(url) = search_url(site.as_str(), query.as_str()) else { return false };
        match Command::new("xdg-open").arg(url.as_str()).process_group(0).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || match child.wait() {
                    Ok(status) if status.success() => {}
                    Ok(status) => eprintln!("xdg-open exited with {status}"),
                    Err(err) => eprintln!("Failed to wait for xdg-open: {err}"),
                });
                true
            }
            Err(err) => {
                eprintln!("Failed to start xdg-open: {err}");
                false
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_urls_preserve_the_query_as_one_parameter() {
        let query = "тест C++ C# & language=ru";
        for &(id, _, page, param) in SITES {
            let url = search_url(id, query).unwrap();
            let base = Url::parse(page).unwrap();
            assert_eq!(url.scheme(), "https", "{id}");
            assert_eq!(url.host_str(), base.host_str(), "{id}");
            assert_eq!(url.path(), base.path(), "{id}");
            assert_eq!(url.fragment(), None, "{id}");
            assert_eq!(
                url.query_pairs().collect::<Vec<_>>(),
                [(param.into(), query.into())],
                "{id}"
            );
        }
        assert!(search_url("https://example.com/", query).is_none());
        assert!(search_url("google", "  ").is_none());
    }
}
