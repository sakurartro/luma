// Загрузка ONNX-моделей PaddleOCR (RapidOCR) и det/rec инференс.
// det: DBNet, выход — sigmoid prob map [1,1,H,W]; rec: CRNN, выход
// [T,1,C] softmax, CTC-декод (blank = класс 0).

use anyhow::{anyhow, Context, Result};
use image::ImageReader;
use ndarray::Array4;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

// Модели лежат в $XDG_CONFIG_HOME/luma/models/ocr/.
pub(crate) fn models_dir() -> PathBuf {
    crate::backend::apps::db_init::config_dir().join("models/ocr")
}

pub(crate) fn model_path(name: &str) -> PathBuf {
    models_dir().join(name)
}

// Ограничения декодинга как в clip: большие картинки не нужны,
// det всё равно ужимает до 960 по длинной стороне.
const MAX_SIDE: u32 = 4096;
const MAX_ALLOC: usize = 48 * 1024 * 1024;

// DB det: длинная сторона ресайзится до 960, H/W кратны 32.
const DET_LIMIT: u32 = 960;
const DET_THRESHOLD: f32 = 0.3;
// DBNet предсказывает сжатые внутрь текст-регионы; postprocess обязан
// расширить их обратно (как ClipperOffset в стандартном пайплайне).
const DET_UNCLIP: f32 = 1.5;

// rec: вход [1,3,48,W].
const REC_HEIGHT: u32 = 48;
const REC_MIN_W: u32 = 16;
// Стандартная ширина rec-входа PP-OCR: длинные строки сжимаются до неё.
const REC_MAX_W: u32 = 320;

pub(crate) struct OcrModels {
    det: Session,
    rec: Session,
    // Словарь rec-модели: класс i (i >= 1) -> dict[i - 1]. Класс 0 — blank.
    dict: Vec<String>,
}

impl OcrModels {
    pub(crate) fn load() -> Option<Self> {
        Self::load_from(&models_dir())
    }

    pub(crate) fn load_from(dir: &Path) -> Option<Self> {
        let (det_path, rec_path, dict_path) = (
            dir.join("det.onnx"),
            dir.join("rec.onnx"),
            dir.join("dict.txt"),
        );
        for p in [&det_path, &rec_path, &dict_path] {
            if !p.exists() {
                return None;
            }
        }
        let env = match ort::environment::init().build() {
            Ok(env) => env,
            Err(err) => {
                eprintln!("ocr disabled, ort init failed: {err}");
                return None;
            }
        };
        // memory_pattern off: размеры входа меняются от картинки к картинке,
        // паттерн, записанный под один размер, раздувает арену для других.
        let build = |model: &PathBuf| {
            Session::builder(&env)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .unwrap_or_else(|e| e.recover())
                .with_memory_pattern(false)?
                .with_intra_threads(2)?
                .commit_from_file(model)
        };
        let (Ok(det), Ok(rec), Ok(dict_text)) =
            (build(&det_path), build(&rec_path), std::fs::read_to_string(&dict_path))
        else {
            eprintln!("ocr disabled: models/ocr/ is broken");
            return None;
        };
        let dict: Vec<String> = dict_text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        Some(OcrModels { det, rec, dict })
    }

