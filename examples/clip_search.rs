//! Каркас CLIP ViT-B/32 (ONNX int8): preprocess -> инференс -> поиск по тексту.
//! Не подключён к приложению, запускается: cargo run --example clip_search -- <запрос> <картинка...>
//!
//! Нужны локальные файлы (скачать с HF, например из Xenova/clip-vit-base-patch32, *_quantized.onnx):
//!   $XDG_CONFIG_HOME/luma/models/clip/tokenizer.json
//!   $XDG_CONFIG_HOME/luma/models/clip/text_model_quantized.onnx
//!   $XDG_CONFIG_HOME/luma/models/clip/vision_model_quantized.onnx

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use image::ImageReader;
use ndarray::{Array2, Array4};
use ort::environment::Environment;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

const IMAGE_SIZE: usize = 224;
const CTX_LEN: usize = 77;
const BOS: i64 = 49406;
const EOS: i64 = 49407;
// CLIP mean/std (RGB)
const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

// Тот же путь, что и у демона (db_init::config_dir, но пример не видит бин-крейт).
fn clip_dir() -> PathBuf {
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config"))
            .unwrap_or_else(std::env::temp_dir),
    };
    dir.join("luma/models/clip")
}

/// Изображение -> [1,3,224,224] float32 (RGB, нормализация CLIP).
fn preprocess_image(path: &Path) -> Result<Array4<f32>> {
    let img = ImageReader::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .decode()
        .with_context(|| format!("decode {}", path.display()))?;
    let resized = image::imageops::resize(
        &img.to_rgb8(),
        IMAGE_SIZE as u32,
        IMAGE_SIZE as u32,
        image::imageops::FilterType::CatmullRom,
    );
    let mut out = Array4::<f32>::zeros((1, 3, IMAGE_SIZE, IMAGE_SIZE));
    for (y, x, px) in resized.enumerate_pixels() {
        for c in 0..3 {
            out[[0, c, y as usize, x as usize]] = (px[c] as f32 / 255.0 - MEAN[c]) / STD[c];
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

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Нормализованные эмбеддинги -> косинус = скалярное произведение.
fn cosine_matrix(query: &mut [f32], emb: &mut [Vec<f32>]) -> Vec<f32> {
    l2_normalize(query);
    for e in emb.iter_mut() {
        l2_normalize(e);
    }
    emb.iter()
        .map(|e| query.iter().zip(e).map(|(a, b)| a * b).sum())
        .collect()
}

fn first_output(outputs: ort::session::SessionOutputs<'_>) -> Result<Vec<f32>> {
    let (_, value) = outputs.iter().next().context("нет выходов у модели")?;
    Ok(value.try_extract_array::<f32>()?.iter().copied().collect())
}

fn embed_image(env: &Environment, path: &Path) -> Result<Vec<f32>> {
    let model = clip_dir().join("vision_model_quantized.onnx");
    let mut session = Session::builder(env)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .unwrap_or_else(|e| e.recover())
        .commit_from_file(model)?;
    let input = preprocess_image(path)?;
    let tensor = Tensor::from_array(input)?;
    let outputs = session.run(ort::inputs!["pixel_values" => tensor])?;
    first_output(outputs)
}

fn embed_text(env: &Environment, tokenizer: &Tokenizer, text: &str) -> Result<Vec<f32>> {
    let model = clip_dir().join("text_model_quantized.onnx");
    let mut session = Session::builder(env)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .unwrap_or_else(|e| e.recover())
        .commit_from_file(model)?;
    let ids = tokenize_text(tokenizer, text)?;
    let outputs = session.run(ort::inputs!["input_ids" => Tensor::from_array(ids)?])?;
    first_output(outputs)
}

fn main() -> Result<()> {
    let tokenizer_path = clip_dir().join("tokenizer.json");
    for p in [
        &tokenizer_path,
        &clip_dir().join("text_model_quantized.onnx"),
        &clip_dir().join("vision_model_quantized.onnx"),
    ] {
        if !p.exists() {
            bail!("нет {}: скачай из HF (Xenova/clip-vit-base-patch32, *_quantized.onnx) в ~/.config/luma/models/clip/", p.display());
        }
    }

    let mut args = std::env::args().skip(1);
    let query = args.next().unwrap_or_else(|| "a photo of a cat".into());
    let images: Vec<String> = args.collect();
    if images.is_empty() {
        bail!("usage: cargo run --example clip_search -- <запрос> <картинка>...");
    }

    let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!("{e}"))?;
    let env = ort::environment::init().build()?;

    let mut query_emb = embed_text(&env, &tokenizer, &query)?;
    let mut image_embs: Vec<Vec<f32>> = images
        .iter()
        .map(|p| embed_image(&env, Path::new(p)))
        .collect::<Result<_>>()?;

    let scores = cosine_matrix(&mut query_emb, &mut image_embs);
    let mut ranked: Vec<(usize, f32)> = scores.into_iter().enumerate().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    println!("запрос: {query:?}");
    for (i, score) in ranked {
        println!("  {score:.4}  {}", images[i]);
    }
    Ok(())
}
