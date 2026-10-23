// Загрузка ONNX-моделей с HuggingFace для воркеров clip и ocr.

use anyhow::Result;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Download `files` ((имя на диске, url)) into `dir` once; files already there
/// are kept. Atomic temp-file rename, so an interrupted run never leaves a
/// truncated "ready" file. `total`/`done` are byte counters for the UI
/// (total 0 = no download in progress). False = the feature stays off this
/// session; retried on the next start.
pub fn ensure_models(
    tag: &str,
    dir: &Path,
    files: &[(&str, &str)],
    total: &AtomicUsize,
    done: &AtomicUsize,
) -> bool {
    if files.iter().all(|(name, _)| dir.join(name).exists()) {
        return true;
    }
    total.store(0, Ordering::Relaxed);
    done.store(0, Ordering::Relaxed);
    let client = match reqwest::blocking::Client::builder().build() {
        Ok(client) => client,
        Err(err) => {
            eprintln!("{tag}: http client error: {err}");
            return false;
        }
    };
    if let Err(err) = std::fs::create_dir_all(dir) {
        eprintln!("{tag} models dir error: {err}");
        return false;
    }
    for (name, url) in files {
        let dst = dir.join(name);
        if dst.exists() {
            continue;
        }
        eprintln!("{tag}: downloading {name}...");
        if let Err(err) = download(&client, url, &dst, total, done) {
            eprintln!("{tag}: download {name} failed: {err}");
            return false;
        }
    }
    true
}

fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    dst: &Path,
    total: &AtomicUsize,
    done: &AtomicUsize,
) -> Result<()> {
    let mut resp = client.get(url).send()?.error_for_status()?;
    total.fetch_add(resp.content_length().unwrap_or(0) as usize, Ordering::Relaxed);
    let tmp = dst.with_extension("part");
    let mut file = std::fs::File::create(&tmp)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done.fetch_add(n, Ordering::Relaxed);
    }
    std::fs::rename(&tmp, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_files_skip_the_network() {
        let dir = std::env::temp_dir().join(format!("luma_dl_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.onnx"), "x").unwrap();
        let (total, done) = (AtomicUsize::new(0), AtomicUsize::new(0));

        // The url is unreachable: any request would make this return false.
        let ok = ensure_models("test", &dir, &[("a.onnx", "http://127.0.0.1:1/a")], &total, &done);

        assert!(ok);
        assert_eq!(total.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
