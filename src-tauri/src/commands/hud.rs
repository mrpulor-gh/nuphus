//! HUD overlay window management
//!
//! Creates and manages a minimal always-on-top overlay
//! that shows execution progress text without stealing focus.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tracing;

const HUD_LABEL: &str = "hud";
const HUD_WIDTH: f64 = 300.0;
const HUD_HEIGHT: f64 = 58.0;

/// Create the HUD window (initialized hidden)
pub fn create<R: tauri::Runtime>(app: &AppHandle<R>) {
    match WebviewWindowBuilder::new(app, HUD_LABEL, WebviewUrl::App("hud.html".into()))
        .title("")
        .inner_size(HUD_WIDTH, HUD_HEIGHT)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .resizable(false)
        .focused(false)
        .build()
    {
        Ok(_) => {
            tracing::info!("[HUD] Window created (hidden)");
            // Position bottom-right after creation
            if let Some(window) = app.get_webview_window(HUD_LABEL) {
                position_bottom_right(&window);
            }
        }
        Err(e) => {
            tracing::warn!("[HUD] Failed to create window: {}", e);
        }
    }
}

/// Show HUD with given text and phase
pub fn show<R: tauri::Runtime>(app: &AppHandle<R>, text: &str, phase: &str) {
    // Get / recreate window if needed
    let window = match app.get_webview_window(HUD_LABEL) {
        Some(w) => w,
        None => {
            tracing::warn!("[HUD] Window missing, recreating...");
            create(app);
            return; // create() builds hidden; next HudUpdate will show it
        }
    };

    if phase == "hidden" || phase == "hide" {
        hide(app);
        return;
    }

    // Restore size and position (may have been moved off-screen by hide)
    let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
        HUD_WIDTH as u32,
        HUD_HEIGHT as u32,
    )));
    position_bottom_right(&window);

    // Emit update event to the frontend
    let _ = window.emit(
        "hud-update",
        serde_json::json!({ "text": text, "phase": phase }),
    );

    // Show if not already visible
    let _ = window.show();
    let _ = window.set_always_on_top(true);
}

/// Hide HUD — robust path with off-screen positioning to prevent
/// WebView2 transparent-window ghosting on Windows.
pub fn hide<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(HUD_LABEL) {
        // Move off-screen first to prevent transparent-layer ghosting,
        // then hide. On Windows, WebView2 compositing can leave a
        // visible artifact if the window is simply hidden in place.
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            -99999, -99999,
        )));
        match window.hide() {
            Ok(_) => tracing::debug!("[HUD] Hidden"),
            Err(e) => {
                tracing::warn!("[HUD] hide() failed: {}, force-closing", e);
                let _ = window.close();
                // Recreate so the window is available next time
                create(app);
            }
        }
    }
}

/// Position the HUD window at bottom-right of the screen (20px from right, 40px from bottom)
/// ⚠️ DPI 修复(2026-08-25)：旧实现把 monitor.size()(物理像素)除以 scale 得到逻辑坐标后，
/// 却用 Position::Physical 设置——Win11 高 DPI(125%~200%)下坐标被二次缩放 → 窗口落到
/// 屏幕中央。修复：坐标全程逻辑像素，set_position 用 LogicalPosition，Tauri 按当前 DPI
/// 自动转换，任何缩放比例都贴右下角。
fn position_bottom_right<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let size = monitor.size();
        let scale = monitor.scale_factor();
        let w = HUD_WIDTH;
        let h = HUD_HEIGHT;
        // 逻辑像素坐标：物理尺寸 / scale → 逻辑；右缘留 20、底部留 40（逻辑像素）
        let x = (size.width as f64 / scale) - w - 20.0;
        let y = (size.height as f64 / scale) - h - 40.0;
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            x, y,
        )));
    }
}

/// Tauri command: update HUD from frontend or agent
#[tauri::command]
pub fn hud_update(app: AppHandle, text: String, phase: String) {
    show(&app, &text, &phase);
}

/// Tauri command: hide HUD
#[tauri::command]
pub fn hud_hide(app: AppHandle) {
    hide(&app);
}

/// Tauri command: pause active workflow
#[tauri::command]
pub async fn hud_pause(state: tauri::State<'_, crate::state::AppState>) -> Result<(), String> {
    let wf_id = nuphus::workflow::hud_control::active_id(&state.signals)
        .ok_or_else(|| "No active workflow".to_string())?;
    let engine = state.workflow_engine.read().await;
    engine.executor.pause(&wf_id).await;
    Ok(())
}

/// Tauri command: resume active workflow
#[tauri::command]
pub async fn hud_resume(state: tauri::State<'_, crate::state::AppState>) -> Result<(), String> {
    let wf_id = nuphus::workflow::hud_control::active_id(&state.signals)
        .ok_or_else(|| "No active workflow".to_string())?;
    let engine = state.workflow_engine.read().await;
    engine.executor.resume(&wf_id).await;
    Ok(())
}

/// Tauri command: stop active workflow
#[tauri::command]
pub async fn hud_stop(state: tauri::State<'_, crate::state::AppState>) -> Result<(), String> {
    let wf_id = nuphus::workflow::hud_control::active_id(&state.signals)
        .ok_or_else(|| "No active workflow".to_string())?;
    let engine = state.workflow_engine.read().await;
    engine.executor.cancel(&wf_id).await;
    // Also resume if paused (so the executor can process the cancel)
    engine.executor.resume(&wf_id).await;
    nuphus::workflow::hud_control::mark_user_cancelled();
    Ok(())
}