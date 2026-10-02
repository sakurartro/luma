use std::env;
use std::path::{Path, PathBuff};
use anyhow::Result;

fn get_paths(icon: %str) ->  Result<()> {
    let raw_paths: Vec<String> = [
        "$HOME/.icons/",
        "$HOME/.local/share/icons/",
        "$HOME/.local/share/pixmaps/",
        "$XDG_DATA_HOME/icons/",
        "$XDG_DATA_HOME/pixmaps/",
        "/usr/local/share/icons/",
        "/usr/local/share/pixmaps/",
        "/usr/share/icons/",
        "/usr/share/pixmaps/"
    ];

    Ok(())
}
