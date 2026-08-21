//! Video subtitle extraction pipeline (background service + tool interface).
//!
//! Three-tier degradation (OCR hard-subtitles intentionally out of scope):
//!   1. embedded  — local file with text subtitle stream → ffmpeg export srt
//!   2. platform  — online URL with official/auto captions → yt-dlp subs only
//!   3. asr       — no subtitles → audio extract → local SenseVoice ASR
//!
//! Modules:
//! - `deps`     — yt-dlp / ffmpeg / ffprobe probe chain + auto-download
//! - `subtitle` — SRT/VTT parsing + normalization into `Cue`
//! - `pipeline` — orchestration (sequential: download completes, then ASR)
//! - `commands` — Tauri command + nuphus-lib tool bridge injection

pub mod commands;
pub mod deps;
pub mod pipeline;
pub mod subtitle;
