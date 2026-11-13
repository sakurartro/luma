// Frames of a video via the ffmpeg/ffprobe CLI: no mature pure-Rust decoder
// covers mp4/mkv/webm with h264/hevc/vp9/av1.

use anyhow::{bail, Context, Result};
use image::DynamicImage;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

// Fewer frames than this miss too much of a short clip.
const MIN_FRAMES: u32 = 4;

// Runs `cmd args` under coreutils `timeout`: std has no wait-with-timeout and
// ffmpeg can hang on a broken file. Non-zero exit (124 = timed out) is an error.
fn run(cmd: &str, args: &[&OsStr]) -> Result<Vec<u8>> {
    let out = Command::new("timeout")
        .arg("60")
        .arg(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("run {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} {}: {}", out.status, String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// ffmpeg and ffprobe both run; checked once per process.
pub fn ffmpeg_available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| ["ffmpeg", "ffprobe"].iter().all(|cmd| run(cmd, &["-version".as_ref()]).is_ok()))
}

/// Container duration in seconds; an error when ffprobe can't tell
/// (broken file, stream without a duration).
pub fn duration_sec(path: &Path) -> Result<f64> {
    let args: [&OsStr; 7] = [
        "-v".as_ref(), "error".as_ref(),
        "-show_entries".as_ref(), "format=duration".as_ref(),
        "-of".as_ref(), "default=nw=1:nk=1".as_ref(),
        path.as_os_str(),
    ];
    let out = String::from_utf8(run("ffprobe", &args)?)?;
    match out.trim().parse::<f64>() {
        Ok(d) if d.is_finite() && d > 0.0 => Ok(d),
        _ => bail!("no duration: {:?}", out.trim()),
    }
}

/// Whether the file has an audio stream.
pub fn has_audio(path: &Path) -> Result<bool> {
    let args: [&OsStr; 9] = [
        "-v".as_ref(), "error".as_ref(),
        "-select_streams".as_ref(), "a".as_ref(),
        "-show_entries".as_ref(), "stream=index".as_ref(),
        "-of".as_ref(), "csv=p=0".as_ref(),
        path.as_os_str(),
    ];
    Ok(!run("ffprobe", &args)?.trim_ascii().is_empty())
}

/// Seconds to sample: one frame per `every_sec`, at least MIN_FRAMES and at
/// most `max_frames`, frame i in the middle of slice i of n equal slices.
pub fn timestamps(duration: f64, every_sec: u32, max_frames: u32) -> Vec<f64> {
    // every_sec 0 gives inf -> max_frames; max_frames 0 still gives one frame
    let n = ((duration / every_sec as f64).ceil() as u32).max(MIN_FRAMES).min(max_frames.max(1));
    (0..n).map(|i| duration * (i as f64 + 0.5) / n as f64).collect()
}

/// The frame at `t` seconds, decoded in memory. `-ss` before `-i` seeks to
/// the nearest keyframe, so only a few frames get decoded even in a movie.
pub fn frame_at(path: &Path, t: f64) -> Result<DynamicImage> {
    let t = format!("{t:.3}");
    let args: [&OsStr; 14] = [
        "-v".as_ref(), "error".as_ref(), "-nostdin".as_ref(),
        "-ss".as_ref(), t.as_ref(),
        "-i".as_ref(), path.as_os_str(),
        "-frames:v".as_ref(), "1".as_ref(),
        "-f".as_ref(), "image2pipe".as_ref(),
        "-c:v".as_ref(), "bmp".as_ref(),
        "-".as_ref(),
    ];
    let bmp = run("ffmpeg", &args)?;
    image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).with_context(|| format!("no frame at {t}s"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_one_per_interval_within_limits() {
        // 100 s / 10 s = 10 frames, centered in their slices
        let ts = timestamps(100.0, 10, 32);
        assert_eq!(ts.len(), 10);
        assert_eq!((ts[0], ts[9]), (5.0, 95.0));
        // short video: still MIN_FRAMES
        let ts = timestamps(2.0, 10, 32);
        assert_eq!(ts, [0.25, 0.75, 1.25, 1.75]);
        // long video: capped at max_frames, all inside the video
        let ts = timestamps(7200.0, 10, 32);
        assert_eq!(ts.len(), 32);
        assert!(ts.iter().all(|t| *t > 0.0 && *t < 7200.0));
        // user limits below the minimum and zeros don't panic
        assert_eq!(timestamps(100.0, 10, 2).len(), 2);
        assert_eq!(timestamps(100.0, 10, 0).len(), 1);
        assert_eq!(timestamps(100.0, 0, 32).len(), 32);
    }

    #[test]
    fn frame_from_generated_video() {
        if !ffmpeg_available() {
            eprintln!("skip: no ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("luma_frames_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("test.mp4");
        let lavfi = ["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=3:size=64x48:rate=10"];
        let status = Command::new("ffmpeg").args(lavfi).arg(&video).status().unwrap();
        assert!(status.success());

        let d = duration_sec(&video).unwrap();
        assert!((d - 3.0).abs() < 0.2, "{d}");
        assert!(!has_audio(&video).unwrap());
        let frame = frame_at(&video, timestamps(d, 10, 32)[1]).unwrap();
        assert_eq!((frame.width(), frame.height()), (64, 48));

        // not a video: errors, no panic
        let text = dir.join("fake.mp4");
        std::fs::write(&text, "not a video").unwrap();
        assert!(duration_sec(&text).is_err());
        assert!(frame_at(&text, 1.0).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
