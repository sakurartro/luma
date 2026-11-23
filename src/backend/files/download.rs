// Download pinned search models and their shared ONNX Runtime.

use super::video::{frames::ffmpeg_available, speech};
use super::{clip, ocr};
use crate::backend::apps::db_init::config_dir;
use crate::backend::settings::Settings;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// Files whose sha256 already matched this session. Model checks run on every
// reindex pass, re-hashing ~154 MB per new photo would be wasted work.
static VERIFIED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

enum DownloadProgress {
    Idle,
    Downloading(i32),
    Complete(Instant),
}

impl DownloadProgress {
    fn percent(&self) -> Option<i32> {
        match self {
            Self::Downloading(percent) => Some(*percent),
            Self::Complete(at) if at.elapsed() < Duration::from_millis(150) => Some(100),
            _ => None,
        }
    }
}

static DOWNLOAD_PROGRESS: Mutex<DownloadProgress> = Mutex::new(DownloadProgress::Idle);

pub fn progress() -> Option<i32> {
    DOWNLOAD_PROGRESS.lock().unwrap().percent()
}

struct Batch<'a> {
    progress: &'a Mutex<DownloadProgress>,
    total: u64,
    received: u64,
    complete: bool,
}

impl<'a> Batch<'a> {
    fn new(total: u64, progress: &'a Mutex<DownloadProgress>) -> Self {
        if total > 0 {
            *progress.lock().unwrap() = DownloadProgress::Downloading(0);
        }
        Self { progress, total, received: 0, complete: false }
    }

    fn record(&mut self, bytes: u64) {
        self.received += bytes;
        // All bytes can arrive before SHA-256 verification and installation finish.
        let percent = (self.received * 100 / self.total).min(99) as i32;
        *self.progress.lock().unwrap() = DownloadProgress::Downloading(percent);
    }
}

impl Drop for Batch<'_> {
    fn drop(&mut self) {
        if self.total > 0 {
            *self.progress.lock().unwrap() = if self.complete {
                DownloadProgress::Complete(Instant::now())
            } else {
                DownloadProgress::Idle
            };
        }
    }
}

/// Local name, pinned URL, SHA-256 and size in bytes.
pub(super) type ModelFile<'a> = (&'a str, &'a str, &'a str, u64);

pub(super) struct PendingModels<'a> {
    tag: &'a str,
    dir: PathBuf,
    files: Vec<ModelFile<'a>>,
}

