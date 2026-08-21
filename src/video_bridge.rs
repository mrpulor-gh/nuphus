//! video_bridge — Injection point for the desktop shell's video subtitle
//! pipeline.
//!
//! The nuphus lib is Tauri-agnostic, and builtin tool executors are bare
//! `fn` pointers that cannot capture state. The video pipeline lives in the
//! desktop shell (src-tauri/src/video/, reusing the resident STT engine),
//! so the shell registers a plain fn here at startup (same process, direct
//! call — no IPC). Pattern mirrors `ScheduleExecCallback` injection.
//!
//! Contract: the registered fn takes the raw tool `input` (URL or local
//! path) and returns the `VideoSubtitleResult` JSON
//! (`{source, title?, duration_ms?, cues:[{start_ms,end_ms,text}], truncated}`).

use std::sync::OnceLock;

/// Bridge implementation signature: input → VideoSubtitleResult JSON.
pub type VideoExtractImpl = fn(&str) -> Result<String, String>;

static IMPL: OnceLock<VideoExtractImpl> = OnceLock::new();

/// Called once by the desktop shell at startup. Idempotent (first wins).
pub fn register_video_extract_impl(f: VideoExtractImpl) {
    let _ = IMPL.set(f);
}

/// True when the desktop shell has registered the pipeline.
pub fn is_available() -> bool {
    IMPL.get().is_some()
}

/// Invoke the pipeline. Err when the shell never registered (e.g. headless
/// builds) — the tool surfaces this as an honest failure, never fabricated.
pub fn extract(input: &str) -> Result<String, String> {
    match IMPL.get() {
        Some(f) => f(input),
        None => Err("视频字幕管线不可用（桌面壳未注册 video bridge）".to_string()),
    }
}
