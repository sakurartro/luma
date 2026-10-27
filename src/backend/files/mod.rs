pub mod clip;
pub mod db_service;
pub mod download;
pub mod files_search;
pub mod ocr;
pub mod thumbs;
pub mod watcher;

// ort allows one Environment per process; clip and ocr share it.
pub(crate) fn ort_env() -> Option<ort::environment::Environment> {
    static ENV: std::sync::OnceLock<Option<ort::environment::Environment>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| ort::environment::init().build().map_err(|e| eprintln!("ort init failed: {e}")).ok())
        .clone()
}

#[cfg(test)]
mod tests {
    #[test]
    fn ort_env_is_shared() {
        assert!(super::ort_env().is_some());
        assert!(super::ort_env().is_some());
    }
}
