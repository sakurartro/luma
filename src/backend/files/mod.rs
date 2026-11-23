pub mod clip;
pub mod db_service;
pub mod docs;
pub mod download;
pub mod files_search;
pub mod ocr;
pub mod thumbs;
pub mod video;
pub mod watcher;

// ort allows one Environment per process; clip and ocr share it.
pub(crate) fn ort_env() -> Option<ort::environment::Environment> {
    static ENV: std::sync::OnceLock<Option<ort::environment::Environment>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| init_ort().map_err(|e| eprintln!("ort init failed: {e:#}")).ok()).clone()
}

// ORT_DYLIB_PATH lets a user point at their own libonnxruntime; otherwise the
// pinned build is downloaded once next to the models. init_from returns an
// error on a bad path where plain init() would panic.
fn init_ort() -> anyhow::Result<ort::environment::Environment> {
    let lib = match std::env::var_os("ORT_DYLIB_PATH").filter(|p| !p.is_empty()) {
        Some(path) => path.into(),
        None => download::ensure_ort_lib(&crate::backend::apps::db_init::config_dir().join("models/ort"))
            .ok_or_else(|| anyhow::anyhow!("onnxruntime library unavailable (download failed, retried on next start)"))?,
    };
    Ok(ort::init_from(lib)?.build()?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ort_env_is_shared() {
        assert!(super::ort_env().is_some());
        assert!(super::ort_env().is_some());
    }
}
