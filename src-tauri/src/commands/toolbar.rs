// toolbar.rs → Main window show/hide / fullscreen overlay mask control
//
// Screenshot flow (pre-screenshot + static storage polling):
//   1. start_overlay_mask: pre-capture full screen → hide main window → create transparent overlay → return OK
//   2. Overlay mouse move calls overlay_magnifier_region → crop directly from pre-screenshot memory
//   3. User drags selection → overlay_capture_confirm(x,y,w,h) → crop from pre-screenshot + save
//   4. User confirms → overlay_capture_done: store CAPTURE_RESULT → restore main window → close overlay
//   5. Main window DesktopToolbar polls via take_capture_result to get result

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Global always-on-top state
static ALWAYS_ON_TOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether overlay window has been created (lazy init)
static OVERLAY_CREATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Pre-screenshot cache: (RGBA pixel data, width, height)
/// Captured before overlay creation in start_overlay_mask, usable directly during overlay lifetime
static PRE_SCREENSHOT: Mutex<Option<(Vec<u8>, u32, u32)>> = Mutex::new(None);

/// Screenshot result cache (for main window polling)
/// Written by overlay_capture_done/cancel, consumed and cleared by take_capture_result
static CAPTURE_RESULT: Mutex<Option<serde_json::Value>> = Mutex::new(None);

// ── Main window control ──

#[tauri::command]
pub async fn toggle_main_window_topmost(app: AppHandle) -> Result<bool, String> {
    let new_state = !ALWAYS_ON_TOP.load(std::sync::atomic::Ordering::SeqCst);
    if let Some(win) = app.get_webview_window("main") {
        win.set_always_on_top(new_state)
            .map_err(|e| format!("设置窗口置顶失败: {e}"))?;
    }
    ALWAYS_ON_TOP.store(new_state, std::sync::atomic::Ordering::SeqCst);
    Ok(new_state)
}

/// Called by frontend when initialization is complete.
/// Closes the splash window and shows the main window.
#[tauri::command]
pub async fn finish_startup(app: AppHandle) -> Result<(), String> {
    // Close splash window
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }
    // Show main window
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
    Ok(())
}

/// Update the splash window's loading status text.
/// Called from both Rust setup and frontend initialization stages.
/// 走 `splash:progress` 事件（不受 splash 页 CSP 限制；旧 eval+内联
/// setStatus 定义被 `script-src 'self'` 拦截，状态文案从未生效）。
#[tauri::command]
pub async fn splash_status_update(app: AppHandle, text: String) -> Result<(), String> {
    crate::splash::emit_splash_progress(&app, None, &text);
    Ok(())
}

