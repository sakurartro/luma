// Splits 16 kHz mono audio into speech chunks at pauses, so silence never
// reaches the model and no chunk is longer than MAX_CHUNK (Moonshine's limit
// is ~64 s, 25 s keeps quality and memory in check).
// shortcut: one fixed energy threshold, no noise-floor tracking; music or
// loud background noise counts as speech, upgrade to a real VAD if it hurts.

pub const RATE: usize = 16_000;
const WIN: usize = RATE / 50; // 20 ms judged at a time
const SILENCE_RMS: f32 = 0.01; // about -40 dBFS
const PAUSE_WINS: usize = 25; // 0.5 s of silence ends a chunk...
const MIN_CHUNK: usize = 2 * RATE; // ...once it is at least this long
const MAX_CHUNK: usize = 25 * RATE;
const CUT_FROM: usize = 15 * RATE; // a forced cut goes to the quietest window after this
const LEAD: usize = RATE / 10; // silence kept before the first word
const TAIL: usize = RATE / 5; // and after the last one
const MIN_VOICED: usize = RATE / 4; // shorter blips are dropped

pub struct Chunk {
    /// Start in the video, seconds.
    pub start_sec: f64,
    pub samples: Vec<f32>,
}

#[derive(Default)]
pub struct Segmenter {
    pending: Vec<f32>, // less than one window, not judged yet
    pos: usize,        // samples turned into windows so far
    lead: Vec<f32>,    // silence before the chunk that has not started
    buf: Vec<f32>,     // the chunk in progress
    rms: Vec<f32>,     // one per window of buf
    start: usize,      // sample index of buf[0]
    silence_run: usize,
}

impl Segmenter {
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<Chunk>) {
        self.pending.extend_from_slice(samples);
        let whole = self.pending.len() / WIN * WIN;
        let windows: Vec<f32> = self.pending.drain(..whole).collect();
        for w in windows.chunks(WIN) {
            self.window(w, out);
        }
    }

    /// The last chunk, if it holds speech.
    pub fn finish(mut self, out: &mut Vec<Chunk>) {
        self.pause_cut(out);
    }

    fn window(&mut self, w: &[f32], out: &mut Vec<Chunk>) {
        let rms = (w.iter().map(|x| x * x).sum::<f32>() / w.len() as f32).sqrt();
        let silent = rms < SILENCE_RMS;
        let at = self.pos;
        self.pos += w.len();
        if self.buf.is_empty() {
            if silent {
                self.lead.extend_from_slice(w);
                self.lead.drain(..self.lead.len().saturating_sub(LEAD));
                return;
            }
            self.start = at - self.lead.len();
            self.buf = std::mem::take(&mut self.lead);
            self.rms = vec![0.0; self.buf.len() / WIN];
            self.silence_run = 0;
        }
        self.buf.extend_from_slice(w);
        self.rms.push(rms);
        self.silence_run = if silent { self.silence_run + 1 } else { 0 };
        if self.silence_run >= PAUSE_WINS && self.buf.len() >= MIN_CHUNK {
            self.pause_cut(out);
        } else if self.buf.len() >= MAX_CHUNK {
            self.forced_cut(out);
        }
    }

    // Emits buf up to its last speech plus TAIL (if there was any speech) and
    // starts over; the silence after it only feeds `lead`.
    fn pause_cut(&mut self, out: &mut Vec<Chunk>) {
        let len = self.buf.len();
        let voiced = len.saturating_sub(self.silence_run * WIN);
        if len > 0 && voiced >= MIN_VOICED {
            let end = (voiced + TAIL).min(len);
            out.push(Chunk { start_sec: self.start as f64 / RATE as f64, samples: self.buf[..end].to_vec() });
        }
        self.lead = self.buf.split_off(len.saturating_sub(LEAD));
        self.buf.clear();
        self.rms.clear();
    }

    // Too long without a pause: cut at the quietest window of the last 10 s.
    fn forced_cut(&mut self, out: &mut Vec<Chunk>) {
        let from = CUT_FROM / WIN;
        let quietest = (from..self.rms.len()).min_by(|&a, &b| self.rms[a].total_cmp(&self.rms[b])).unwrap_or(from);
        let end = quietest * WIN;
        let rest = self.buf.split_off(end);
        out.push(Chunk { start_sec: self.start as f64 / RATE as f64, samples: std::mem::replace(&mut self.buf, rest) });
        self.rms.drain(..quietest);
        self.start += end;
        self.silence_run = self.rms.iter().rev().take_while(|&&r| r < SILENCE_RMS).count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(sec: f32) -> Vec<f32> {
        (0..(sec * RATE as f32) as usize).map(|i| 0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin()).collect()
    }

    fn silence(sec: f32) -> Vec<f32> {
        vec![0.0; (sec * RATE as f32) as usize]
    }

    fn run(audio: &[f32], piece: usize) -> Vec<Chunk> {
        let (mut seg, mut out) = (Segmenter::default(), Vec::new());
        for p in audio.chunks(piece) {
            seg.push(p, &mut out);
        }
        seg.finish(&mut out);
        out
    }

    fn secs(c: &Chunk) -> f64 {
        c.samples.len() as f64 / RATE as f64
    }

    #[test]
    fn tone_silence_tone_gives_two_chunks() {
        let audio = [silence(1.0), tone(3.0), silence(2.0), tone(3.0), silence(1.0)].concat();
        let chunks = run(&audio, 4096);
        assert_eq!(chunks.len(), 2);
        // starts 0.1 s (LEAD) before the tone, ends 0.2 s (TAIL) after it
        assert!((chunks[0].start_sec - 0.9).abs() < 0.05, "{}", chunks[0].start_sec);
        assert!((secs(&chunks[0]) - 3.3).abs() < 0.05, "{}", secs(&chunks[0]));
        assert!((chunks[1].start_sec - 5.9).abs() < 0.05, "{}", chunks[1].start_sec);
        assert!((secs(&chunks[1]) - 3.3).abs() < 0.05, "{}", secs(&chunks[1]));
    }

    #[test]
    fn silence_and_blips_give_nothing() {
        assert!(run(&silence(30.0), 4096).is_empty());
        let blip = [silence(1.0), tone(0.1), silence(2.0)].concat();
        assert!(run(&blip, 4096).is_empty());
    }

    #[test]
    fn short_pause_does_not_split_a_short_chunk() {
        // 1 s tone, 0.6 s pause, 1 s tone: the chunk is under MIN_CHUNK at the pause
        let audio = [tone(1.0), silence(0.6), tone(1.0)].concat();
        assert_eq!(run(&audio, 4096).len(), 1);
    }

    #[test]
    fn no_chunk_exceeds_the_limit_and_nothing_is_lost() {
        let audio = tone(70.0);
        let chunks = run(&audio, 4096);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.samples.len() <= MAX_CHUNK));
        // chunks are contiguous: each starts where the previous ended
        for pair in chunks.windows(2) {
            assert!((pair[0].start_sec + secs(&pair[0]) - pair[1].start_sec).abs() < 1e-6);
        }
        let total: usize = chunks.iter().map(|c| c.samples.len()).sum();
        assert!(total >= audio.len() - WIN, "{total}");
    }

    #[test]
    fn piece_size_does_not_matter() {
        let audio = [silence(1.0), tone(3.0), silence(2.0), tone(30.0)].concat();
        let key = |cs: Vec<Chunk>| cs.iter().map(|c| (c.start_sec.to_bits(), c.samples.len())).collect::<Vec<_>>();
        assert_eq!(key(run(&audio, 1000)), key(run(&audio, 100_000)));
    }
}
