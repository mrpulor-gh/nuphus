//! VAD segmentation pipeline shared by the mic session worker and the
//! `stt_recognize_file` regression command.
//!
//! Design (report-live.md §3, product-layer fixes):
//! - Pre-roll lookback (300ms): silero VAD reports segment start *at detected
//!   speech onset*, clipping leading phonemes (two「把」-initial sentences
//!   were decapitated in the prototype). We keep an absolute-indexed history
//!   and decode from `segment.start - PREROLL`.
//! - Partial results: while inside speech, re-decode the in-progress
//!   utterance every `PARTIAL_INTERVAL` so the UI gets live feedback
//!   (RTF 0.030 makes this cheap on CPU).
//! - All recognized text passes the tech-dict post-correction.

use super::dict;
use super::engine::{Recognizer, Segment, Vad, SAMPLE_RATE};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Pre-roll lookback before VAD segment start (samples). 300ms @16k.
pub const PREROLL_SAMPLES: usize = SAMPLE_RATE / 1000 * 300;

/// Minimum in-speech audio before emitting partials (samples). 800ms @16k.
const PARTIAL_MIN_SAMPLES: usize = SAMPLE_RATE / 1000 * 800;

/// Wall-clock interval between partial decodes.
const PARTIAL_INTERVAL: Duration = Duration::from_millis(800);

/// History retention: 30s of audio is enough for pre-roll + in-flight segment
/// (VAD max_speech_duration is 20s).
const HISTORY_KEEP: usize = SAMPLE_RATE * 30;

/// A finalized utterance ready for the UI.
pub struct FinalSegment {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Sink for pipeline output. The mic worker emits Tauri events; the
/// regression command collects into a Vec.
pub trait Sink {
    fn on_partial(&mut self, text: &str);
    fn on_final(&mut self, seg: FinalSegment);
}

pub struct Pipeline {
    vad: Vad,
    recognizer: Arc<Recognizer>,
    /// Absolute-indexed sample history; `history[0]` == sample `base`.
    history: Vec<f32>,
    base: usize,
    /// Total samples fed so far (absolute VAD timeline).
    fed: usize,
    in_speech: bool,
    /// Approx absolute index where current speech started.
    speech_start: usize,
    last_partial: Instant,
}

impl Pipeline {
    pub fn new(vad: Vad, recognizer: Arc<Recognizer>) -> Self {
        Self {
            vad,
            recognizer,
            history: Vec::with_capacity(HISTORY_KEEP),
            base: 0,
            fed: 0,
            in_speech: false,
            speech_start: 0,
            last_partial: Instant::now(),
        }
    }

    /// Feed one chunk of 16kHz mono f32 audio; decode completed segments.
    pub fn feed(&mut self, chunk: &[f32], sink: &mut dyn Sink) {
        if chunk.is_empty() {
            return;
        }
        self.vad.accept_waveform(chunk);
        self.history.extend_from_slice(chunk);
        self.fed += chunk.len();

        let detected = self.vad.detected();
        if detected && !self.in_speech {
            // Speech onset: back off one VAD window from "now".
            self.speech_start = self.fed.saturating_sub(512);
            self.last_partial = Instant::now();
        }
        self.in_speech = detected;

        self.drain_segments(sink);
        self.maybe_partial(sink);
        self.trim_history();
    }

    /// End of input: flush VAD tail and decode remaining segments.
    pub fn finish(&mut self, sink: &mut dyn Sink) {
        self.vad.flush();
        self.drain_segments(sink);
        self.in_speech = false;
    }

    /// Abort: drop all pending segments without decoding.
    pub fn abort(&mut self) {
        self.vad.clear();
        self.in_speech = false;
    }

    fn drain_segments(&mut self, sink: &mut dyn Sink) {
        while !self.vad.is_empty() {
            let Some(seg) = self.vad.take_front() else {
                break;
            };
            if let Some(f) = self.decode_segment(&seg) {
                sink.on_final(f);
            }
        }
    }

    /// Decode one completed VAD segment with pre-roll extension.
    fn decode_segment(&self, seg: &Segment) -> Option<FinalSegment> {
        let seg_end = seg.start + seg.samples.len();
        let start = seg.start.saturating_sub(PREROLL_SAMPLES).max(self.base);
        let audio = self.slice(start, seg_end);
        if audio.is_empty() {
            return None;
        }
        let text = dict::correct(&self.recognizer.recognize(audio));
        if text.trim().is_empty() {
            return None;
        }
        Some(FinalSegment {
            text,
            start_ms: (start * 1000 / SAMPLE_RATE) as i64,
            end_ms: (seg_end * 1000 / SAMPLE_RATE) as i64,
        })
    }

    /// Live partial of the in-progress utterance (never dict-corrected twice —
    /// correction is idempotent for exact matches, and fuzzy hits are stable).
    fn maybe_partial(&mut self, sink: &mut dyn Sink) {
        if !self.in_speech {
            return;
        }
        if self.fed.saturating_sub(self.speech_start) < PARTIAL_MIN_SAMPLES {
            return;
        }
        if self.last_partial.elapsed() < PARTIAL_INTERVAL {
            return;
        }
        self.last_partial = Instant::now();
        let start = self
            .speech_start
            .saturating_sub(PREROLL_SAMPLES)
            .max(self.base);
        let audio = self.slice(start, self.fed);
        if audio.is_empty() {
            return;
        }
        let text = dict::correct(&self.recognizer.recognize(audio));
        sink.on_partial(&text);
    }

    /// Absolute-indexed slice of history; clamps to retained range.
    fn slice(&self, start: usize, end: usize) -> &[f32] {
        let s = start.max(self.base).min(self.fed);
        let e = end.min(self.fed).max(s);
        &self.history[s - self.base..e - self.base]
    }

    fn trim_history(&mut self) {
        // Never drop audio that could belong to the in-flight segment.
        let keep_from = if self.in_speech {
            self.speech_start.saturating_sub(PREROLL_SAMPLES)
        } else {
            self.fed.saturating_sub(HISTORY_KEEP)
        };
        let target_base = (self.fed.saturating_sub(HISTORY_KEEP)).min(keep_from);
        if target_base > self.base {
            let drop = target_base - self.base;
            self.history.drain(..drop);
            self.base = target_base;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preroll_constant() {
        assert_eq!(PREROLL_SAMPLES, 4800);
    }
}