/// 用户在 splash 上点击「后台下载」：关闭 splash、显示主窗口，让进行中的
/// 模型下载继续在后台跑（preload_model / preload_ocr 的下载在 spawn_blocking
/// 线程上，不受 splash 窗口生命周期影响；ModelsPage 经 `models:download` 事件
/// 继续展示进度）。同时广播 `splash:skipped`，主界面立即进入 ready。
#[tauri::command]
pub async fn splash_skip_download(app: AppHandle) -> Result<(), String> {
    tracing::info!("[Splash] user chose background download — closing splash early");
    if let Some(splash) = app.get_webview_window("splash") {
        let _ = splash.close();
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
    // 主界面提示：模型在后台继续下载（可在「设置-模型」查看进度）
    super::hud::hud_update(
        app.clone(),
        "模型正在后台下载…可在「设置-模型」查看进度".to_string(),
        "info".to_string(),
    );
    let _ = app.emit("splash:skipped", ());
    Ok(())
}

// ── Fullscreen overlay mask (pre-screenshot approach) ──

/// 主窗口隐藏确认轮询：30 次 × 10ms ≈ 300ms 上限（正常 1~2 次即确认）
const HIDE_POLL_ATTEMPTS: usize = 30;
const HIDE_POLL_INTERVAL_MS: u64 = 10;
/// DXGI 桌面合成刷帧等待（>2 个 vsync；再长会拖慢截图工具呼出速度）
const COMPOSITOR_SETTLE_MS: u64 = 150;

/// Start fullscreen overlay mask: hide main window → capture full screen → show overlay → return
/// Overlay window is created once at startup and reused (show/hide), eliminating white flash.
/// mode parameter is injected into overlay frontend via window.__setOverlayMode__().
#[tauri::command]
pub async fn start_overlay_mask(app: AppHandle, mode: Option<String>) -> Result<(), String> {
    // 0. Hide main window first so screenshot is clean (no self-capture).
    //    xcap 走 DXGI 桌面采集，hide() 后立刻截图会拿到隐藏前的旧帧 →
    //    冻结背景里会带上主窗口。先轮询确认 OS 层已隐藏，再等桌面合成刷新帧。
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
        for _ in 0..HIDE_POLL_ATTEMPTS {
            if !win.is_visible().unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(HIDE_POLL_INTERVAL_MS)).await;
        }
    }
    tokio::time::sleep(std::time::Duration::from_millis(COMPOSITOR_SETTLE_MS)).await;

    // 1. Pre-capture PRIMARY monitor — 既是冻结背景（overlay 不透明渲染），也是放大镜/OCR 数据源
    let (pixels, pw, ph) = capture_primary_monitor()?;

    // 2. Encode frozen frame as image data URL (opaque overlay background).
    //    JPEG：全屏编码比 PNG 快 4~5 倍，背景只是视觉层，选区/OCR 数据仍走无损 RGBA。
    let data_url = encode_bg_data_url(&pixels, pw, ph)?;

    // 3. Store in global cache (for magnifier / OCR region read)
    {
        let mut cache = PRE_SCREENSHOT.lock().expect("store pre-screenshot");
        *cache = Some((pixels, pw, ph));
    }

    // 4. Ensure overlay window exists (lazy init)
    ensure_overlay(&app)?;

    // 5. Pin overlay to primary monitor exactly (physical bounds), inject frozen bg, then show
    let overlay = app
        .get_webview_window("capture_overlay")
        .ok_or_else(|| "capture_overlay window not found".to_string())?;

    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("获取主显示器失败: {e}"))?
        .ok_or_else(|| "未找到主显示器".to_string())?;

    let mode_str = mode.as_deref().unwrap_or("screenshot");
    // Hide first if already showing (previous session)
    let _ = overlay.hide();
    let _ = overlay.set_position(tauri::Position::Physical(*monitor.position()));
    let _ = overlay.set_size(tauri::Size::Physical(*monitor.size()));
    // Inject bg BEFORE show to avoid pre-paint flash. base64 在 JS 单引号字符串内无转义风险。
    let _ = overlay.eval(format!(
        "window.__setBg__ && window.__setBg__('{}')",
        data_url
    ));
    let _ = overlay.eval(format!(
        "window.__setOverlayMode__ && window.__setOverlayMode__('{}')",
        mode_str
    ));
    let _ = overlay.show();
    let _ = overlay.set_focus();
    Ok(())
}

/// Capture the primary monitor into raw RGBA pixels (physical resolution).
/// 无 is_primary 报告时回退到第一台显示器。
fn capture_primary_monitor() -> Result<(Vec<u8>, u32, u32), String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("获取显示器列表失败: {e}"))?;
    let primary = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .cloned()
        .or_else(|| monitors.first().cloned())
        .ok_or_else(|| "未找到显示器".to_string())?;

    let full = primary
        .capture_image()
        .map_err(|e| format!("屏幕截图失败: {e}"))?;
    let pw = full.width();
    let ph = full.height();
    Ok((full.into_raw(), pw, ph))
}

/// Encode RGBA pixels as a JPEG `data:image/jpeg;base64,...` URL (overlay frozen background).
/// JPEG 比 PNG 编码快且体积小（背景仅视觉展示，不用无损）。
fn encode_bg_data_url(pixels: &[u8], w: u32, h: u32) -> Result<String, String> {
    use base64::Engine;
    let img_buf = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, pixels.to_vec())
        .ok_or_else(|| "创建全屏图像失败".to_string())?;
    let mut jpg_buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::from(img_buf)
        .write_to(&mut jpg_buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG 编码失败: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpg_buf.into_inner());
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

/// Ensure capture_overlay window exists (created once, hidden, reused)
pub fn ensure_overlay(app: &AppHandle) -> Result<(), String> {
    if OVERLAY_CREATED.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }

    // 不用 fullscreen + transparent：
    //   · fullscreen 不带显示器会落到"默认显示器"（多屏时往往是副屏）
    //   · WebView2 透明窗口在全屏/多屏/混合 DPI 下透明失效 → 渲染成黑色
    // 改为普通不透明窗口，显示时（start_overlay_mask）按主屏物理尺寸精确铺满，
    // 并把预截图注入为背景（opaque overlay，见 start_overlay_mask）。
    let overlay = WebviewWindowBuilder::new(
        app,
        "capture_overlay",
        WebviewUrl::App("capture_overlay.html".into()),
    )
    .title("")
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|e| format!("创建覆盖窗失败: {e}"))?;

    OVERLAY_CREATED.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = overlay; // keep alive
    Ok(())
}

