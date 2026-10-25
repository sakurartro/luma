// Загрузка ONNX-моделей (один раз на поток) и препроцессинг/инференс CLIP.

use anyhow::{anyhow, Context, Result};
use image::ImageReader;
use ndarray::{Array2, Array3, Array4};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

const IMAGE_SIZE: usize = 224;
const CTX_LEN: usize = 77;
const BOS: i64 = 49406;
const EOS: i64 = 49407;
// CLIP mean/std (RGB)
const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

// Модели лежат в $XDG_CONFIG_HOME/luma/models/clip/, рядом с бд и настройками —
// оттуда их не сносит ни переустановка бинарника, ни релиз-сборка.
pub(crate) fn models_dir() -> PathBuf {
    crate::backend::apps::db_init::config_dir().join("models/clip")
}

pub(crate) fn model_path(name: &str) -> PathBuf {
    models_dir().join(name)
}

// Inference batch: tensor [N,3,224,224] f32 ≈ N*0.6 MB, so 4 ≈ 2.5 MB per
// run. Small on purpose: ORT's arena grows to the peak activation usage of
// the largest batch (attention maps scale with N) and never returns it to
// the OS, so a big batch inflates steady-state RSS.
pub(crate) const INFERENCE_BATCH: usize = 4;

pub(crate) struct ClipModels {
    vision: Session,
    text: Session,
    tokenizer: Tokenizer,
}

// What survives indexing: queries need only the text side; the vision
// session is dropped after indexing so its ORT arena is freed.
pub(crate) struct QueryModels {
    text: Session,
    tokenizer: Tokenizer,
}

impl ClipModels {
    // Sessions/tokenizer are built once per worker thread and reused for every
    // image (creating a session per image is what made the example unusable).
    pub(crate) fn load() -> Option<Self> {
        let (tokenizer_path, text_model, vision_model) = (
            model_path("tokenizer.json"),
            model_path("text_model_quantized.onnx"),
            model_path("vision_model_quantized.onnx"),
        );
        for p in [&tokenizer_path, &text_model, &vision_model] {
            if !p.exists() {
                return None;
            }
        }
        let env = match ort::environment::init().build() {
            Ok(env) => env,
            Err(err) => {
                eprintln!("clip disabled, ort init failed: {err}");
                return None;
            }
        };
        // `threads` limits intra-op parallelism; 0 = ORT default. Vision
        // batches are the heavy part of indexing and would otherwise
        // saturate all cores; the text side runs one tiny query embedding.
        let build = |model: &PathBuf, threads: usize| {
            let builder = Session::builder(&env)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .unwrap_or_else(|e| e.recover())
                // the batch size varies (short last group), the pattern
                // recorded for one size only overallocates for others
                .with_memory_pattern(false)?;
            let mut builder =
                if threads > 0 { builder.with_intra_threads(threads)? } else { builder };
            builder.commit_from_file(model)
        };
        let (Ok(vision), Ok(text), Ok(tokenizer)) = (
            build(&vision_model, 2),
            build(&text_model, 0),
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow!("{e}")),
        ) else {
            eprintln!("clip disabled: models/clip/ is broken");
            return None;
        };
        Some(ClipModels { vision, text, tokenizer })
    }

    // End of the indexing worker: consume the full pair, drop the vision
    // session (its arena is the memory hog) and keep only the query side.
    pub(crate) fn into_query(self) -> QueryModels {
        QueryModels { text: self.text, tokenizer: self.tokenizer }
    }

    // None rows = failed images; they stay unindexed and are retried on the
    // next start.
    // Preprocess only the current batch and run inference on it: keeping the
    // whole DB chunk preprocessed (~0.6 MB per image) would waste ~150 MB.
    pub(crate) fn embed_images(&mut self, paths: &[String]) -> Vec<Option<Vec<f32>>> {
        let mut out: Vec<Option<Vec<f32>>> = vec![None; paths.len()];
        for (g, group) in paths.chunks(INFERENCE_BATCH).enumerate() {
            let mut imgs = Vec::with_capacity(group.len());
            let mut ok_idx = Vec::with_capacity(group.len());
            for (i, p) in group.iter().enumerate() {
                match preprocess_image(Path::new(p)) {
                    Ok(img) => {
                        imgs.push(img);
                        ok_idx.push(i);
                    }
                    Err(err) => eprintln!("clip: skip {p}: {err}"),
                }
            }
            match run_images(&mut self.vision, &imgs) {
                Ok(rows) => {
                    for (row, i) in rows.into_iter().zip(ok_idx) {
                        out[g * INFERENCE_BATCH + i] = Some(row);
                    }
                }
                Err(err) => eprintln!("clip: batch {g} inference failed: {err}"),
            }
        }
        out
    }
}

