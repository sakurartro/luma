// Image thumbnails for the files grid, cached as PNGs on disk.
use image::ImageReader;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

// Longest side of a cached thumbnail, px (2x the 48px tile for HiDPI).
pub const THUMB_SIZE: u32 = 96;
// Bigger files keep the icon: decoding them costs too much on a low-RAM box.
const MAX_FILE_BYTES: u64 = 40 * 1024 * 1024;
// Decoder allocation cap (~32 Mpx RGBA); larger images keep the icon.
const MAX_ALLOC: u64 = 128 * 1024 * 1024;

/// $XDG_CACHE_HOME/luma/thumbs (defaults to ~/.cache, /tmp without HOME).
pub fn cache_dir() -> PathBuf {
    let dir = match std::env::var_os("XDG_CACHE_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".cache"))
            .unwrap_or_else(std::env::temp_dir),
    };
    dir.join("luma").join("thumbs")
}

// Cached thumbnail of `path` in `dir`, generated on a miss. None for files
// over the size cap and anything that fails to decode (the icon stays).
pub fn thumbnail(path: &Path, dir: &Path) -> Option<PathBuf> {
    let meta = path.metadata().ok()?;
    if meta.len() > MAX_FILE_BYTES {
        return None;
    }
    // shortcut: DefaultHasher output isn't stable across Rust versions, so a
    // toolchain upgrade just regenerates the cache; switch to a fixed hash if
    // the cache must survive that.
    let mut hasher = DefaultHasher::new();
    (path, meta.modified().ok(), meta.len()).hash(&mut hasher);
    let out = dir.join(format!("{:016x}.png", hasher.finish()));
    if out.exists() {
        return Some(out);
    }

    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    reader.limits(limits);
    let thumb = reader.decode().ok()?.thumbnail(THUMB_SIZE, THUMB_SIZE);
    std::fs::create_dir_all(dir).ok()?;
    // Write + rename: a crash mid-write must not leave a broken PNG that
    // counts as a cache hit forever.
    let tmp = out.with_extension("tmp");
    thumb.save_with_format(&tmp, image::ImageFormat::Png).ok()?;
    std::fs::rename(&tmp, &out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_is_cached_and_small() {
        let dir = std::env::temp_dir().join(format!("luma_thumbs_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.png");
        image::RgbImage::from_pixel(400, 200, image::Rgb([200, 30, 30])).save(&src).unwrap();
        let cache = dir.join("cache");

        let out = thumbnail(&src, &cache).unwrap();
        let thumb = image::open(&out).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (THUMB_SIZE, THUMB_SIZE / 2));

        // A hit returns the cached file untouched instead of decoding again.
        std::fs::write(&out, b"cached").unwrap();
        assert_eq!(thumbnail(&src, &cache).unwrap(), out);
        assert_eq!(std::fs::read(&out).unwrap(), b"cached");

        assert!(thumbnail(&dir.join("missing.png"), &cache).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
