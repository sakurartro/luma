use hyprland::data::Clients;
use hyprland::shared::HyprData;
use std::path::{Path, PathBuf};

// One suggested action: run `command` with cwd `dir`, `label` is shown in the UI.
pub struct ContextAction {
    pub label: String,
    pub command: String,
    pub dir: PathBuf,
}

// marker file/dir -> actions runnable in the directory containing it
// shortcut: hardcoded rules, user-defined actions.json comes later
const RULES: &[(&str, &[(&str, &str)])] = &[(
    "Cargo.toml",
    &[
        ("Cargo build", "cargo build"),
        ("Cargo check", "cargo check"),
        ("Cargo test", "cargo test"),
        ("Cargo fmt check", "cargo fmt --check"),
        ("Cargo clippy", "cargo clippy"),
    ],
)];

// git pull/push need the branch: plain `git push` fails without upstream.
// Branch from .git/HEAD (plain repos); empty on detached HEAD or worktrees
// (.git is a file there) — fall back to upstream-less commands.
fn git_actions(dir: &Path) -> Vec<ContextAction> {
    let head = std::fs::read_to_string(dir.join(".git/HEAD")).unwrap_or_default();
    let branch = head
        .trim()
        .strip_prefix("ref: refs/heads/")
        .unwrap_or_default()
        .to_string();
    let action = |label: &str, command: &str| ContextAction {
        label: label.to_string(),
        command: command.to_string(),
        dir: dir.to_path_buf(),
    };
    if branch.is_empty() {
        return vec![
            action("Git status", "git status"),
            action("Git pull", "git pull"),
            action("Git push", "git push"),
        ];
    }
    vec![
        action("Git status", "git status"),
        action(&format!("Git pull ({branch})"), &format!("git pull origin {branch}")),
        action(&format!("Git push ({branch})"), &format!("git push origin {branch}")),
    ]
}

// The window focused before the launcher: focus_history_id 1 is the previous
// window (0 is the launcher itself).
fn previous_window_pid() -> Option<i32> {
    let clients = Clients::get().ok()?;
    clients
        .iter()
        .filter(|c| c.pid > 0 && c.class != "luma-rust" && c.focus_history_id > 0)
        .min_by_key(|c| c.focus_history_id)
        .map(|c| c.pid)
}

// Terminal windows sit at $HOME; the shell child carries the real cwd.
fn shell_child_cwd(pid: i32) -> Option<PathBuf> {
    let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
    let shells = ["bash", "zsh", "fish", "sh", "nu", "pwsh"];
    for child in children.split_whitespace() {
        let comm = std::fs::read_to_string(format!("/proc/{child}/comm")).unwrap_or_default();
        let name = comm.trim();
        if shells.contains(&name) {
            return std::fs::read_link(format!("/proc/{child}/cwd")).ok();
        }
    }
    None
}

pub fn cwd_of_previous_window() -> Option<PathBuf> {
    let pid = previous_window_pid()?;
    shell_child_cwd(pid).or_else(|| std::fs::read_link(format!("/proc/{pid}/cwd")).ok())
}

// Walk up from cwd matching markers; nearest-to-cwd wins, one hit per marker.
pub fn detect(cwd: &Path) -> Vec<ContextAction> {
    let mut actions = Vec::new();
    let mut done_markers: Vec<&str> = Vec::new();
    let mut dir = Some(cwd);
    for _ in 0..6 {
        let Some(current) = dir else { break };
        for (marker, rules) in RULES {
            if done_markers.contains(marker) || !current.join(marker).exists() {
                continue;
            }
            done_markers.push(marker);
            for (label, command) in *rules {
                actions.push(ContextAction {
                    label: label.to_string(),
                    command: command.to_string(),
                    dir: current.to_path_buf(),
                });
            }
        }
        if !done_markers.contains(&".git") && current.join(".git").exists() {
            done_markers.push(".git");
            actions.extend(git_actions(current));
        }
        dir = current.parent();
    }
    actions
}

// Run an action and return the exit status plus the tail of its output.
pub fn run(action: &ContextAction) -> Result<String, anyhow::Error> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&action.command)
        .current_dir(&action.dir)
        .output()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let tail: Vec<&str> = text.lines().rev().take(3).collect::<Vec<_>>();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    Ok(format!("{} (exit {})", tail, out.status.code().unwrap_or(-1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_finds_nearest_markers() {
        let root = std::env::temp_dir().join(format!("luma_ctx_test_{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let actions = detect(&root.join("sub/deep"));
        let dirs: Vec<&Path> = actions.iter().map(|a| a.dir.as_path()).collect();
        assert!(dirs.iter().all(|d| *d == root.as_path()));
        assert_eq!(actions.len(), 8);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detect_no_markers_is_empty() {
        let dir = std::env::temp_dir();
        let actions = detect(&dir);
        assert!(actions.iter().all(|a| a.dir != dir));
    }

    #[test]
    #[ignore] // needs a live Hyprland session
    fn debug_cwd_of_previous_window() {
        println!("pid: {:?}", previous_window_pid());
        println!("cwd: {:?}", cwd_of_previous_window());
        if let Some(cwd) = cwd_of_previous_window() {
            for a in detect(&cwd) {
                println!("{} | {} | {}", a.label, a.command, a.dir.display());
            }
        }
    }
}