/// Get small region screenshot near cursor (crop from pre-screenshot memory, no xcap, no overlay window)
/// Returns base64 PNG data URL
#[tauri::command]
pub async fn overlay_magnifier_region(x: i32, y: i32, size: u32) -> Result<String, String> {
    use base64::Engine;

    let size = size.max(1);
    let half = size / 2;
    let sx = (x.saturating_sub(half as i32)).max(0) as u32;
    let sy = (y.saturating_sub(half as i32)).max(0) as u32;

    // Crop small region from pre-screenshot cache (operate within lock only, don't clone entire image)
    let (cropped, cw, ch) = {
        let cache = PRE_SCREENSHOT
            .lock()
            .expect("read pre-screenshot for magnifier");
        let (pixels, pw, ph) = cache
            .as_ref()
            .ok_or_else(|| "预截图未就绪，请先调用 start_overlay_mask".to_string())?;
        let cw = size.min(pw.saturating_sub(sx));
        let ch = size.min(ph.saturating_sub(sy));
        (crop_rgba(pixels, *pw, sx, sy, cw, ch), cw, ch)
    };

    let img_buf = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(cw, ch, cropped)
        .ok_or_else(|| "创建裁剪图像失败".to_string())?;

    let mut png_buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::from(img_buf)
        .write_to(&mut png_buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(png_buf.into_inner());
    Ok(format!("data:image/png;base64,{}", b64))
}

/// User confirms selection → crop selection and return PNG base64
///
/// * mode = "screenshot" (default): hide overlay → live screenshot (with dynamic content) → restore overlay
/// * mode = "ocr" / "picker": crop directly from PRE_SCREENSHOT (clean, no overlay mask interference)
/// * mode = "rec_region" / "rec_template": crop directly from PRE_SCREENSHOT（录制铁律：ROI 证据与
///   find_image 模板一律走预截图裁剪，禁止 live capture——透明竞态根因），PNG 保存到当前录制会话
///   screenshots 目录（rec.rs 会话 state 注入），返回结构不变。
/// Overlay is closed by overlay_capture_done (confirm) or overlay_capture_cancel (cancel).
#[tauri::command]
pub async fn overlay_capture_confirm(
    app: AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    mode: Option<String>,
) -> Result<serde_json::Value, String> {
    use base64::Engine;

    let is_screenshot = mode.as_deref() == Some("screenshot");
    // 录制框选（ROI 证据 / find_image 模板）：走 PRE_SCREENSHOT 裁剪分支（is_screenshot=false 即命中），
    // 保存目录/文件名前缀按 mode 区分，其余行为与 ocr/picker 完全一致。
    let rec_prefix = match mode.as_deref() {
        Some("rec_region") => Some("rec_region"),
        Some("rec_template") => Some("rec_template"),
        _ => None,
    };

    let (pw, ph, pixels) = if is_screenshot {
        // ── Screenshot mode: hide overlay → live capture (dynamic content) ─
        if let Some(overlay) = app.get_webview_window("capture_overlay") {
            let _ = overlay.hide();
        }
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;

        let (pixels, pw, ph) = capture_primary_monitor()?;

        // Restore overlay (for preview thumbnail display)
        if let Some(overlay) = app.get_webview_window("capture_overlay") {
            let _ = overlay.show();
            let _ = overlay.set_focus();
        }

        (pw, ph, pixels)
    } else {
        // ── OCR/selection mode: crop directly from PRE_SCREENSHOT (no mask interference) ─
        let cache = PRE_SCREENSHOT
            .lock()
            .expect("read pre-screenshot for confirm");
        let (pixels, pw, ph) = cache
            .as_ref()
            .ok_or_else(|| "预截图未就绪，请先调用 start_overlay_mask".to_string())?;
        (*pw, *ph, pixels.clone())
    };

    // 裁剪选区
    let cap_x = x.max(0) as u32;
    let cap_y = y.max(0) as u32;
    let cap_w = width.max(1).min(pw.saturating_sub(cap_x));
    let cap_h = height.max(1).min(ph.saturating_sub(cap_y));

    let cropped = crop_rgba(&pixels, pw, cap_x, cap_y, cap_w, cap_h);
    let dyn_img = image::DynamicImage::from(
        image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(cap_w, cap_h, cropped)
            .ok_or_else(|| "创建裁剪图像失败".to_string())?,
    );

    // Save as PNG —— 录制模式保存到当前录制会话截图目录（会话未初始化则报错）
    let (save_dir, file_prefix) = if let Some(prefix) = rec_prefix {
        (
            crate::commands::rec::rec_active_screenshots_dir()?,
            prefix.to_string(),
        )
    } else {
        (
            nuphus::desktop::captures_dir_path().map_err(|e| format!("获取截图目录失败: {e}"))?,
            "capture".to_string(),
        )
    };
    std::fs::create_dir_all(&save_dir).map_err(|e| format!("创建截图目录失败: {e}"))?;
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let save_path = save_dir.join(format!("{file_prefix}_{ts}.png"));

    dyn_img
        .save(&save_path)
        .map_err(|e| format!("保存截图失败: {e}"))?;

    let path = save_path.display().to_string();

    // Encode as PNG base64 (for frontend preview)
    let mut png_buf = std::io::Cursor::new(Vec::new());
    dyn_img
        .write_to(&mut png_buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(png_buf.into_inner());
    let data_url = format!("data:image/png;base64,{}", b64);

    let result = serde_json::json!({
        "path": path,
        "x": cap_x,
        "y": cap_y,
        "width": cap_w,
        "height": cap_h,
        "base64": data_url,
    });

    Ok(result)
}

/// User confirms screenshot → store CAPTURE_RESULT → clear pre-screenshot → close overlay
#[tauri::command]
pub async fn overlay_capture_done(
    app: AppHandle,
    path: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    // 截图预览图（PNG data URL）。前端渲染直接用 base64 显示，绕开 asset 协议
    // 对系统 Temp 目录的 scope 覆盖不确定性（截图路径在 $TEMP/nuphus/captures）。
    // 坐标模式不传（None），调用方只从 region 提取坐标。
    base64: Option<String>,
) -> Result<(), String> {
    // 1. Store result in global cache (for take_capture_result polling consumption)
    *CAPTURE_RESULT.lock().expect("store capture result") = Some(serde_json::json!({
        "path": path,
        "region": {
            "x": x,
            "y": y,
            "width": width,
            "height": height,
        },
        "base64": base64,
    }));

    // 2. Clear pre-screenshot cache (no longer needed)
    *PRE_SCREENSHOT
        .lock()
        .expect("clear pre-screenshot after done") = None;

    // 3. Hide overlay window (keep alive for reuse, no white flash on next show)
    hide_overlay(&app);

    // 4. Restore main window now that screenshot is done
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }

    Ok(())
}

/// Pick color — read specified pixel color from pre-screenshot, store directly in CAPTURE_RESULT and hide overlay
#[tauri::command]
pub async fn overlay_pick_color(app: AppHandle, x: i32, y: i32) -> Result<(), String> {
    let (r, g, b) = {
        let cache = PRE_SCREENSHOT
            .lock()
            .expect("read pre-screenshot for pick color");
        let (pixels, pw, ph) = cache.as_ref().ok_or_else(|| "预截图未就绪".to_string())?;
        let px = x.max(0) as u32;
        let py = y.max(0) as u32;
        if px >= *pw || py >= *ph {
            return Err("坐标超出屏幕范围".to_string());
        }
        let idx = ((py * pw + px) * 4) as usize;
        (pixels[idx], pixels[idx + 1], pixels[idx + 2])
    };

    *CAPTURE_RESULT.lock().expect("store pick-color result") = Some(serde_json::json!({
        "color_rgb": [r, g, b],
        "hex": format!("#{:02X}{:02X}{:02X}", r, g, b),
        "x": x,
        "y": y,
    }));

    // Clear pre-screenshot cache
    *PRE_SCREENSHOT
        .lock()
        .expect("clear pre-screenshot after pick color") = None;

    // Hide overlay window + restore main window
    hide_overlay(&app);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }

    Ok(())
}

