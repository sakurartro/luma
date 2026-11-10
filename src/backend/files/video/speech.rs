// Speech recognition of video audio: Moonshine base (English, int8 ONNX).
// The model takes raw 16 kHz mono audio, no log-mel step.

use super::vad::{Chunk, Segmenter, RATE};
use crate::backend::files::download::ensure_models;
use anyhow::{anyhow, bail, Context, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::time::Duration;
use tokenizers::Tokenizer;

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

// Decoder geometry (config.json): 8 layers, 8 heads of 52.
const LAYERS: usize = 8;
const HEADS: i64 = 8;
const HEAD_DIM: i64 = 52;
const BOS: i64 = 1;
const EOS: i64 = 2;
const VOCAB: usize = 32768;
// Speech is ~3-4 tokens/s; twice that bounds a repetition loop.
const MAX_TOKENS_PER_SEC: f64 = 8.0;
// ffmpeg that produces nothing for this long is hung on a broken file.
const STALL: Duration = Duration::from_secs(60);

pub struct SpeechModels {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
}

impl SpeechModels {
    pub fn load() -> Option<Self> {
        Self::load_from(&models_dir())
    }

    pub(crate) fn load_from(dir: &Path) -> Option<Self> {
        let Some(env) = crate::backend::files::ort_env() else {
            eprintln!("speech disabled: ort init failed");
            return None;
        };
        // memory_pattern off: input sizes change from chunk to chunk.
        let build = |name: &str| {
            Session::builder(&env)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .unwrap_or_else(|e| e.recover())
                .with_memory_pattern(false)?
                .with_intra_threads(2)?
                .commit_from_file(dir.join(name))
        };
        let (Ok(encoder), Ok(decoder), Ok(tokenizer)) =
            (build("encoder.onnx"), build("decoder.onnx"), Tokenizer::from_file(dir.join("tokenizer.json")))
        else {
            eprintln!("speech disabled: models/moonshine/ is missing or broken");
            return None;
        };
        Some(Self { encoder, decoder, tokenizer })
    }

    /// Text of a 16 kHz mono chunk: encoder once, then greedy decoding with
    /// the decoder's KV cache.
    pub(crate) fn recognize(&mut self, samples: &[f32]) -> Result<String> {
        let input = Tensor::from_array(([1i64, samples.len() as i64], samples.to_vec()))?;
        let mut enc_out = self.encoder.run(ort::inputs!["input_values" => input])?;
        let hidden = enc_out.remove("last_hidden_state").ok_or_else(|| anyhow!("encoder: no last_hidden_state"))?;
        drop(enc_out);

        // (past input name, present output name) for every cache tensor
        let names: Vec<(String, String)> = (0..LAYERS)
            .flat_map(|l| ["decoder", "encoder"].into_iter().map(move |side| (l, side)))
            .flat_map(|(l, side)| ["key", "value"].into_iter().map(move |kv| (l, side, kv)))
            .map(|(l, side, kv)| (format!("past_key_values.{l}.{side}.{kv}"), format!("present.{l}.{side}.{kv}")))
            .collect();
        let empty = || -> Result<DynValue> {
            Ok(Tensor::from_array(([1i64, HEADS, 0, HEAD_DIM], Vec::<f32>::new()))?.into_dyn())
        };
        let mut cache: Vec<DynValue> = names.iter().map(|_| empty()).collect::<Result<_>>()?;

        let max_tokens = (samples.len() as f64 / RATE as f64 * MAX_TOKENS_PER_SEC).ceil() as usize + 4;
        let mut tokens: Vec<u32> = Vec::new();
        let mut last = BOS;
        for step in 0..max_tokens {
            let use_cache = step > 0;
            let mut inputs: Vec<(String, SessionInputValue)> = vec![
                ("input_ids".into(), Tensor::from_array(([1i64, 1], vec![last]))?.into()),
                ("encoder_hidden_states".into(), (&hidden).into()),
                ("use_cache_branch".into(), Tensor::from_array(([1i64], vec![use_cache]))?.into()),
            ];
            inputs.extend(names.iter().zip(&cache).map(|((past, _), value)| (past.clone(), value.into())));
            let mut out = self.decoder.run(inputs)?;

            let (shape, logits) = out["logits"].try_extract_tensor::<f32>()?;
            let at = (shape[1] as usize - 1) * VOCAB;
            let next = logits[at..at + VOCAB]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i as i64)
                .ok_or_else(|| anyhow!("decoder: empty logits"))?;
            if next == EOS {
                break;
            }
            tokens.push(next as u32);
            last = next;

            // First step fills the whole cache; later the encoder part never
            // changes (its present outputs are empty), only the decoder part grows.
            for ((_, present), slot) in names.iter().zip(&mut cache) {
                if !use_cache || present.contains(".decoder.") {
                    *slot = out.remove(present).ok_or_else(|| anyhow!("decoder: no {present}"))?;
                }
            }
        }
        let text = self.tokenizer.decode(&tokens, true).map_err(|e| anyhow!("tokenizer: {e}"))?;
        Ok(text.trim().to_string())
    }

    /// Speech of a video as (start second, text) segments, audio streamed
    /// from ffmpeg so an hour never sits in memory.
    pub(crate) fn transcribe(&mut self, path: &Path) -> Result<Vec<(f64, String)>> {
        let mut segments = Vec::new();
        for_each_chunk(path, |chunk| {
            let text = self.recognize(&chunk.samples)?;
            if !text.is_empty() {
                segments.push((chunk.start_sec, text));
            }
            Ok(())
        })?;
        Ok(segments)
    }
}

