//! HUD overlay window management
//!
//! Creates and manages a minimal always-on-top overlay
//! that shows execution progress text without stealing focus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tracing;

const HUD_LABEL: &str = "hud";
const HUD_WIDTH: f64 = 300.0;
const HUD_HEIGHT: f64 = 58.0;
/// 相对**工作区**右缘 / 下缘的留白（逻辑像素）。
/// 注意是工作区而非全屏——Dock / 任务栏由 `work_area()` 排除，见 `position_bottom_right`。
const HUD_MARGIN_RIGHT: f64 = 20.0;
const HUD_MARGIN_BOTTOM: f64 = 16.0;
/// `hide()` 把窗口挪到屏幕外的哨兵坐标（Windows WebView2 透明层残影规避）。
const HUD_OFFSCREEN: i32 = -99999;

/// 用户是否手动拖动过 HUD。拖过之后 `show()` 不再自动贴右下角（仅本次会话，重启还原）。
static HUD_USER_MOVED: AtomicBool = AtomicBool::new(false);
/// 用户最终把 HUD 放在哪（物理像素）。
static HUD_USER_POS: Mutex<Option<tauri::PhysicalPosition<i32>>> = Mutex::new(None);
/// 最近一次由**程序**设置的位置——用来区分「程序移动」与「用户拖动」：
/// Moved 事件坐标与它相同 = 我们自己设的；不同 = 用户拖的。
static HUD_LAST_SET_POS: Mutex<Option<tauri::PhysicalPosition<i32>>> = Mutex::new(None);

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
                observe_user_drag(&window);
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

    // 尺寸按当前缩放换算成物理像素恢复。旧实现直接拿逻辑常量当物理值用
    // （create() 的 inner_size 是逻辑像素，show() 却 set_size(Physical(300,58))），
    // 2x 屏上会把 HUD 缩成 150×29 逻辑像素。
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let _ = window.set_size(tauri::Size::Physical(hud_physical_size(
            monitor.scale_factor(),
        )));
    }

    // 位置：用户拖过就尊重用户的位置（hide() 会把窗口挪到屏幕外，这里顺带还原回去），
    // 从没拖过才自动贴右下角。
    match HUD_USER_POS.lock().ok().and_then(|p| *p) {
        Some(pos) if HUD_USER_MOVED.load(Ordering::Relaxed) => {
            move_programmatically(&window, pos);
        }
        _ => position_bottom_right(&window),
    }

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
        move_programmatically(
            &window,
            tauri::PhysicalPosition::new(HUD_OFFSCREEN, HUD_OFFSCREEN),
        );
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

/// HUD 目标尺寸（物理像素）= 逻辑尺寸 × 当前缩放。
fn hud_physical_size(scale: f64) -> tauri::PhysicalSize<u32> {
    tauri::PhysicalSize::new(
        (HUD_WIDTH * scale).round() as u32,
        (HUD_HEIGHT * scale).round() as u32,
    )
}

/// 程序主动移动窗口：**先记账再移动**，Moved 事件据此判定"这不是用户拖的"。
///
/// 记账必须在 set_position 之前：set_position 是异步派发到主线程的，事件回来时
/// 若还没记账就会被误判成用户拖动。
fn move_programmatically<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    pos: tauri::PhysicalPosition<i32>,
) {
    if let Ok(mut last) = HUD_LAST_SET_POS.lock() {
        *last = Some(pos);
    }
    let _ = window.set_position(tauri::Position::Physical(pos));
}

