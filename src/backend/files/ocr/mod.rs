// OCR-текст картинок (PaddleOCR v4 mobile ONNX): mod.rs — только склейка;
// инференс в inference.rs, воркер индексации в index.rs. Без
// ~/.config/luma/models/ocr/ фича молча не работает.

mod inference;

pub(crate) use inference::{model_path, models_dir, OcrModels};
