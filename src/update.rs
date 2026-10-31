// `luma update`: reinstall the latest GitHub release via install.sh when it is newer.

use anyhow::{Context, Result};

const REPO: &str = "https://github.com/sakurartro/luma";

pub fn run() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    // /releases/latest redirects to /releases/tag/vX.Y.Z, same trick as install.sh.
    let resp = reqwest::blocking::get(format!("{REPO}/releases/latest"))
        .and_then(|r| r.error_for_status())
        .context("cannot reach GitHub to check the latest release")?;
    let latest = resp.url().path().rsplit('/').next().unwrap_or("");
    let latest = latest.strip_prefix('v').context("cannot resolve the latest release tag")?;
    if !is_newer(latest, current) {
        println!("Luma {current} is up to date (latest release: {latest}).");
        return Ok(());
    }
    println!("Updating Luma {current} -> {latest}");

    // Install next to the running binary, not the script's default ~/.local/bin.
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("binary has no parent dir")?;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#"curl -fsSL https://raw.githubusercontent.com/sakurartro/luma/main/install.sh | sh -s "$0""#)
        .arg(dir)
        .status()?;
    std::process::exit(status.code().unwrap_or(1));
}

// Numeric compare, so a local dev build ahead of the release is never downgraded.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> { v.split('.').filter_map(|p| p.parse().ok()).collect() };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
    }
}