impl QueryModels {
    // Kept hot in the MODELS mutex; clip_search calls it per query.
    pub(crate) fn embed_text(&mut self, text: &str) -> Result<Vec<f32>> {
        let ids = tokenize_text(&self.tokenizer, text)?;
        let outputs = self.text.run(ort::inputs!["input_ids" => Tensor::from_array(ids)?])?;
        Ok(first_output_rows(outputs, 1)?.remove(0))
    }
}

/// Изображение -> [3,224,224] f32 (RGB, нормализация CLIP).
fn preprocess_image(path: &Path) -> Result<Array3<f32>> {
    // Декодинг ограничен: 224x224 на выходе, больше 4096 по стороне и 48 МБ
    // на буфер не нужно — это режет пиковую память декодинга.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(48 * 1024 * 1024);
    let mut reader = ImageReader::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    reader.limits(limits);
    let img = reader
        .decode()
        .with_context(|| format!("decode {}", path.display()))?;
    let resized = image::imageops::resize(
        &img.to_rgb8(),
        IMAGE_SIZE as u32,
        IMAGE_SIZE as u32,
        image::imageops::FilterType::CatmullRom,
    );
    let mut out = Array3::<f32>::zeros((3, IMAGE_SIZE, IMAGE_SIZE));
    for (y, x, px) in resized.enumerate_pixels() {
        for c in 0..3 {
            out[[c, y as usize, x as usize]] = (px[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    Ok(out)
}

/// Текст -> input_ids [1,77] (BOS, токены, EOS, паддинг нулями).
/// У экспорта Xenova один вход input_ids: каузальное внимание + паддинг после EOS
/// уже в графе, маска не нужна.
fn tokenize_text(tokenizer: &Tokenizer, text: &str) -> Result<Array2<i64>> {
    let encoding = tokenizer
        .encode(text, false)
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("tokenize {text:?}"))?;
    let tokens: Vec<i64> = encoding.get_ids().iter().map(|&t| t as i64).collect();

    let mut ids = vec![0i64; CTX_LEN];
    ids[0] = BOS;
    let mut i = 1;
    for &t in &tokens {
        if i >= CTX_LEN - 1 {
            break; // обрезаем длинный текст, оставляя место под EOS
        }
        ids[i] = t;
        i += 1;
    }
    ids[i] = EOS;

    Ok(Array2::from_shape_vec((1, CTX_LEN), ids)?)
}

// Cosine = dot-product of normalized vectors; clip_search normalizes both
// sides before scoring.
pub(crate) fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

// First model output, split into n equal rows (one per input in the batch).
fn first_output_rows(outputs: ort::session::SessionOutputs<'_>, n: usize) -> Result<Vec<Vec<f32>>> {
    let (_, value) = outputs.iter().next().context("нет выходов у модели")?;
    let flat: Vec<f32> = value.try_extract_array::<f32>()?.iter().copied().collect();
    let row_len = flat.len() / n;
    Ok(flat.chunks(row_len).map(|r| r.to_vec()).collect())
}

// Embed the ok rows in one run; falls back to per-image runs if the model
// doesn't accept a dynamic batch.
fn run_images(session: &mut Session, imgs: &[Array3<f32>]) -> Result<Vec<Vec<f32>>> {
    let n = imgs.len();
    let flat: Vec<f32> = imgs.iter().flat_map(|img| img.iter().copied()).collect();
    let batch = Array4::from_shape_vec((n, 3, IMAGE_SIZE, IMAGE_SIZE), flat)?;
    let tensor = Tensor::from_array(batch)?;
    if let Ok(outputs) = session.run(ort::inputs!["pixel_values" => tensor]) {
        return first_output_rows(outputs, n);
    }
    // модель без динамического батча — по одной
    let mut rows = Vec::with_capacity(n);
    for img in imgs {
        let tensor = Tensor::from_array(img.clone().into_shape_with_order((1, 3, IMAGE_SIZE, IMAGE_SIZE))?)?;
        let outputs = session.run(ort::inputs!["pixel_values" => tensor])?;
        rows.push(first_output_rows(outputs, 1)?.remove(0));
    }
    Ok(rows)
}
