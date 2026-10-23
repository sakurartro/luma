// OCR-текст картинок (PaddleOCR v4 mobile ONNX): mod.rs — только склейка;
// preprocess.rs — входы моделей, inference.rs — прогон ort, postprocess.rs —
// боксы и CTC-декод, воркер индексации и скачивание моделей — в index.rs, поиск — в search.rs.

mod index;
mod inference;
mod postprocess;
mod preprocess;
mod search;

pub use index::{start_indexing, OCR_DL_DONE, OCR_DL_TOTAL, OCR_DONE, OCR_TOTAL};
pub use search::{ocr_search, MATCH_OCR};
