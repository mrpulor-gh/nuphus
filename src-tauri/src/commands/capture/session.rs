//! 截图 / 选区（picker）流程编排 —— 把「抓屏排除/隐藏主窗 → 等合成器 → 抓屏 → 原生遮罩 → 确认 → 恢复」串起来。
//!
//! **截图与选区共用同一套原生遮罩**（同一个冻结帧、同一套 hover/8 节点/OK-X 交互），
//! 差别只在交付物：截图裁切落盘，选区只回夹紧后的屏幕坐标（见 `clamp_sel`）。
//!
//! **自身主窗必须从冻结帧里排除**：靠 `WDA_EXCLUDEFROMCAPTURE`（与抓帧时机无关）
//! + 隐藏主窗双保险；单靠 hide() 会抢在 DWM 淡出动画之前抓帧（详见 run_screenshot ①）。
//!
//! 职责边界：
//!   · 本模块：流程顺序、主窗隐藏/恢复、抓屏（xcap）、结果裁剪与保存
//!   · `surface::*`：遮罩窗口本身（渲染 + 输入）
//!   · `state` / `geometry`：纯状态与几何
//!
//! **确定性原则**（硬约束）：不使用轮询与硬 sleep。等待 DWM 完成窗口移除
//! 用 `DwmFlush`（合成器同步，返回即确定），而非「猜一个 150ms」。

use tauri::{AppHandle, Manager};

/// 一次截图的结果（确认后）。
pub struct CaptureResult {
    /// 选区（物理像素，可能越界/为负；由下游裁剪时夹紧）
    pub sel: (i32, i32, i32, i32),
    /// 冻结帧（top-down BGRA，长度 = screen_w * screen_h）
    pub frozen: Vec<u32>,
    pub screen_w: i32,
    pub screen_h: i32,
}

/// 抓主显示器整屏（RGBA → 转换为 top-down BGRA u32）。
pub fn capture_primary_bgra() -> Result<(Vec<u32>, i32, i32), String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("获取显示器列表失败: {e}"))?;
    let primary = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| "未找到显示器".to_string())?;
    let img = primary
        .capture_image()
        .map_err(|e| format!("屏幕截图失败: {e}"))?;
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut out: Vec<u32> = Vec::with_capacity((w * h) as usize);
    for p in img.pixels() {
        let (r, g, b, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
        // 目标布局 0xAARRGGBB（与 GDI+ PixelFormat32bppARGB / DIB 的 BGRA 内存序一致）
        out.push((a << 24) | (r << 16) | (g << 8) | b);
    }
    Ok((out, w, h))
}

/// 等 DWM 完成窗口移除（Windows 用 DwmFlush 做合成器同步；其他平台空操作）。
///
/// 实测：隐藏窗口后 xcap(DXGI) 抓屏约 5ms 就干净；这里用合成器同步而非 sleep，
/// 保证「返回即确定」，不靠猜时间。
#[cfg(windows)]
pub fn wait_visible_settle() {
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmFlush() -> i32;
    }
    unsafe {
        // 两次即可覆盖「窗口隐藏」这一级合成变化；再多是浪费（实测 1~2 次足够，
        // 4 次是隐藏分层窗口的最坏情况余量）。
        for _ in 0..2 {
            DwmFlush();
        }
    }
}

#[cfg(not(windows))]
pub fn wait_visible_settle() {}

/// Win32 抓屏亲和性：把窗口从抓屏接口的结果里排除。
///
/// 只用 `user32!SetWindowDisplayAffinity`（无需第三方 crate）。必须在会话结束时
/// 用 `restore_capture_affinity` 还原，否则本进程后续所有抓屏都看不到这个窗口。
#[cfg(windows)]
mod win32 {
    /// 窗口对抓屏接口不可见：抓到的该区域显示其下层内容。
    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0011;
    /// 取消排除（默认值）。
    const WDA_NONE: u32 = 0x0000;

    #[link(name = "user32")]
    extern "system" {
        fn SetWindowDisplayAffinity(hwnd: isize, affinity: u32) -> i32;
    }

    /// 让自身主窗对抓屏「不存在」。返回是否设置成功。
    pub fn exclude_from_capture(hwnd: isize) -> bool {
        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) != 0 }
    }

    /// 还原抓屏亲和性。
    pub fn restore_capture_affinity(hwnd: isize) {
        unsafe {
            SetWindowDisplayAffinity(hwnd, WDA_NONE);
        }
    }
}

