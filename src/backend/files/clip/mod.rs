// CLIP-эмбеддинги картинок (ViT-B/32 int8 ONNX) для будущего векторного поиска.
// mod.rs — только склейка; preload/инференс в inference.rs, воркер индексации
// в index.rs. Без ~/.config/luma/models/clip/ фича молча не работает.

mod clipboard;
mod index;
mod inference;

pub(crate) use inference::open_image;

pub use clipboard::read_image;
pub use index::{
    clip_ready, clip_search, clip_similar, is_stale, next_query, NEVER_STALE, CLIP_DONE, DL_DONE, DL_TOTAL, EMB_DONE, EMB_TOTAL, MATCH_NAME,
    MATCH_SEMANTIC, ONNX_LOCK, schedule_reindex, start_indexing,
};
