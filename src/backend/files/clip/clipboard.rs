// Картинка из буфера обмена: arboard, при неудаче — wl-paste / xclip (PNG через stdout).

use image::{DynamicImage, RgbaImage};
use std::process::Command;

fn rgba_to_image(width: usize, height: usize, bytes: Vec<u8>) -> Option<DynamicImage> {
    RgbaImage::from_raw(width as u32, height as u32, bytes).map(DynamicImage::ImageRgba8)
}

fn from_command(program: &str, args: &[&str]) -> Option<DynamicImage> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    image::load_from_memory(&out.stdout).ok()
}

pub fn read_image() -> Option<DynamicImage> {
    if let Ok(img) = arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
        if let Some(img) = rgba_to_image(img.width, img.height, img.bytes.into_owned()) {
            return Some(img);
        }
    }
    from_command("wl-paste", &["--type", "image/png"])
        .or_else(|| from_command("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_converts() {
        let img = rgba_to_image(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        assert_eq!((img.width(), img.height()), (2, 1));
        assert_eq!(img.to_rgba8().get_pixel(1, 0).0, [4, 5, 6, 255]);
    }

    #[test]
    fn rgba_wrong_size_is_none() {
        assert!(rgba_to_image(2, 2, vec![0; 4]).is_none());
    }
}