/// Take screenshot result (for polling) — called by main window, cleared after consumption
#[tauri::command]
pub fn take_capture_result() -> Result<Option<serde_json::Value>, String> {
    Ok(CAPTURE_RESULT.lock().expect("take capture result").take())
}

/// User cancels screenshot — hide overlay and restore main window
#[tauri::command]
pub async fn overlay_capture_cancel(app: AppHandle) -> Result<(), String> {
    // Store cancellation flag
    *CAPTURE_RESULT.lock().expect("store cancel flag") =
        Some(serde_json::json!({"cancelled": true}));

    // Clear pre-screenshot cache
    *PRE_SCREENSHOT
        .lock()
        .expect("clear pre-screenshot on cancel") = None;

    hide_overlay(&app);

    // Restore main window
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }

    Ok(())
}

fn hide_overlay(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("capture_overlay") {
        let _ = win.hide();
    }
}

/// Crop rectangle region from RGBA pixel buffer
fn crop_rgba(pixels: &[u8], full_width: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let bpp = 4u32;
    let stride = full_width * bpp;
    let mut out = Vec::with_capacity((w * h * bpp) as usize);

    for row in 0..h {
        let src_start = ((y + row) * stride + x * bpp) as usize;
        let src_end = (src_start + (w * bpp) as usize).min(pixels.len());
        out.extend_from_slice(&pixels[src_start..src_end]);
    }

    out
}
