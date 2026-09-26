use anyhow::Ok;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use crate::db_service::insert_batch;
use crate::db_service::get_data;
use rusqlite::{Connection};

#[derive(Debug, Hash)]
pub struct App {
    pub app_name: String,
    pub path: String,
    pub command: String,
    pub icon_path: String,
    pub categories: Vec<String>,
}

pub struct Apps {
    pub apps: Vec<App>
}

impl Apps{

    fn find_apps_paths(dir: &str, extension: &str) -> Result<Vec<PathBuf>> {
        let mut apps = Vec::new();

        for entry_result in WalkDir::new(dir) {
            let entry = entry_result?;
            let path = entry.path();

            if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
                apps.push(path.to_path_buf());
            }
        }

        Ok(apps)
    }

    pub fn parse_config(conn: &mut Connection, paths: Vec<PathBuf>) -> Result<Self> {
        let mut apps: Vec<App> = Vec::new();
        for path in paths {
            let content = fs::read_to_string(&path)?;
            let desktop_entry: Vec<&str> = content
                .lines()
                .map(str::trim)
                .skip_while(|line| *line != "[Desktop Entry]")
                .skip(1)
                .take_while(|line| !line.starts_with('['))
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect();

            let is_application = desktop_entry.iter().any(|line| *line == "Type=Application");

            if !is_application {
                continue;
            }

            let name = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Name="));
            let command = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Exec="));

            let icon = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Icon="));

            let str_categories = desktop_entry
                .iter()
                .find_map(|line| line.strip_prefix("Categories="));

            let categories: Vec<String> = str_categories
                .unwrap_or("")
                .split(";")
                .map(String::from)
                .collect();

            let mut icon_path = String::new();

            if let Some(icon_name) = icon {
                let clean_icon = icon_name.trim().trim_matches('"');
                let path = Path::new(clean_icon);

                if path.is_absolute() {
                    if path.exists() {
                        icon_path.push_str(clean_icon);
                    }
                }
            } else {
                icon_path.push_str("/home/sakura/Downloads/ghost.svg")
            }

            if name.is_none() || command.is_none() {
                continue;
            } else {
                let icon_path = String::new();
                let app = App {
                    app_name: name.unwrap_or("").to_owned(),
                    path: path.to_string_lossy().into_owned(),
                    command: command.unwrap_or("").to_owned(),
                    icon_path: icon_path,
                    categories: categories,
                };
                apps.push(app);
            }
        }

        for chunk in apps.chunks(100) {
            let _ = insert_batch(chunk, conn)?;
        }

        Ok(Self { apps })
    }

    pub fn refresh(conn: &mut Connection) -> Result<Self> {
        let apps = get_data(conn)?;
        Ok(Self { apps })

    }

    pub fn find_apps(conn: &mut Connection) -> Result<Self> {
        let paths = Self::find_apps_paths("/usr/share/applications", "desktop")?;
        Self::parse_config(conn, paths)
    }

}
