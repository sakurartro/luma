// Загрузка ONNX-моделей с HuggingFace для воркеров clip и ocr.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

// Files whose sha256 already matched this session: ensure_models runs on every
// reindex pass, re-hashing ~154 MB per new photo would be wasted work.
static VERIFIED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Make `files` ((имя на диске, url, sha256)) present in `dir` with the
/// expected sha256. Files already there are hashed once per session; a
/// mismatch (fetched before pinning, corrupted, tampered) is downloaded again.
/// Atomic temp-file rename after the hash check, so a truncated or wrong
/// download never becomes a "ready" file. `total`/`done` are byte counters for
/// the UI (total 0 = no download in progress). False = the feature stays off
/// this session; retried on the next start.
pub fn ensure_models(
    tag: &str,
    dir: &Path,
    files: &[(&str, &str, &str)],
    total: &AtomicUsize,
    done: &AtomicUsize,
) -> bool {
    let missing: Vec<_> = files
        .iter()
        .filter(|(name, _, sha256)| {
            let path = dir.join(name);
            if !path.exists() {
                return true;
            }
            let ok = verify(&path, sha256);
            if !ok {
                eprintln!("{tag}: {name} sha256 mismatch, downloading again");
            }
            !ok
        })
        .collect();
    if missing.is_empty() {
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
    for (name, url, sha256) in missing {
        let dst = dir.join(name);
        eprintln!("{tag}: downloading {name}...");
        if let Err(err) = download(&client, url, sha256, &dst, total, done) {
            eprintln!("{tag}: download {name} failed: {err}");
            // Otherwise total > done forever and the UI banner never goes away.
            total.store(0, Ordering::Relaxed);
            return false;
        }
        mark_verified(dst);
    }
    true
}

fn verify(path: &Path, sha256: &str) -> bool {
    if VERIFIED.lock().unwrap().contains(path) {
        return true;
    }
    let ok = sha256_file(path).is_ok_and(|got| got == sha256);
    if ok {
        mark_verified(path.to_path_buf());
    }
    ok
}

fn mark_verified(path: PathBuf) {
    VERIFIED.lock().unwrap().insert(path);
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn download(
    client: &reqwest::blocking::Client,
    url: &str,
    sha256: &str,
    dst: &Path,
    total: &AtomicUsize,
    done: &AtomicUsize,
) -> Result<()> {
    let tmp = dst.with_extension("part");
    let result = fetch(client, url, &tmp, total, done).and_then(|got| {
        anyhow::ensure!(got == sha256, "sha256 mismatch: expected {sha256}, got {got}");
        Ok(std::fs::rename(&tmp, dst)?)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// Streams `url` into `tmp`, returns the sha256 of what was written.
fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    tmp: &Path,
    total: &AtomicUsize,
    done: &AtomicUsize,
) -> Result<String> {
    let mut resp = client.get(url).send()?.error_for_status()?;
    total.fetch_add(resp.content_length().unwrap_or(0) as usize, Ordering::Relaxed);
    let mut file = std::fs::File::create(tmp)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done.fetch_add(n, Ordering::Relaxed);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // sha256 of the one-byte body "x".
    const X_SHA: &str = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("luma_dl_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // One-shot HTTP server answering any request with body "x".
    fn serve_x() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0u8; 4096]);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx");
        });
        format!("http://{addr}/m")
    }

    #[test]
    fn verified_files_skip_the_network() {
        let dir = test_dir("skip");
        std::fs::write(dir.join("a.onnx"), "x").unwrap();
        let (total, done) = (AtomicUsize::new(0), AtomicUsize::new(0));

        // The url is unreachable: any request would make this return false.
        let ok = ensure_models("test", &dir, &[("a.onnx", "http://127.0.0.1:1/a", X_SHA)], &total, &done);

        assert!(ok);
        assert_eq!(total.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mismatched_existing_file_is_downloaded_again() {
        let dir = test_dir("refetch");
        std::fs::write(dir.join("a.onnx"), "tampered").unwrap();
        let (total, done) = (AtomicUsize::new(0), AtomicUsize::new(0));

        let ok = ensure_models("test", &dir, &[("a.onnx", &serve_x(), X_SHA)], &total, &done);

        assert!(ok);
        assert_eq!(std::fs::read(dir.join("a.onnx")).unwrap(), b"x");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wrong_download_is_never_kept() {
        let dir = test_dir("reject");
        let (total, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let wrong = "0".repeat(64);

        let ok = ensure_models("test", &dir, &[("a.onnx", &serve_x(), &wrong)], &total, &done);

        assert!(!ok);
        assert!(!dir.join("a.onnx").exists());
        assert!(!dir.join("a.part").exists());
        assert_eq!(total.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
