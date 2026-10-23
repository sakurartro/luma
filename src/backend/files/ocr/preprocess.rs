// Препроцессинг: подготовка входов det/rec из файла картинки и кропов.
// Модели и тензорный рантайм сюда не заходят — только image + ndarray.

use anyhow::{Context, Result};
use image::ImageReader;
use ndarray::Array4;
use std::path::Path;

// Ограничения декодинга как в clip: большие картинки не нужны,
// det всё равно ужимает до 960 по длинной стороне.
const MAX_SIDE: u32 = 4096;
const MAX_ALLOC: usize = 48 * 1024 * 1024;

// DB det: длинная сторона ресайзится до 960, H/W кратны 32.
const DET_LIMIT: u32 = 960;

// rec: вход [1,3,48,W].
const REC_HEIGHT: u32 = 48;
const REC_MIN_W: u32 = 16;
// Стандартная ширина rec-входа PP-OCR: длинные строки сжимаются до неё.
const REC_MAX_W: u32 = 320;

// Декод -> RGB-картинка + вход det [1,3,H,W] + масштаб (вход/исходник).
pub(super) fn preprocess_det(path: &Path) -> Result<(image::RgbImage, Array4<f32>, f32)> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC as u64);
    let mut reader = ImageReader::open(path).with_context(|| format!("open {}", path.display()))?;
    reader.limits(limits);
    let img = reader.decode().with_context(|| format!("decode {}", path.display()))?.to_rgb8();

    // Ресайз: длинная сторона <= 960, короткая — с сохранением пропорций.
    let (w, h) = (img.width(), img.height());
    let scale = if w.max(h) > DET_LIMIT { DET_LIMIT as f32 / w.max(h) as f32 } else { 1.0 };
    let rw = ((w as f32 * scale).round() as u32).max(32);
    let rh = ((h as f32 * scale).round() as u32).max(32);
    let resized = image::imageops::resize(&img, rw, rh, image::imageops::FilterType::Triangle);
    // Паддинг до кратности 32 чёрным канвасом (не растягивать — детект
    // чувствителен к пропорциям).
    let pw = rw.div_ceil(32) * 32;
    let ph = rh.div_ceil(32) * 32;
    let mut padded = image::RgbImage::new(pw, ph);
    image::imageops::overlay(&mut padded, &resized, 0, 0);

    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let mut out = Array4::<f32>::zeros((1, 3, ph as usize, pw as usize));
    for (x, y, px) in padded.enumerate_pixels() {
        for c in 0..3 {
            out[[0, c, y as usize, x as usize]] = (px[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    Ok((img, out, scale))
}

// Кропы -> вход rec [N,3,48,Wmax] (каждый ужат/ дополнен до общей ширины
// нулями). Нормализация (x/255-0.5)/0.5.
pub(super) fn preprocess_rec_batch(crops: &[image::RgbImage]) -> Result<Array4<f32>> {
    let widths: Vec<u32> = crops
        .iter()
        .map(|c| {
            let ratio = REC_HEIGHT as f32 / c.height() as f32;
            ((c.width() as f32 * ratio).round() as u32).clamp(REC_MIN_W, REC_MAX_W)
        })
        .collect();
    let wmax = widths.iter().copied().max().unwrap_or(REC_MIN_W);
    let mut out = Array4::<f32>::zeros((crops.len(), 3, REC_HEIGHT as usize, wmax as usize));
    for (i, (crop, &w)) in crops.iter().zip(&widths).enumerate() {
        let resized =
            image::imageops::resize(crop, w, REC_HEIGHT, image::imageops::FilterType::Triangle);
        for (x, y, px) in resized.enumerate_pixels() {
            for c in 0..3 {
                out[[i, c, y as usize, x as usize]] = px[c] as f32 / 255.0 * 2.0 - 1.0;
            }
        }
    }
    Ok(out)
}

// Боксы det (в координатах det-входа) -> кропы исходной картинки для rec.
// scale — отношение det-входа к исходнику; крошечные кропы отбрасываются.
pub(super) fn crop_boxes(
    img: &image::RgbImage,
    boxes: &[(f32, f32, f32, f32)],
    scale: f32,
) -> Vec<image::RgbImage> {
    let mut crops = Vec::new();
    for &(x0, y0, x1, y1) in boxes {
        let crop_w = ((x1 - x0) / scale).round().max(1.0) as u32;
        let crop_h = ((y1 - y0) / scale).round().max(1.0) as u32;
        let cx0 = ((x0 / scale).round().max(0.0) as u32).min(img.width().saturating_sub(1));
        let cy0 = ((y0 / scale).round().max(0.0) as u32).min(img.height().saturating_sub(1));
        let crop_w = crop_w.min(img.width() - cx0);
        let crop_h = crop_h.min(img.height() - cy0);
        if crop_w < 8 || crop_h < 8 {
            continue;
        }
        crops.push(image::imageops::crop_imm(img, cx0, cy0, crop_w, crop_h).to_image());
    }
    crops
}
