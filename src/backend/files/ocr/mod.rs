// OCR-текст картинок (PaddleOCR v4 mobile ONNX): mod.rs — только склейка;
// preprocess.rs — входы моделей, inference.rs — прогон ort, postprocess.rs —
// боксы и CTC-декод, воркер индексации и скачивание моделей — в index.rs, поиск — в search.rs.

mod index;
mod inference;
mod postprocess;
mod preprocess;
mod search;
mod video;

pub(super) use index::pending_models;
pub use index::{schedule_reindex, start_indexing, OCR_DONE, OCR_PASS_DONE, OCR_TOTAL};
pub use search::{ocr_ready, ocr_search, MATCH_OCR};