/// 监听窗口移动，把"用户拖动"与"程序移动"区分开并记住用户的位置。
///
/// 判据是坐标比对而非"正在定位"标志位：定位标志会在 set_position 返回后立刻清零，
/// 而 Moved 事件是稍后才到的，标志位方案必然漏判。
pub fn observe_user_drag<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    // 捕获 AppHandle 而不是 window 本身：这个闭包会被该窗口持有，捕获窗口会形成
    // 引用环（window → handler → window）。AppHandle 是刻意的可克隆句柄。
    let app = window.app_handle().clone();
    window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::Moved(pos) => {
                let programmatic = HUD_LAST_SET_POS
                    .lock()
                    .ok()
                    .and_then(|last| *last)
                    .map(|last| last == *pos)
                    .unwrap_or(false);
                // x <= HUD_OFFSCREEN 是 hide() 的屏幕外哨兵，永远不算用户位置
                if programmatic || pos.x <= HUD_OFFSCREEN {
                    return;
                }
                HUD_USER_MOVED.store(true, Ordering::Relaxed);
                if let Ok(mut user) = HUD_USER_POS.lock() {
                    *user = Some(*pos);
                }
            }
            // 显示模式 / 缩放切换（4K↔1080p、HiDPI、外接屏）。
            //
            // 实测事故（2026-09-15）：显示器从 3840×2160「looks like 1920×1080」(scale=2)
            // 切回 1920×1080(scale=1) 后，HUD 停在按 2x 算出的物理坐标 (3200,1832) 上——
            // 在 1x 体系里就是屏幕外：既看不见、也点不到（暂停/终止/关闭全丢），
            // 连 `screencapture -l <winid>` 都抓不到屏幕外窗口。
            //
            // 窗口的物理坐标不随缩放自动重算，所以这里必须自己重贴一次；用户手动拖过
            // 就尊重其位置（hud_bottom_right 里的 clamp 保证它至少不会留在屏幕外）。
            tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let Some(w) = app.get_webview_window(HUD_LABEL) else {
                    return;
                };
                if !HUD_USER_MOVED.load(Ordering::Relaxed) {
                    tracing::info!("[HUD] 缩放变化为 {}，重新贴回工作区右下角", scale_factor);
                    position_bottom_right(&w);
                    return;
                }
                // 用户手动摆过 → 不擅自搬走；但显示体系变了，必须保证它还留在可见区内，
                // 否则会变成"看不见也点不到"的死物（实测过一次）。
                let user_pos = HUD_USER_POS.lock().ok().and_then(|p| *p);
                if let (Some(pos), Ok(Some(monitor))) = (user_pos, w.primary_monitor()) {
                    let size = hud_physical_size(*scale_factor);
                    let fixed = clamp_into_area(pos, monitor.work_area(), size);
                    if fixed != pos {
                        tracing::info!(
                            "[HUD] 缩放变化后用户位置越界 ({}x{} → {}x{})，夹回工作区",
                            pos.x,
                            pos.y,
                            fixed.x,
                            fixed.y
                        );
                        if let Ok(mut user) = HUD_USER_POS.lock() {
                            *user = Some(fixed);
                        }
                        move_programmatically(&w, fixed);
                    }
                }
            }
            _ => {}
        }
    });
}

/// 把 HUD 贴到**工作区**右下角（不是全屏右下角）。
///
/// ⚠️ macOS 实测（2026-09-15，1920×1080 + 底部 Dock）：
/// `NSScreen.frame` = (0,0) 1920×1080，而 `NSScreen.visibleFrame` = (0,90) 1920×960
/// —— Dock 占掉底部 90px、菜单栏占掉顶部 30px。旧实现用 `monitor.size()`（= 全屏 frame）
/// 减硬编码 40px，于是 HUD 的 58px 里有 50px（86%）落在 Dock 背后。
///
/// 现改用 Tauri 官方 `Monitor::work_area()`：macOS 取 `NSScreen.visibleFrame`、
/// Windows 取 `GetMonitorInfoW` 的 `rcWork`、Linux 也有实现——三平台都有，不必自写 objc2。
///
/// ⚠️ 坐标**全程物理像素**（与 `set_position(Physical)` 同单位），不做逻辑/物理混算：
/// 混算正是历史上 Win11 高 DPI 下窗口跑到屏幕中央的根因（2026-08-25 那次修复）。
pub fn position_bottom_right<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let (size, pos) = hud_bottom_right(monitor.work_area(), monitor.scale_factor());
        let _ = window.set_size(tauri::Size::Physical(size));
        move_programmatically(window, pos);
    }
}

/// 纯计算：工作区（物理像素）+ 缩放 → HUD 的尺寸与右下角坐标。
///
/// 抽出来是为了能直接单测这段坐标数学——Dock 遮挡与 DPI 混算两次事故都出在这里，
/// 而它们都只能靠"真的摆一个窗口上去看"才发现的。
fn hud_bottom_right(
    area: &tauri::PhysicalRect<i32, u32>,
    scale: f64,
) -> (tauri::PhysicalSize<u32>, tauri::PhysicalPosition<i32>) {
    let size = hud_physical_size(scale);
    let margin_right = (HUD_MARGIN_RIGHT * scale).round() as i32;
    let margin_bottom = (HUD_MARGIN_BOTTOM * scale).round() as i32;

    let x = area.position.x + area.size.width as i32 - size.width as i32 - margin_right;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin_bottom;

    (
        size,
        clamp_into_area(tauri::PhysicalPosition::new(x, y), area, size),
    )
}