impl<'a> PendingModels<'a> {
    pub(super) fn new(tag: &'a str, dir: PathBuf, files: &[ModelFile<'a>]) -> Self {
        let files = files
            .iter()
            .copied()
            .filter(|(name, _, sha256, _)| {
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
        Self { tag, dir, files }
    }

    fn bytes(&self) -> u64 {
        self.files.iter().map(|(_, _, _, size)| size).sum()
    }

    pub(super) fn install_as_batch(&self) -> bool {
        let mut batch = Batch::new(self.bytes(), &DOWNLOAD_PROGRESS);
        batch.complete = self.install(&mut batch);
        batch.complete
    }

    fn install(&self, batch: &mut Batch<'_>) -> bool {
        if self.files.is_empty() {
            return true;
        }
        let tag = self.tag;
        let client = match reqwest::blocking::Client::builder().build() {
            Ok(client) => client,
            Err(err) => {
                eprintln!("{tag}: http client error: {err}");
                return false;
            }
        };
        if let Err(err) = std::fs::create_dir_all(&self.dir) {
            eprintln!("{tag} models dir error: {err}");
            return false;
        }
        for &file in &self.files {
            let (name, ..) = file;
            let dst = self.dir.join(name);
            eprintln!("{tag}: downloading {name}...");
            if let Err(err) = download(&client, file, &dst, batch) {
                eprintln!("{tag}: download {name} failed: {err}");
                return false;
            }
            mark_verified(dst);
        }
        true
    }
}

#[derive(Default)]
pub struct SearchAssets {
    pub clip: bool,
    pub ocr: bool,
    pub speech: bool,
}

pub fn prepare_search_assets(st: &Settings) -> SearchAssets {
    let clip = st.semantic.then(clip::pending_models);
    let ocr = st.ocr.then(ocr::pending_models);
    let speech = (st.video && st.video_speech && ffmpeg_available()).then(speech::pending_models);
    if clip.is_none() && ocr.is_none() && speech.is_none() {
        return SearchAssets::default();
    }

    let ort = ORT_LIBRARY.get().is_none().then(OrtLibrary::plan);
    let total = ort.as_ref().map_or(0, OrtLibrary::bytes)
        + [&clip, &ocr, &speech]
            .into_iter()
            .filter_map(Option::as_ref)
            .map(PendingModels::bytes)
            .sum::<u64>();
    let mut batch = Batch::new(total, &DOWNLOAD_PROGRESS);
    if ORT_LIBRARY.get_or_init(|| ort?.install(&mut batch)).is_none() {
        return SearchAssets::default();
    }

    let assets = SearchAssets {
        clip: clip.is_none_or(|models| models.install(&mut batch)),
        ocr: ocr.is_none_or(|models| models.install(&mut batch)),
        speech: speech.is_none_or(|models| models.install(&mut batch)),
    };
    batch.complete = assets.clip && assets.ocr && assets.speech;
    assets
}

// ONNX Runtime is loaded at runtime (ort `load-dynamic`) instead of linked in:
// pyke's static build needs glibc >= 2.38, Microsoft's wheel only 2.28. The
// wheel is unpacked once; its tiny size beats a tar/gzip dependency.
const ORT_WHEEL: &str = "onnxruntime-1.31.1-cp312-cp312-manylinux_2_28_x86_64.whl";
const ORT_WHEEL_URL: &str = "https://files.pythonhosted.org/packages/eb/27/c6a823bac55b2977bea2027c3ec9393a2c1002cb9a09eb40354563e02730/onnxruntime-1.31.1-cp312-cp312-manylinux_2_28_x86_64.whl";
const ORT_WHEEL_SHA: &str = "1f3dea47f2f0e5117c2404cea7d21fd253ae7836a3bf167a501cdfbb2600d536";
const ORT_FILES: [ModelFile<'static>; 1] = [(ORT_WHEEL, ORT_WHEEL_URL, ORT_WHEEL_SHA, 23_760_988)];
const ORT_LIB: &str = "libonnxruntime.so.1.31.1";
const ORT_LIB_SHA: &str = "fc44112fbe06d7d689c57e11d3862031f64711007e4ee020f8eed5185fd2c1d3";
static ORT_LIBRARY: OnceLock<Option<PathBuf>> = OnceLock::new();

enum OrtLibrary {
    Ready(PathBuf),
    NeedsWheel(PendingModels<'static>),
}

impl OrtLibrary {
    fn plan() -> Self {
        Self::plan_from(std::env::var_os("ORT_DYLIB_PATH"), config_dir().join("models/ort"))
    }

    fn plan_from(override_path: Option<std::ffi::OsString>, dir: PathBuf) -> Self {
        if let Some(path) = override_path.filter(|p| !p.is_empty()) {
            return Self::Ready(path.into());
        }
        let lib = dir.join(ORT_LIB);
        if verify(&lib, ORT_LIB_SHA) {
            Self::Ready(lib)
        } else {
            Self::NeedsWheel(PendingModels::new("ort", dir, &ORT_FILES))
        }
    }

    fn bytes(&self) -> u64 {
        match self {
            Self::Ready(_) => 0,
            Self::NeedsWheel(models) => models.bytes(),
        }
    }

    fn install(self, batch: &mut Batch<'_>) -> Option<PathBuf> {
        let models = match self {
            Self::Ready(lib) => return Some(lib),
            Self::NeedsWheel(models) => models,
        };
        if !models.install(batch) {
            return None;
        }
        let lib = models.dir.join(ORT_LIB);
        let wheel = models.dir.join(ORT_WHEEL);
        let result = unpack_ort_lib(&wheel, &lib);
        let _ = std::fs::remove_file(&wheel);
        match result {
            Ok(()) => Some(lib),
            Err(err) => {
                eprintln!("ort: unpacking {ORT_WHEEL} failed: {err}");
                None
            }
        }
    }
}

/// The GUI prepares this path at startup. Standalone callers can prepare it on demand.
pub(super) fn ort_library() -> Option<&'static Path> {
    ORT_LIBRARY
        .get_or_init(|| {
            let ort = OrtLibrary::plan();
            let mut batch = Batch::new(ort.bytes(), &DOWNLOAD_PROGRESS);
            let lib = ort.install(&mut batch);
            batch.complete = lib.is_some();
            lib
        })
        .as_deref()
}

fn unpack_ort_lib(wheel: &Path, lib: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(wheel)?))?;
    let tmp = lib.with_extension("part");
    let result = (|| {
        let mut entry = zip.by_name(&format!("onnxruntime/capi/{ORT_LIB}"))?;
        std::io::copy(&mut entry, &mut std::fs::File::create(&tmp)?)?;
        let got = sha256_file(&tmp)?;
        anyhow::ensure!(got == ORT_LIB_SHA, "sha256 mismatch: expected {ORT_LIB_SHA}, got {got}");
        Ok(std::fs::rename(&tmp, lib)?)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
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
    (_, url, sha256, size): ModelFile<'_>,
    dst: &Path,
    batch: &mut Batch<'_>,
) -> Result<()> {
    let tmp = dst.with_extension("part");
    let result = fetch(client, url, &tmp, size, batch).and_then(|got| {
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
    size: u64,
    batch: &mut Batch<'_>,
) -> Result<String> {
    let mut resp = client.get(url).send()?.error_for_status()?;
    let mut file = std::fs::File::create(tmp)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut received = 0;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        received += n as u64;
        anyhow::ensure!(received <= size, "download exceeds expected size of {size} bytes");
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        batch.record(n as u64);
    }
    anyhow::ensure!(received == size, "expected {size} bytes, received {received}");
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
        // The url is unreachable: any request would make this return false.
        let models = PendingModels::new("test", dir.clone(), &[("a.onnx", "http://127.0.0.1:1/a", X_SHA, 1)]);
        let progress = Mutex::new(DownloadProgress::Idle);
        let mut batch = Batch::new(models.bytes(), &progress);
        batch.complete = models.install(&mut batch);

        assert!(batch.complete);
        drop(batch);
        assert_eq!(progress.lock().unwrap().percent(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mismatched_existing_file_is_downloaded_again() {
        let dir = test_dir("refetch");
        std::fs::write(dir.join("a.onnx"), "tampered").unwrap();
        let url = serve_x();
        let models = PendingModels::new("test", dir.clone(), &[("a.onnx", &url, X_SHA, 1)]);
        let progress = Mutex::new(DownloadProgress::Idle);
        let mut batch = Batch::new(models.bytes(), &progress);

        assert_eq!(models.bytes(), 1);
        assert!(models.install(&mut batch));
        assert_eq!(std::fs::read(dir.join("a.onnx")).unwrap(), b"x");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wrong_download_is_never_kept() {
        let dir = test_dir("reject");
        let url = serve_x();
        let wrong = "0".repeat(64);
        let models = PendingModels::new("test", dir.clone(), &[("a.onnx", &url, &wrong, 1)]);
        let progress = Mutex::new(DownloadProgress::Idle);
        let mut batch = Batch::new(models.bytes(), &progress);

        assert!(!models.install(&mut batch));
        assert!(!dir.join("a.onnx").exists());
        assert!(!dir.join("a.part").exists());
        drop(batch);
        assert_eq!(progress.lock().unwrap().percent(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_total_covers_multiple_groups_and_skips_cached_files() {
        let dir = test_dir("batch");
        std::fs::write(dir.join("cached.onnx"), "x").unwrap();
        let url_a = serve_x();
        let url_b = serve_x();
        let first = PendingModels::new("first", dir.clone(), &[
            ("cached.onnx", "http://127.0.0.1:1/cached", X_SHA, 1),
            ("a.onnx", &url_a, X_SHA, 1),
        ]);
        let second = PendingModels::new("second", dir.clone(), &[("b.onnx", &url_b, X_SHA, 1)]);
        let progress = Mutex::new(DownloadProgress::Idle);
        let mut batch = Batch::new(first.bytes() + second.bytes(), &progress);

        assert_eq!(progress.lock().unwrap().percent(), Some(0));
        assert!(first.install(&mut batch));
        assert_eq!(progress.lock().unwrap().percent(), Some(50));
        assert!(second.install(&mut batch));
        assert_eq!(progress.lock().unwrap().percent(), Some(99));
        batch.complete = true;
        drop(batch);
        assert!(matches!(*progress.lock().unwrap(), DownloadProgress::Complete(_)));
        assert_eq!(DownloadProgress::Complete(Instant::now() - Duration::from_secs(1)).percent(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn runtime_override_skips_the_wheel() {
        let dir = test_dir("ort_override");
        let path = dir.join("custom.so");
        let ort = OrtLibrary::plan_from(Some(path.clone().into_os_string()), dir.clone());
        let progress = Mutex::new(DownloadProgress::Idle);
        let mut batch = Batch::new(ort.bytes(), &progress);

        assert_eq!(ort.bytes(), 0);
        assert_eq!(ort.install(&mut batch), Some(path));
        let bundled = OrtLibrary::plan_from(Some("".into()), dir.clone());
        assert_eq!(bundled.bytes(), ORT_FILES[0].3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
