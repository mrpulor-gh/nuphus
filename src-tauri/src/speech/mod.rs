//! Speech-to-text: two engines behind one event contract —
//! cloud (OpenAI-compatible /audio/transcriptions, cloud-first when
//! `capabilities.stt` resolves, see cloud.rs) and local sherpa-onnx
//! (FunASR-Nano, sense-voice int8) offline ASR with silero VAD endpointing,
//! cpal mic capture, and tech-dict correction.
//!
//! Architecture (local path mirrors the validated prototype CLI chain
//! `sherpa-onnx-vad-with-offline-asr`, implemented in-process):
//!
//! ```text
//! cpal mic (native rate) → mono downmix → Resampler → 16kHz f32
//!   → Pipeline: silero VAD (+300ms pre-roll lookback, live partials)
//!   → OfflineRecognizer (sense-voice, language=zh, greedy_search)
//!   → dict::correct (closed tech vocabulary, edit-distance ≤2)
//!   → Tauri events: stt:partial / stt:final / stt:error
//! ```
//!
//! See `commands` for the full command/event contract.

pub mod cloud;
pub mod commands;
pub mod dict;
pub mod download;
pub mod engine;
pub mod ffi;
pub mod mic;
pub mod pipeline;
pub mod wav;

pub use commands::SpeechState;