struct Ffmpeg(Child);

impl Drop for Ffmpeg {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs `f` on every speech chunk of the file's audio, in order. Errors when
/// ffmpeg can't read the file or hangs; a file that breaks halfway keeps
/// what was read.
pub(crate) fn for_each_chunk(path: &Path, mut f: impl FnMut(Chunk) -> Result<()>) -> Result<()> {
    let mut child = Ffmpeg(
        Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("run ffmpeg")?,
    );
    let mut stdout = child.0.stdout.take().context("ffmpeg stdout")?;
    // A thread does the blocking reads so the loop below can time out on a
    // silent ffmpeg; the bounded channel keeps ffmpeg from running ahead of
    // the model.
    let (tx, rx) = sync_channel::<Vec<f32>>(4);
    std::thread::spawn(move || {
        let mut bytes = [0u8; 64 * 1024];
        let mut carry = 0; // bytes of a split sample left from the last read
        loop {
            let n = match stdout.read(&mut bytes[carry..]) {
                Ok(0) | Err(_) => return,
                Ok(n) => carry + n,
            };
            let whole = n / 4 * 4;
            let samples = bytes[..whole].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            bytes.copy_within(whole..n, 0);
            carry = n - whole;
            if tx.send(samples).is_err() {
                return;
            }
        }
    });

    let (mut seg, mut chunks, mut total) = (Segmenter::default(), Vec::new(), 0usize);
    loop {
        match rx.recv_timeout(STALL) {
            Ok(samples) => {
                total += samples.len();
                seg.push(&samples, &mut chunks);
                for chunk in chunks.drain(..) {
                    f(chunk)?;
                }
            }
            Err(RecvTimeoutError::Timeout) => bail!("ffmpeg stalled on {}", path.display()),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    seg.finish(&mut chunks);
    for chunk in chunks {
        f(chunk)?;
    }
    if total == 0 && !child.0.wait()?.success() {
        bail!("ffmpeg could not read audio of {}", path.display());
    }
    Ok(())
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

    #[test]
    fn chunks_from_generated_audio() {
        if !crate::backend::files::video::frames::ffmpeg_available() {
            eprintln!("skip: no ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("luma_speech_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("tone.wav");
        // 8 s at 44.1 kHz stereo: a 440 Hz tone for 3 s, then 1 s of silence, twice
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("aevalsrc=if(lt(mod(t\\,4)\\,3)\\,0.3*sin(2*PI*440*t)\\,0)|if(lt(mod(t\\,4)\\,3)\\,0.3*sin(2*PI*440*t)\\,0):s=44100:d=8")
            .arg(&audio)
            .status()
            .unwrap();
        assert!(status.success());

        let mut chunks = Vec::new();
        for_each_chunk(&audio, |c| {
            chunks.push((c.start_sec, c.samples.len() as f64 / RATE as f64));
            Ok(())
        })
        .unwrap();
        assert_eq!(chunks.len(), 2, "{chunks:?}");
        assert!(chunks[0].0 < 0.1 && (chunks[0].1 - 3.2).abs() < 0.15, "{chunks:?}");
        assert!((chunks[1].0 - 3.9).abs() < 0.15 && (chunks[1].1 - 3.3).abs() < 0.15, "{chunks:?}");

        // not audio: an error, no panic
        let fake = dir.join("fake.wav");
        std::fs::write(&fake, "not audio").unwrap();
        assert!(for_each_chunk(&fake, |_| Ok(())).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Real model on real speech (downloads ~66 MB); the CI job `speech` runs it:
    //   SPEECH_AUDIO=speech.wav cargo test speech_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn speech_live() {
        let audio = std::env::var("SPEECH_AUDIO").unwrap();
        assert!(ensure_model(), "model download failed");
        let mut models = SpeechModels::load().expect("models do not load");
        let start = std::time::Instant::now();
        let segments = models.transcribe(Path::new(&audio)).unwrap();
        println!("{segments:?} in {:?}", start.elapsed());
        let text = segments.iter().map(|(_, t)| t.to_lowercase()).collect::<Vec<_>>().join(" ");
        let found = ["quick", "brown", "fox", "jumps", "lazy", "dog"].iter().filter(|w| text.contains(**w)).count();
        assert!(found >= 4, "recognized: {text:?}");
    }
}
