// CLIP-эмбеддинги картинок (ViT-B/32 int8 ONNX) для будущего векторного поиска.
// mod.rs — только склейка; preload/инференс в inference.rs, воркер индексации
// в index.rs. Без ~/.config/luma/models/clip/ фича молча не работает.

mod index;
mod inference;

pub use index::{
    clip_ready, clip_search, is_stale, next_query, NEVER_STALE, CLIP_DONE, DL_DONE, DL_TOTAL, EMB_DONE, EMB_TOTAL, MATCH_NAME,
    MATCH_SEMANTIC, ONNX_LOCK, schedule_reindex, start_indexing,
};