/// 把一个窗口位置夹进工作区，保证窗口**完整落在可见范围内**。
///
/// 为什么需要：显示模式/缩放切换后，窗口的物理坐标不会自动重算，而调用点可能拿到
/// 过期或退化的工作区（实测：显示器从 2x 切回 1x 后 HUD 停在 (3080,1828)——
/// 屏幕外，既看不见也点不到，连 `screencapture -l <winid>` 都抓不到屏幕外窗口）。
/// 工作区比窗口还小这种退化情形下，贴到左上角也好过消失。
fn clamp_into_area(
    pos: tauri::PhysicalPosition<i32>,
    area: &tauri::PhysicalRect<i32, u32>,
    size: tauri::PhysicalSize<u32>,
) -> tauri::PhysicalPosition<i32> {
    let min_x = area.position.x;
    let min_y = area.position.y;
    let max_x = (area.position.x + area.size.width as i32 - size.width as i32).max(min_x);
    let max_y = (area.position.y + area.size.height as i32 - size.height as i32).max(min_y);
    tauri::PhysicalPosition::new(pos.x.clamp(min_x, max_x), pos.y.clamp(min_y, max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: u32, h: u32) -> tauri::PhysicalRect<i32, u32> {
        tauri::PhysicalRect {
            position: tauri::PhysicalPosition::new(x, y),
            size: tauri::PhysicalSize::new(w, h),
        }
    }

    /// 开发机实测的那组数字（macOS 1920×1080 + 底部 Dock，scale=1）：
    /// `NSScreen.frame` (0,0,1920,1080)、`visibleFrame` (0,90,1920,960)
    /// → Tauri 的 work_area 换算成左上原点后是 position=(0,30) size=1920×960。
    ///
    /// 旧实现拿全屏 1080 减 40 得到 y=982，HUD 落到 982..1040，而 Dock 从 990 起 ——
    /// 58px 里 50px 被盖住。这条断言把"HUD 必须整体在 Dock 之上"钉死。
    #[test]
    fn hud_sits_above_dock_on_macos_1x() {
        let (size, pos) = hud_bottom_right(&rect(0, 30, 1920, 960), 1.0);

        assert_eq!(size, tauri::PhysicalSize::new(300, 58));
        assert_eq!(pos, tauri::PhysicalPosition::new(1600, 916));

        // Dock 上沿 = 屏幕高 1080 - Dock 高 90 = 990；HUD 底边必须在其上方
        let hud_bottom = pos.y + size.height as i32;
        assert!(
            hud_bottom <= 990,
            "HUD 底边 {hud_bottom} 落到了 Dock 区域（Dock 上沿 990）"
        );
    }

    /// Retina 2x：逻辑尺寸不变（300×58），物理尺寸翻倍。
    /// 旧实现 create() 用逻辑 300×58、show() 却 set_size(Physical(300,58))，
    /// 在 2x 屏上窗口只有 150×29 逻辑像素——半尺寸。
    #[test]
    fn hud_keeps_logical_size_on_retina_2x() {
        let (size, pos) = hud_bottom_right(&rect(0, 60, 3840, 1920), 2.0);

        assert_eq!(size, tauri::PhysicalSize::new(600, 116));
        // 逻辑尺寸 == 600/2 × 116/2 == 300 × 58
        assert_eq!(size.width as f64 / 2.0, HUD_WIDTH);
        assert_eq!(size.height as f64 / 2.0, HUD_HEIGHT);
        // 右缘留白 20 逻辑 = 40 物理；底部留白 16 逻辑 = 32 物理
        assert_eq!(pos, tauri::PhysicalPosition::new(3200, 1832));
    }

    /// 非零工作区原点（副屏在主屏右侧/上方时会碰到）不能被忽略。
    #[test]
    fn hud_respects_non_zero_work_area_origin() {
        let (size, pos) = hud_bottom_right(&rect(1920, 30, 1920, 960), 1.0);

        assert_eq!(size, tauri::PhysicalSize::new(300, 58));
        assert_eq!(pos, tauri::PhysicalPosition::new(3520, 916));
    }

    /// **实测事故的回归**：显示器从 3840×2160「looks like 1920×1080」(scale=2) 切回
    /// 1920×1080(scale=1) 后，HUD 停在按 2x 算出的物理坐标 (3080,1828) 上——在 1x 的
    /// 1920×1080 工作区里完全在屏幕外：看不见、点不到，连截图工具都抓不到。
    /// clamp_into_area 必须把它拉回可见范围。
    #[test]
    fn clamp_pulls_offscreen_hud_back_into_work_area() {
        let area = rect(0, 30, 1920, 960); // 1x 屏：菜单栏 30 + Dock 90
        let size = tauri::PhysicalSize::new(300, 58);
        let fixed = clamp_into_area(tauri::PhysicalPosition::new(3080, 1828), &area, size);

        // 右下角贴边：x = 1920-300 = 1620，y = 30+960-58 = 932
        assert_eq!(fixed, tauri::PhysicalPosition::new(1620, 932));
        assert!(fixed.x + size.width as i32 <= area.position.x + area.size.width as i32);
        assert!(fixed.y + size.height as i32 <= area.position.y + area.size.height as i32);
    }

    /// 已经在工作区内的位置不能被 clamp 改动（否则用户拖过的位置会被悄悄挪走）。
    #[test]
    fn clamp_keeps_in_bounds_position_untouched() {
        let area = rect(0, 30, 1920, 960);
        let size = tauri::PhysicalSize::new(300, 58);
        for pos in [(1600, 916), (0, 30), (1620, 932), (100, 500)] {
            let p = tauri::PhysicalPosition::new(pos.0, pos.1);
            assert_eq!(clamp_into_area(p, &area, size), p, "不应改动 {pos:?}");
        }
    }

    /// 退化情形：工作区比窗口还小。贴到左上角好过整个消失（拿不到屏幕外窗口）。
    #[test]
    fn clamp_degrades_to_area_origin_when_area_smaller_than_window() {
        let area = rect(10, 20, 100, 20);
        let size = tauri::PhysicalSize::new(300, 58);
        let fixed = clamp_into_area(tauri::PhysicalPosition::new(9999, 9999), &area, size);

        assert_eq!(fixed, tauri::PhysicalPosition::new(10, 20));
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

/// 取活动工作流 id，并在取不到时留下日志。
///
/// 这三个 HUD 控制命令原先只 `ok_or_else` 返回错误、前端又 `catch {}` 吞掉，于是
/// "点了没反应"既无界面反馈也无日志线索。错误文案也改成用户看得懂的说法
/// （前端已改为把它显示在 HUD 上）。
fn require_active_workflow(
    state: &tauri::State<'_, crate::state::AppState>,
) -> Result<String, String> {
    nuphus::workflow::hud_control::active_id(&state.signals).ok_or_else(|| {
        tracing::warn!("[HUD] 控制命令被忽略：当前没有活动工作流");
        "当前没有活动工作流".to_string()
    })
}

/// Tauri command: pause active workflow
#[tauri::command]
pub async fn hud_pause(state: tauri::State<'_, crate::state::AppState>) -> Result<(), String> {
    let wf_id = require_active_workflow(&state)?;
    let engine = state.workflow_engine.read().await;
    engine.executor.pause(&wf_id).await;
    Ok(())
}

/// Tauri command: resume active workflow
#[tauri::command]
pub async fn hud_resume(state: tauri::State<'_, crate::state::AppState>) -> Result<(), String> {
    let wf_id = require_active_workflow(&state)?;
    let engine = state.workflow_engine.read().await;
    engine.executor.resume(&wf_id).await;
    Ok(())
}

/// Tauri command: stop active workflow
#[tauri::command]
pub async fn hud_stop(state: tauri::State<'_, crate::state::AppState>) -> Result<(), String> {
    let wf_id = require_active_workflow(&state)?;
    let engine = state.workflow_engine.read().await;
    engine.executor.cancel(&wf_id).await;
    // Also resume if paused (so the executor can process the cancel)
    engine.executor.resume(&wf_id).await;
    nuphus::workflow::hud_control::mark_user_cancelled();
    tracing::info!("[HUD] 终止活动工作流: {}", wf_id);
    Ok(())
}