    // Полный пайплайн по одному файлу: det -> кропы -> rec (одним батчем,
    // иначе оверхед ORT на рантайм ~300мс на кроп) -> склейка строк сверху
    // вниз. Ошибку отдаёт наверх (воркер решает, ретраить ли).
    pub(crate) fn recognize(&mut self, path: &Path) -> Result<String> {
        let (img, det_input, scale) = preprocess_det(path)?;
        let (probs, dh, dw) = self.run_det(det_input)?;
        let mut crops: Vec<image::RgbImage> = Vec::new();
        for (x0, y0, x1, y1) in find_boxes(&probs, dh, dw) {
            // Кроп в координатах исходной картинки.
            let crop_w = ((x1 - x0) / scale).round().max(1.0) as u32;
            let crop_h = ((y1 - y0) / scale).round().max(1.0) as u32;
            let cx0 = (x0 / scale).round().max(0.0) as u32;
            let cy0 = (y0 / scale).round().max(0.0) as u32;
            let cx0 = cx0.min(img.width().saturating_sub(1));
            let cy0 = cy0.min(img.height().saturating_sub(1));
            let crop_w = crop_w.min(img.width() - cx0);
            let crop_h = crop_h.min(img.height() - cy0);
            if crop_w < 8 || crop_h < 8 {
                continue;
            }
            crops.push(image::imageops::crop_imm(&img, cx0, cy0, crop_w, crop_h).to_image());
        }
        if crops.is_empty() {
            return Ok(String::new());
        }
        let texts = self.recognize_batch(&crops)?;
        Ok(texts.into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n"))
    }

    fn run_det(&mut self, input: Array4<f32>) -> Result<(Vec<f32>, usize, usize)> {
        let (_, _, h, w) = input.dim();
        let outputs = self.det.run(ort::inputs!["x" => Tensor::from_array(input)?])?;
        let (_, value) = outputs.iter().next().ok_or_else(|| anyhow!("нет выходов у det"))?;
        let flat: Vec<f32> = value.try_extract_tensor::<f32>()?.1.to_vec();
        if flat.len() != h * w {
            return Err(anyhow!("det output {} != {}x{}", flat.len(), h, w));
        }
        Ok((flat, h, w))
    }

    // Кропы -> тексты: все паддингом к одной ширине, один прогон rec,
    // CTC-декод по каждому ряду батча. Пусто, если rec ничего не распознал.
    fn recognize_batch(&mut self, crops: &[image::RgbImage]) -> Result<Vec<String>> {
        let (input, widths) = preprocess_rec_batch(crops)?;
        let outputs = self.rec.run(ort::inputs!["x" => Tensor::from_array(input)?])?;
        let (_, value) = outputs.iter().next().ok_or_else(|| anyhow!("нет выходов у rec"))?;
        // Выход [T,N,C]: C читаем из модели, не гадаем.
        let (shape, data) = value.try_extract_tensor::<f32>()?;
        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let classes = dims[dims.len() - 1];
        let n = crops.len();
        let t = data.len() / (classes * n);
        let mut texts = Vec::with_capacity(n);
        for row in 0..n {
            let mut text = String::new();
            let mut prev = 0usize;
            for i in 0..t {
                // Смещение ряда внутри [T,N,C]: ряд row, шаг i -> data[(i*n + row)*classes].
                let base = (i * n + row) * classes;
                let r = &data[base..base + classes];
                let (best, _) = r
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or((0, &0.0));
                if best != 0 && best != prev {
                    // Класс 0 — blank; 1..=dict.len() — словарь; хвост за
                    // словарём (97-й класс при 95 строках словаря) — пробел.
                    if let Some(ch) = self.dict.get(best - 1) {
                        text.push_str(ch);
                    } else {
                        text.push(' ');
                    }
                }
                prev = best;
            }
            let _ = widths;
            texts.push(text);
        }
        Ok(texts)
    }
}

// Декод -> RGB-картинка + вход det [1,3,H,W] + масштаб (вход/исходник).
fn preprocess_det(path: &Path) -> Result<(image::RgbImage, Array4<f32>, f32)> {
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
// нулями) + исходные ширины. Нормализация (x/255-0.5)/0.5.
fn preprocess_rec_batch(crops: &[image::RgbImage]) -> Result<(Array4<f32>, Vec<u32>)> {
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
    Ok((out, widths))
}

// Prob map [H*W] -> боксы (x0,y0,x1,y1) в координатах det-входа, отсортированные
// сверху вниз, слева направо. Без OpenCV: связные компоненты + bbox.
// shortcut: повернутый текст детектится криво (bbox по осям), когда пригодится —
// min-area rect по контуру.
fn find_boxes(probs: &[f32], h: usize, w: usize) -> Vec<(f32, f32, f32, f32)> {
    if h == 0 || w == 0 || probs.len() != h * w {
        return Vec::new();
    }
    let mask: Vec<bool> = probs.iter().map(|&p| p >= DET_THRESHOLD).collect();
    let mut visited = vec![false; mask.len()];
    let mut boxes = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || visited[start] {
            continue;
        }
        // Flood fill одной компоненты.
        let mut stack = vec![start];
        visited[start] = true;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0, 0);
        let mut area = 0usize;
        while let Some(idx) = stack.pop() {
            let (x, y) = (idx % w, idx / w);
            area += 1;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < w && ny < h {
                    let n = ny * w + nx;
                    if mask[n] && !visited[n] {
                        visited[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        // Мусор: одиночные пиксели и шумы.
        if area < 20 {
            continue;
        }
        // Unclip: расширяем сжатый DB-бокс. distance = area*ratio/периметр,
        // как в стандартном DBPostProcess (аппроксимация для прямых боксов).
        let bw = (max_x - min_x + 1) as f32;
        let bh = (max_y - min_y + 1) as f32;
        let distance = (area as f32 * DET_UNCLIP) / (2.0 * (bw + bh)).max(1.0);
        let x0 = (min_x as f32 - distance).max(0.0);
        let y0 = (min_y as f32 - distance).max(0.0);
        let x1 = (max_x as f32 + distance).min(w as f32 - 1.0);
        let y1 = (max_y as f32 + distance).min(h as f32 - 1.0);
        boxes.push((x0, y0, x1, y1));
    }
    boxes.sort_by(|a, b| (a.1, a.0).partial_cmp(&(b.1, b.0)).unwrap_or(std::cmp::Ordering::Equal));
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    // Живой прогон на реальных моделях/картинках:
    //   OCR_DIR=/tmp/opencode/ocr OCR_IMG=/tmp/opencode/ocr/test.png cargo test -- --ignored ocr_recognize
    #[test]
    #[ignore]
    fn ocr_recognize() {
        let dir = std::env::var("OCR_DIR").unwrap();
        let img = std::env::var("OCR_IMG").unwrap();
        let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
            panic!("модели в {dir} не читаются");
        };
        let start = std::time::Instant::now();
        let text = models.recognize(Path::new(&img)).unwrap();
        println!("=== {img} === (за {:?})\n{text}", start.elapsed());
        assert!(!text.trim().is_empty(), "пустой текст");
    }

    // Отладка det: боксы и rec по каждому кропу.
    #[test]
    #[ignore]
    fn ocr_debug_det() {
        let dir = std::env::var("OCR_DIR").unwrap();
        let img_path = std::env::var("OCR_IMG").unwrap();
        let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
            panic!("модели в {dir} не читаются");
        };
        let (img, det_input, scale) = preprocess_det(Path::new(&img_path)).unwrap();
        println!("img {}x{}, scale {scale}", img.width(), img.height());
        let t0 = std::time::Instant::now();
        let (probs, dh, dw) = models.run_det(det_input).unwrap();
        println!("det out {dh}x{dw} за {:?}", t0.elapsed());
        let boxes = find_boxes(&probs, dh, dw);
        println!("боксов {}", boxes.len());
        let t1 = std::time::Instant::now();
        let mut n = 0;
        for (x0, y0, x1, y1) in boxes {
            n += 1;
            let tc = std::time::Instant::now();
            let crop_w = ((x1 - x0) / scale).round().max(1.0) as u32;
            let crop_h = ((y1 - y0) / scale).round().max(1.0) as u32;
            let cx0 = ((x0 / scale).round().max(0.0) as u32).min(img.width().saturating_sub(1));
            let cy0 = ((y0 / scale).round().max(0.0) as u32).min(img.height().saturating_sub(1));
            let crop_w = crop_w.min(img.width() - cx0);
            let crop_h = crop_h.min(img.height() - cy0);
            println!("box ({x0},{y0})-({x1},{y1}) -> crop {cx0},{cy0} {crop_w}x{crop_h} за {:?}", tc.elapsed());
            let crop = image::imageops::crop_imm(&img, cx0, cy0, crop_w, crop_h).to_image();
            println!(
                "  -> {:?} за {:?}",
                models.recognize_batch(&[crop]).map(|mut v| v.remove(0)),
                tc.elapsed()
            );
        }
        println!("rec {} кропов за {:?}", n, t1.elapsed());
    }

    // Отладка батча: форма выхода rec при N>1.
    #[test]
    #[ignore]
    fn ocr_debug_batch() {
        let dir = std::env::var("OCR_DIR").unwrap();
        let img_path = std::env::var("OCR_IMG").unwrap();
        let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
            panic!("модели в {dir} не читаются");
        };
        let img = image::open(&img_path).unwrap().to_rgb8();
        // Три одинаковых кропа, чтобы отличить раскладки по содержимому.
        let half = img.width() / 3;
        let crops: Vec<image::RgbImage> = (0..3)
            .map(|i| image::imageops::crop_imm(&img, i * half, 0, half, img.height()).to_image())
            .collect();
        let (input, _) = preprocess_rec_batch(&crops).unwrap();
        println!("input {:?}", input.dim());
        let outputs = models.rec.run(ort::inputs!["x" => Tensor::from_array(input).unwrap()]).unwrap();
        let (_, value) = outputs.iter().next().unwrap();
        let (shape, data) = value.try_extract_tensor::<f32>().unwrap();
        println!("out shape {shape:?}, len {}", data.len());
        let classes = shape[shape.len() - 1] as usize;
        let n = 3;
        let t = data.len() / (classes * n);
        // Печатаем декод в трёх гипотезах раскладки.
        for (name, idx) in [
            ("[T,N,C]", |i: usize, r: usize| (i * n + r) * classes),
            ("[N,T,C]", |i: usize, r: usize| (r * t + i) * classes),
            ("[T,C,N]", |i: usize, r: usize| (i * classes + r)),
        ] {
            let mut texts = Vec::new();
            for row in 0..n {
                let mut text = String::new();
                let mut prev = 0usize;
                for i in 0..t {
                    let base = idx(i, row);
                    if base + classes > data.len() {
                        break;
                    }
                    let r = &data[base..base + classes];
                    let best = r
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap()
                        .0;
                    if best != 0 && best != prev {
                        text.push_str(models.dict.get(best - 1).map(|s| s.as_str()).unwrap_or(" "));
                    }
                    prev = best;
                }
                texts.push(text);
            }
            println!("{name}: {texts:?}");
        }
    }

    // Отладка rec без det: сырые argmax-классы по всей картинке.
    #[test]
    #[ignore]
    fn ocr_debug_rec() {
        let dir = std::env::var("OCR_DIR").unwrap();
        let img_path = std::env::var("OCR_IMG").unwrap();
        let Some(mut models) = OcrModels::load_from(Path::new(&dir)) else {
            panic!("модели в {dir} не читаются");
        };
        let img = image::open(&img_path).unwrap().to_rgb8();
        let (input, _) = preprocess_rec_batch(&[img]).unwrap();
        let outputs = models.rec.run(ort::inputs!["x" => Tensor::from_array(input).unwrap()]).unwrap();
        let (_, value) = outputs.iter().next().unwrap();
        let (shape, data) = value.try_extract_tensor::<f32>().unwrap();
        let classes = shape[shape.len() - 1] as usize;
        let t = data.len() / classes;
        println!("shape {shape:?}, classes {classes}, dict len {}", models.dict.len());
        let mut ids = Vec::new();
        for i in 0..t {
            let row = &data[i * classes..(i + 1) * classes];
            let best = row
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap()
                .0;
            ids.push(best);
        }
        println!("argmax: {ids:?}");
        for off in [0usize, 1] {
            let s: String = ids
                .iter()
                .filter(|&&b| b != 0)
                .map(|&b| models.dict.get(b - 1 + off).map(|s| s.as_str()).unwrap_or("?").to_string())
                .collect();
            println!("off {off}: {s:?}");
        }
    }
}