/// 执行一次原生遮罩会话（截图 / 选区 / 取色 / 鼠标 / OCR 共用）。
///
/// `point_pick = true` 进入单点拾取模式（取色 / 鼠标坐标）：单击即确认、选区 = 该点 1x1。
/// `pick_color = true`（point_pick 的子集）额外绘制取色 HUD（放大镜 + 实时色值）。
///
/// 返回 `Ok(Some(result))` = 用户确认；`Ok(None)` = 取消。
/// **无论成功失败，主窗都会被恢复**（含所有错误路径）。
#[cfg(windows)]
pub fn run_screenshot(
    app: &AppHandle,
    point_pick: bool,
    pick_color: bool,
) -> Result<Option<CaptureResult>, String> {
    // 只依赖 `surface` 的契约（平台无关）：三平台各自实现同一 `run_overlay` 签名。
    use super::surface::{run_overlay, OverlayOutcome, OverlayParams};

    let main = app.get_webview_window("main");
    let main_hwnd: Option<isize> = main
        .as_ref()
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize);

    // ① 抓屏排除自身主窗：WDA_EXCLUDEFROMCAPTURE 让抓屏接口在合成结果里「当它不存在」
    //    （该区域显示下层内容）。这是**与抓帧时机无关**的确定性手段。
    //    单靠 hide() 不够 —— SW_HIDE 在 DWM 上是一段淡出动画：2026-10-05 真机实测，
    //    hide() 后 33ms 抓到的帧里主窗仍完好、+119ms 半透明、+263ms 才真正消失，
    //    33ms 的 DwmFlush×2 必然抢在动画前 → 冻结帧里带自身主窗。
    let excluded = main_hwnd.map(win32::exclude_from_capture).unwrap_or(false);
    tracing::info!("[capture] 主窗抓屏排除 exclude_from_capture={excluded} hwnd={main_hwnd:?}");

    // ② 隐藏主窗（可见性语义；与 ① 双保险 —— ① 失效时至少窗口已消失）
    if let Some(w) = &main {
        let _ = w.hide();
    }
    // 主窗恢复的保障：无论后面哪一步失败都要恢复可见性并解除抓屏排除。用 guard 兜底。
    struct Restore {
        win: Option<tauri::WebviewWindow>,
        hwnd: Option<isize>,
        excluded: bool,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(w) = &self.win {
                let _ = w.show();
                let _ = w.set_focus();
            }
            if self.excluded {
                if let Some(h) = self.hwnd {
                    win32::restore_capture_affinity(h);
                }
            }
        }
    }
    let _guard = Restore {
        win: main.clone(),
        hwnd: main_hwnd,
        excluded,
    };

    // ③ 等 DWM 完成主窗的合成层移除（合成器同步，非 sleep）。
    //    ① 的抓屏排除已让帧内容与抓帧时机无关，这里只做一次合成器同步。
    wait_visible_settle();

    // ④ 抓屏（画面干净：① 已把自身主窗从抓屏结果里排除）
    let (frozen, screen_w, screen_h) = capture_primary_bgra()?;

    // ⑤ 独立线程跑原生遮罩（阻塞直到用户确认/取消；不阻塞 Tauri 运行时）
    let params = OverlayParams {
        frozen_bg: frozen.clone(),
        screen_w,
        screen_h,
        point_pick,
        pick_color,
        // macOS 需要它把 AppKit 建窗/节拍调度回主线程；Windows / Linux 忽略。
        app: Some(app.clone()),
    };
    let outcome = std::thread::Builder::new()
        .name("nuphus-capture-overlay".into())
        .spawn(move || run_overlay(params))
        .map_err(|e| format!("启动遮罩线程失败: {e}"))?
        .join()
        .map_err(|_| "遮罩线程异常结束".to_string())?;

    // ⑤ 主窗由 _guard 在函数返回时恢复
    Ok(match outcome {
        OverlayOutcome::Confirmed { sel } => Some(CaptureResult {
            sel,
            frozen,
            screen_w,
            screen_h,
        }),
        OverlayOutcome::Cancelled => None,
    })
}

/// 非 Windows：返回 None，由调用方回退到现有 WebView 链路。
#[cfg(not(windows))]
pub fn run_screenshot(
    _app: &AppHandle,
    _point_pick: bool,
    _pick_color: bool,
) -> Result<Option<CaptureResult>, String> {
    Err("native-capture-unsupported".to_string())
}

/// 把选区夹到屏幕内（越界沿屏幕边缘切掉）。返回夹紧后的 `(x, y, w, h)`；
/// 夹紧后为空（宽或高 ≤ 0）返回 `None`。
///
/// 选区与截图共用同一口径：对外交付的坐标一律是**夹紧后的屏幕物理像素**。
pub fn clamp_sel(
    sel: (i32, i32, i32, i32),
    screen_w: i32,
    screen_h: i32,
) -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = sel;
    let x0 = x.max(0).min(screen_w);
    let y0 = y.max(0).min(screen_h);
    let x1 = (x + w).max(0).min(screen_w);
    let y1 = (y + h).max(0).min(screen_h);
    let (cw, ch) = (x1 - x0, y1 - y0);
    if cw <= 0 || ch <= 0 {
        None
    } else {
        Some((x0, y0, cw, ch))
    }
}

/// 从冻结帧按选区裁剪（选区越界时沿屏幕边缘夹紧）。
/// 返回 (x, y, w, h, top-down BGRA u32 像素) —— x/y 是**夹紧后**的屏幕坐标，
/// 供前端 region 显示时与实际产物一致。
pub fn crop_from_frozen(
    frozen: &[u32],
    screen_w: i32,
    screen_h: i32,
    sel: (i32, i32, i32, i32),
) -> Result<(i32, i32, i32, i32, Vec<u32>), String> {
    let (x0, y0, cw, ch) =
        clamp_sel(sel, screen_w, screen_h).ok_or_else(|| "选区非法（裁剪后为空）".to_string())?;
    let mut out: Vec<u32> = Vec::with_capacity((cw * ch) as usize);
    for row in y0..y0 + ch {
        let s = (row as usize) * (screen_w as usize) + x0 as usize;
        let e = s + cw as usize;
        if e > frozen.len() {
            return Err("冻结帧尺寸与选区不一致".to_string());
        }
        out.extend_from_slice(&frozen[s..e]);
    }
    Ok((x0, y0, cw, ch, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_clamps_to_screen() {
        let w: i32 = 100;
        let h: i32 = 50;
        let frozen: Vec<u32> = (0..((w * h) as u32)).collect();
        let (x, y, cw, ch, px) = crop_from_frozen(&frozen, w, h, (-10, -10, 30, 30)).unwrap();
        assert_eq!((x, y, cw, ch), (0, 0, 20, 20));
        assert_eq!(px.len(), 400);
    }

    #[test]
    fn crop_rejects_empty() {
        let frozen: Vec<u32> = vec![0; 100];
        assert!(crop_from_frozen(&frozen, 10, 10, (20, 20, 5, 5)).is_err());
    }
}
