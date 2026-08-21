//! Preload commands — warm up models and resources at startup

use crate::state::AppState;
use tauri::{AppHandle, State};

/// Preload the Candle embedding model (bge-small-zh) at startup.
/// Calls Embedder::get() which triggers lazy init on first call.
/// Returns true once loaded (or if already loaded).
#[tauri::command]
pub fn preload_model(_state: State<'_, AppState>) -> Result<bool, String> {
    tracing::info!("[Preload] Starting embedding model load...");

    // Try loading the model via Embedder::get() which creates on first call
    match nuphus::embed::Embedder::get() {
        Some(_) => {
            tracing::info!("[Preload] Embedding model loaded successfully");
            Ok(true)
        }
        None => {
            tracing::warn!(
                "[Preload] Embedding model failed to load (will lazy-init on first use)"
            );
            Ok(false)
        }
    }
}

/// Ensure vision models (PaddleOCR + YOLO) are present, kicking off a
/// background download if any file is missing. Non-blocking: returns true
/// immediately while the download proceeds on a worker thread.
///
/// Also serves as the ModelsPage "retry" entry point — the frontend invokes
/// this with no arguments; `AppHandle` is injected by Tauri.
#[tauri::command]
pub fn preload_ocr(app: AppHandle) -> Result<bool, String> {
    crate::models::bootstrap::ensure_vision_models(&app)?;
    Ok(true)
}
