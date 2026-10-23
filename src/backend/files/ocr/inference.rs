// Загрузка ONNX-моделей PaddleOCR (RapidOCR) и прогон det/rec. Пред- и
// постобработка — в preprocess.rs / postprocess.rs.
// det: DBNet, выход — sigmoid prob map [1,1,H,W]; rec: CRNN, выход
// [T,N,C] softmax, CTC-декод (blank = класс 0).

use super::postprocess::{ctc_decode, find_boxes};
use super::preprocess::{crop_boxes, preprocess_det, preprocess_rec_batch};
use anyhow::{anyhow, Result};
use ndarray::Array4;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

// Модели лежат в $XDG_CONFIG_HOME/luma/models/ocr/.
pub(crate) fn models_dir() -> PathBuf {
    crate::backend::apps::db_init::config_dir().join("models/ocr")
}

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
        let crops = crop_boxes(&img, &find_boxes(&probs, dh, dw), scale);
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
        let input = preprocess_rec_batch(crops)?;
        let outputs = self.rec.run(ort::inputs!["x" => Tensor::from_array(input)?])?;
        let (_, value) = outputs.iter().next().ok_or_else(|| anyhow!("нет выходов у rec"))?;
        // Выход [T,N,C]: C читаем из модели, не гадаем.
        let (shape, data) = value.try_extract_tensor::<f32>()?;
        let classes = shape[shape.len() - 1] as usize;
        Ok(ctc_decode(data, crops.len(), classes, &self.dict))
    }
}

#[cfg(test)]
mod tests;
