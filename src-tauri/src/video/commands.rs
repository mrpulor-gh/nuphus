//! Tauri command surface + nuphus-lib tool bridge for the video pipeline.
//!
//! Contract:
//! - Command: `video_extract_subtitles(input)` → `VideoSubtitleResult`
//!   (`{source, title?, duration_ms?, cues, truncated}`); Err is an actionable
//!   message (never fabricated subtitles).
//! - Events: `video:progress { stage, percent?, message }` — see pipeline.rs
//!   for stage vocabulary; "done"/"error" are terminal exactly once per run.
//! - Tool bridge: `nuphus::video_bridge` fn-pointer injection (single
//!   process, no IPC), registered at app setup so the Agent tool
//!   `video_subtitle_extract` can reach this pipeline.

use super::pipeline::{self, VideoSubtitleResult};
use std::sync::Arc;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;

/// AppHandle for the bridge path (tool executor has no Tauri State access).
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Called once from main.rs setup: stores the AppHandle and registers the
/// bridge implementation into the nuphus lib.
pub fn init_bridge(app: &AppHandle) {
    let _ = APP.set(app.clone());
    nuphus::video_bridge::register_video_extract_impl(bridge_extract);
    tracing::info!("[video] tool bridge registered");
}

/// Bridge entry: runs the pipeline and serializes the result to JSON
/// (the nuphus lib renders it into "[mm:ss] text" lines for the LLM).
fn bridge_extract(input: &str) -> Result<String, String> {
    let app = APP
        .get()
        .ok_or_else(|| "视频字幕管线未初始化（桌面壳未注册桥接）".to_string())?;
    let cache = Arc::clone(&app.state::<AppState>().speech.cache);
    let result = pipeline::run(app, &cache, input)?;
    serde_json::to_string(&result).map_err(|e| format!("序列化字幕结果失败: {e}"))
}

#[tauri::command]
pub async fn video_extract_subtitles(
    input: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<VideoSubtitleResult, String> {
    let cache = Arc::clone(&state.speech.cache);
    tauri::async_runtime::spawn_blocking(move || pipeline::run(&app, &cache, &input))
        .await
        .map_err(|e| format!("video_extract_subtitles task failed: {e}"))?
}
