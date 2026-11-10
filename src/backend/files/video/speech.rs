// Speech recognition of video audio: Moonshine base (English, int8 ONNX).
// The model takes raw 16 kHz mono audio, no log-mel step.

use crate::backend::files::download::ensure_models;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

// Model download progress in bytes; SPEECH_DL_TOTAL 0 = no download in progress.
pub static SPEECH_DL_TOTAL: AtomicUsize = AtomicUsize::new(0);
pub static SPEECH_DL_DONE: AtomicUsize = AtomicUsize::new(0);

// Models live in $XDG_CONFIG_HOME/luma/models/moonshine/, next to clip and ocr.
pub(crate) fn models_dir() -> PathBuf {
    crate::backend::apps::db_init::config_dir().join("models/moonshine")
}

// onnx-community/moonshine-base-ONNX (~66 MB): encoder, merged decoder with KV
// cache and the tokenizer. Local names are fixed.
// Pinned to a repo commit + sha256: a changed upstream file must not load.
//
// Checked against the files (onnx 1.x): encoder takes `input_values`
// [batch, samples] f32, gives `last_hidden_state` [batch, frames, 416];
// decoder takes `input_ids` i64, `encoder_hidden_states`, `use_cache_branch`
// [1] bool and 32 `past_key_values.{0..7}.{decoder,encoder}.{key,value}`
// [batch, 8, seq, 52], gives `logits` [batch, seq, 32768] and the 32 `present.*`.
// bos = 1, eos = 2 (config.json).
const MODEL_FILES: [(&str, &str, &str); 3] = [
    (
        "encoder.onnx",
        "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/b1e9b6aae3c3c7298f10c3798393fdf38e8fbbad/onnx/encoder_model_int8.onnx",
        "4edb38db96b52ddee5a3b25d2211bb6394f9a9a2d95f67bce0e8c861da018a4d",
    ),
    (
        "decoder.onnx",
        "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/b1e9b6aae3c3c7298f10c3798393fdf38e8fbbad/onnx/decoder_model_merged_int8.onnx",
        "45febab0347ffbe1326459e092ae20a2cfda10ca33ab018b3c299247284de61b",
    ),
    (
        "tokenizer.json",
        "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/b1e9b6aae3c3c7298f10c3798393fdf38e8fbbad/tokenizer.json",
        "7b913404bdd039af4756783218af4440bc07fb7d6d8258d677e34f95b3ec416f",
    ),
];

/// Downloads missing or corrupted model files; false = speech stays off this session.
pub fn ensure_model() -> bool {
    ensure_models("moonshine", &models_dir(), &MODEL_FILES, &SPEECH_DL_TOTAL, &SPEECH_DL_DONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_files_are_pinned() {
        for (name, url, sha) in MODEL_FILES {
            assert!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()), "{name}: sha256");
            let commit = url.split("/resolve/").nth(1).and_then(|r| r.split('/').next()).unwrap();
            assert!(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()), "{name}: commit");
        }
    }
}
