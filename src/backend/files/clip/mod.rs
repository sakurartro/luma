// CLIP-эмбеддинги картинок (ViT-B/32 int8 ONNX) для будущего векторного поиска.
// mod.rs — только склейка; preload/инференс в inference.rs, воркер индексации
// в index.rs. Без ~/.config/luma/models/clip/ фича молча не работает.

mod index;
mod inference;

pub use index::start_indexing;
