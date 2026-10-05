//! macOS 原生遮罩（AppKit）—— 替代 WebView overlay 渲染。
//!
//! # 实现方式
//!
//! | 环节 | 手段 |
//! |---|---|
//! | 线程 | AppKit 的 UI 操作必须在主线程 → `AppHandle::run_on_main_thread` 建窗/驱动，调用方线程用 `mpsc` 等待结果 |
//! | 窗口 | `NSWindow`：borderless、`level = 1000`（高于菜单栏）、frame = `NSScreen.main`，覆盖主屏 |
//! | 呈现 | 像素缓冲（BGRA，与 `paint` 同一份 0xAARRGGBB）→ `CGImage` → `NSImage` → `NSImageView.setImage`，每帧替换 |
//! | 输入 | `NSEvent.addLocalMonitorForEventsMatchingMask`：窗口全屏覆盖 ⇒ 鼠标事件必属本应用，监视器**吞掉**全部事件（右键 = 退出，不是菜单） |
//! | 节拍 | `NSTimer`（16ms）驱动「dirty → `paint::render_opaque` → 呈现」 |
//! | 抓屏 | 由 `session.rs` 用 xcap（CGDisplay 后端）完成；本模块只显示 |
//! | 视觉规格 | `surface::paint`（与 Windows / Linux 同一份颜色、圆角、字形、HUD 实现） |
//!
//! # 为什么不用 CGEventTap
//!
//! `CGEventTap` 需要「辅助功能」授权（`AXIsProcessTrusted`），首次使用要引导用户去系统设置，
//! 对截图工具是过重的门槛。这里改为：窗口全屏覆盖 + 应用内事件监视 —— 事件天然属于本应用，
//! 无需任何系统权限即可拦截。
//!
//! # 已知限制（真机验证时逐条确认）
//!
//! - **ESC 键**：borderless 窗口默认 `canBecomeKey = NO`，收不到键盘事件。当前只保证
//!   右键退出；若要 ESC，需要 NSWindow 子类覆写 `canBecomeKey`（真机迭代）。
//! - **混合 DPI / 多显示器**：只覆盖主屏（与 Windows / Linux 实现同范围）；事件坐标按
//!   `backingScaleFactor` 换算到物理像素。
//! - **非预乘 alpha**：`paint` 的描边色（`0x33FF_FFFF`）是直通 alpha，CoreGraphics 按
//!   预乘解释，边缘会有极轻微偏色（观感可忽略，记录在此以免误判为 bug）。
//!
//! # 验证状态
//!
//! **本文件在 Windows 开发机上不参与编译**（`#[cfg(target_os = "macos")]`），
//! 只能由 CI（`check-macos`：macOS arm64 `cargo check --workspace`）验证编译；
//! 交互行为需 macOS 真机验证。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSPoint, NSRect, NSSize};

// 仅用 `msg_send!` / `class!` 不会把 AppKit / Foundation 链进二进制，必须显式 link，
// 否则运行时类查找失败（编译期不报错）。
#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "Foundation", kind = "framework")]
extern "C" {}

use super::{OverlayOutcome, OverlayParams};
use crate::commands::capture::geometry::Point;
use crate::commands::capture::paint;
use crate::commands::capture::state::{Action, App, Phase};

/// 硬超时：无论如何都能自退出（与 Windows / Linux 实现同值）。
const HARD_TIMEOUT_SECS: u64 = 180;
/// 渲染节拍（≈60fps）。
const TICK_SECS: f64 = 0.016;

// ---- AppKit 常量（用字面量，避免依赖各版本绑定的常量命名）----

/// `NSWindowStyleMask::Borderless`
const STYLE_BORDERLESS: usize = 0;
/// `NSBackingStoreType::Buffered`
const BACKING_BUFFERED: usize = 2;
/// `NSScreenSaverWindowLevel`（高于菜单栏 24 / 浮动层 3）
const WINDOW_LEVEL_SCREEN_SAVER: isize = 1000;
/// `NSEventMask`：左/右/其它键按下抬起 + 移动 + 拖动 + 键盘
const EVENT_MASK: usize = (1 << 1)  // LeftMouseDown
    | (1 << 2)                      // LeftMouseUp
    | (1 << 3)                      // RightMouseDown
    | (1 << 4)                      // RightMouseUp
    | (1 << 5)                      // MouseMoved
    | (1 << 6)                      // LeftMouseDragged
    | (1 << 7)                      // RightMouseDragged
    | (1 << 10)                     // KeyDown
    | (1 << 25)                     // OtherMouseDown
    | (1 << 26); // OtherMouseUp

// `NSEventType`
const EV_LEFT_MOUSE_DOWN: usize = 1;
const EV_LEFT_MOUSE_UP: usize = 2;
const EV_RIGHT_MOUSE_DOWN: usize = 3;
const EV_RIGHT_MOUSE_UP: usize = 4;
const EV_MOUSE_MOVED: usize = 5;
const EV_LEFT_MOUSE_DRAGGED: usize = 6;
const EV_RIGHT_MOUSE_DRAGGED: usize = 7;
const EV_KEY_DOWN: usize = 10;

/// macOS 虚拟键码：ESC / Return / Space。
const KEY_ESC: u16 = 53;
const KEY_RETURN: u16 = 36;
const KEY_SPACE: u16 = 49;

/// CGBitmapInfo：`kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little`
/// = 2 | (2 << 12) —— 即 BGRA8888 预乘，正好对上 `paint` 的 `0xAARRGGBB` 内存字节序。
const CG_BITMAP_INFO_BGRA_PREMULTIPLIED: u32 = 2 | (2 << 12);

/// 跨平台统一入口（契约见 `surface::mod`）：阻塞直到用户确认 / 取消。
pub fn run_overlay(params: OverlayParams) -> OverlayOutcome {
    let Some(handle) = params.app.clone() else {
        tracing::warn!("[capture/macos] 缺少 AppHandle，无法在主线程建窗");
        return OverlayOutcome::Cancelled;
    };

    let (tx, rx) = mpsc::channel::<OverlayOutcome>();
    let shared = Rc::new(RefCell::new(MacState {
        app: App::new(params.screen_w.max(1), params.screen_h.max(1)),
        buf: vec![0u32; (params.screen_w.max(1) as usize) * (params.screen_h.max(1) as usize)],
        frozen: params.frozen_bg.clone(),
        w: params.screen_w.max(1),
        h: params.screen_h.max(1),
        point_pick: params.point_pick,
        pick_color: params.pick_color,
        dirty: true,
        done: false,
        outcome: OverlayOutcome::Cancelled,
        window: None,
        image_view: None,
        timer: None,
        monitor: None,
        tx: Some(tx),
    }));

    // 建窗 + 节拍 + 事件监视全部在主线程（AppKit 硬要求）
    let for_main = shared.clone();
    if let Err(e) = handle.run_on_main_thread(move || {
        if let Err(err) = setup_main_thread(&for_main) {
            tracing::warn!("[capture/macos] 主线程建窗失败：{err}");
            let mut st = for_main.borrow_mut();
            st.done = true;
            finish(&mut st);
        }
    }) {
        tracing::warn!("[capture/macos] run_on_main_thread 调用失败：{e}");
        return OverlayOutcome::Cancelled;
    }

    match rx.recv_timeout(Duration::from_secs(HARD_TIMEOUT_SECS)) {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::warn!("[capture/macos] 等待遮罩结束超时，按取消处理");
            let for_main = shared.clone();
            let _ = handle.run_on_main_thread(move || {
                let mut st = for_main.borrow_mut();
                st.done = true;
                finish(&mut st);
            });
            OverlayOutcome::Cancelled
        }
    }
}

/// 遮罩运行态（**只在主线程访问**：`Rc<RefCell<_>>` 不跨线程）。
struct MacState {
    app: App,
    buf: Vec<u32>,
    frozen: Vec<u32>,
    w: i32,
    h: i32,
    point_pick: bool,
    pick_color: bool,
    dirty: bool,
    done: bool,
    outcome: OverlayOutcome,
    window: Option<Retained<AnyObject>>,
    image_view: Option<Retained<AnyObject>>,
    timer: Option<Retained<AnyObject>>,
    monitor: Option<Retained<AnyObject>>,
    tx: Option<mpsc::Sender<OverlayOutcome>>,
    /// 主屏 backing scale（逻辑点 → 物理像素），事件坐标换算用。
    scale: f64,
}

/// 主线程：建窗 → 装视图 → 挂定时器 → 挂事件监视 → 首帧。
fn setup_main_thread(shared: &Rc<RefCell<MacState>>) -> Result<(), String> {
    let (w, h) = {
        let st = shared.borrow();
        (st.w, st.h)
    };

    unsafe {
        // NSScreen.main：窗口 frame 用**点**（AppKit 坐标），图像用物理像素，
        // 由 backing scale 自动缩放，两边口径统一到屏幕物理像素。
        let screen: Retained<AnyObject> = msg_send![class!(NSScreen), mainScreen];
        let screen_frame: NSRect = msg_send![&screen, frame];
        let scale: f64 = msg_send![&screen, backingScaleFactor];

        // ① NSWindow（borderless + 置顶 + 不透明）
        let alloc: Retained<AnyObject> = msg_send![class!(NSWindow), alloc];
        let window: Retained<AnyObject> = msg_send![
            alloc,
            initWithContentRect: screen_frame,
            styleMask: STYLE_BORDERLESS,
            backing: BACKING_BUFFERED,
            defer: false
        ];
        let _: () = msg_send![&window, setLevel: WINDOW_LEVEL_SCREEN_SAVER];
        let _: () = msg_send![&window, setOpaque: true];
        let _: () = msg_send![&window, setIgnoresMouseEvents: false];
        let _: () = msg_send![&window, setHasShadow: false];

        // ② NSImageView 铺满窗口（呈现层）
        let iv_alloc: Retained<AnyObject> = msg_send![class!(NSImageView), alloc];
        let image_view: Retained<AnyObject> = msg_send![iv_alloc, initWithFrame: screen_frame];
        let _: () = msg_send![&image_view, setImageScaling: 3isize]; // NSImageScaleAxesIndependently
        let _: () = msg_send![&window, setContentView: &*image_view];
        let _: () = msg_send![&window, makeKeyAndOrderFront: core::ptr::null_mut::<AnyObject>()];

        {
            let mut st = shared.borrow_mut();
            st.window = Some(window);
            st.image_view = Some(image_view);
            // 记录物理/逻辑换算比例（事件坐标用）
            st.scale = scale;
        }

        // ③ 事件监视：本应用事件一律吞掉（右键 = 退出，不让菜单/下层收到）
        let for_events = shared.clone();
        let handler = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
            if event.is_null() {
                return core::ptr::null_mut();
            }
            on_event(&for_events, event);
            core::ptr::null_mut() // 吞掉
        });
        let monitor: Retained<AnyObject> = msg_send![
            class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: EVENT_MASK,
            handler: &*handler
        ];
        shared.borrow_mut().monitor = Some(monitor);

        // ④ 节拍：dirty → 渲染 → 呈现；done → 收尾
        let for_tick = shared.clone();
        let tick = block2::RcBlock::new(move |_timer: *mut AnyObject| {
            let mut st = for_tick.borrow_mut();
            if st.done {
                return;
            }
            if st.dirty {
                st.dirty = false;
                if let Err(e) = render_and_present(&mut st) {
                    tracing::warn!("[capture/macos] 渲染失败：{e}");
                }
            }
            if st.done {
                finish(&mut st);
            }
        });
        let timer: Retained<AnyObject> = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: TICK_SECS,
            repeats: true,
            block: &*tick
        ];
        shared.borrow_mut().timer = Some(timer);

        // ⑤ 首帧（构造时 dirty = true，下一次 tick 就会画）
        let mut st = shared.borrow_mut();
        render_and_present(&mut st)?;
        st.dirty = false;
    }

    Ok(())
}

/// 事件分派：坐标从窗口**点**换算到屏幕**物理像素**（与 `paint` 的像素口径一致）。
fn on_event(shared: &Rc<RefCell<MacState>>, event: *mut AnyObject) {
    unsafe {
        let etype: usize = msg_send![event, type];
        let loc: NSPoint = msg_send![event, locationInWindow];

        let mut st = shared.borrow_mut();
        let (w, h, scale) = (st.w, st.h, st.scale);
        // AppKit 窗口坐标原点在左下；`paint` 是 top-down（y=0 在屏幕最上）→ 翻转 y。
        let px = (loc.x * scale).round() as i32;
        let py = (h as f64 - loc.y * scale).round() as i32;
        let (px, py) = (px.clamp(0, w - 1), py.clamp(0, h - 1));

        match etype {
            EV_LEFT_MOUSE_DOWN => {
                st.app.last_mouse = Point { x: px, y: py };
                match st.app.on_ldown(px, py, w, h) {
                    Action::ConfirmSave => {
                        st.outcome = OverlayOutcome::Confirmed {
                            sel: (st.app.sel.x, st.app.sel.y, st.app.sel.w, st.app.sel.h),
                        };
                        st.done = true;
                    }
                    Action::Cancel => st.done = true,
                    Action::None => {}
                }
                st.dirty = true;
            }
            EV_LEFT_MOUSE_UP => {
                if st.point_pick {
                    // 单点拾取（取色 / 鼠标坐标）：松手即确认，选区 = 该点 1×1
                    st.app.pick_point_at(px, py);
                    st.outcome = OverlayOutcome::Confirmed {
                        sel: (px, py, 1, 1),
                    };
                    st.done = true;
                } else {
                    st.app.on_lup(px, py);
                    st.dirty = true;
                }
            }
            EV_RIGHT_MOUSE_DOWN | EV_RIGHT_MOUSE_UP => {
                // 右键 = 退出（事件已被监视器吞掉，不会有右键菜单）
                st.app.enter_exiting();
                st.done = true;
            }
            EV_MOUSE_MOVED | EV_LEFT_MOUSE_DRAGGED | EV_RIGHT_MOUSE_DRAGGED => {
                st.app.last_mouse = Point { x: px, y: py };
                st.app.on_move(px, py, w, h);
                st.dirty = true;
            }
            EV_KEY_DOWN => {
                let code: u16 = msg_send![event, keyCode];
                match code {
                    KEY_ESC => {
                        st.app.enter_exiting();
                        st.done = true;
                    }
                    KEY_RETURN | KEY_SPACE => {
                        if matches!(
                            st.app.phase,
                            Phase::Confirm | Phase::Moving | Phase::Adjusting
                        ) {
                            st.outcome = OverlayOutcome::Confirmed {
                                sel: (st.app.sel.x, st.app.sel.y, st.app.sel.w, st.app.sel.h),
                            };
                            st.done = true;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// 渲染一帧并贴到视图：`paint::render_opaque`（与 Linux 同一条路径）→ `CGImage` → `NSImage`。
unsafe fn render_and_present(st: &mut MacState) -> Result<(), String> {
    let (w, h) = (st.w, st.h);
    paint::render_opaque(
        &mut st.buf,
        w,
        h,
        &mut st.app,
        st.point_pick,
        st.pick_color,
        &st.frozen,
    );

    let Some(image_view) = st.image_view.as_ref() else {
        return Err("视图未就绪".to_string());
    };

    // CGImage 直接引用 `st.buf` 的内存（不复制）：每帧重建，替换视图内容。
    let color_space = CGColorSpaceCreateDeviceRGB();
    let provider = CGDataProviderCreateWithData(
        core::ptr::null_mut(),
        st.buf.as_ptr() as *const core::ffi::c_void,
        st.buf.len() * 4,
        None, // releaseData：缓冲区由 MacState 持有，CG 不接管所有权
    );
    // 注意：`CGBitmapInfo` / `CGColorRenderingIntent` 在新版绑定里若被生成为
    // bitflags / enum 类型，这里的字面量需要相应包一层（真机/CI 首跑时按报错调整）。
    let image = CGImageCreate(
        w as usize,
        h as usize,
        8,                // bits per component
        32,               // bits per pixel
        (w as usize) * 4, // bytes per row
        color_space.as_deref(),
        CG_BITMAP_INFO_BGRA_PREMULTIPLIED,
        provider.as_deref(),
        core::ptr::null(), // decode
        false,             // shouldInterpolate
        0,                 // kCGRenderingIntentDefault
    )
    .ok_or_else(|| "CGImageCreate 失败".to_string())?;

    let size = NSSize {
        width: w as f64,
        height: h as f64,
    };
    let ns_alloc: Retained<AnyObject> = msg_send![class!(NSImage), alloc];
    let ns_image: Retained<AnyObject> = msg_send![ns_alloc, initWithCGImage: &*image, size: size];
    let _: () = msg_send![image_view, setImage: &*ns_image];
    Ok(())
}

/// 收尾（幂等）：停表、摘监视器、关窗、回传结果。
fn finish(st: &mut MacState) {
    unsafe {
        if let Some(timer) = st.timer.take() {
            let _: () = msg_send![&timer, invalidate];
        }
        if let Some(monitor) = st.monitor.take() {
            let _: () = msg_send![class!(NSEvent), removeMonitor: &*monitor];
        }
        if let Some(window) = st.window.take() {
            let _: () = msg_send![&window, orderOut: core::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![&window, close];
        }
        if let Some(tx) = st.tx.take() {
            let _ = tx.send(st.outcome);
        }
    }
}

// ---- CoreGraphics FFI（零新增 crate 之外：直接用 objc2-core-graphics 的 C 函数）----

use objc2_core_graphics::{
    CGColorSpaceCreateDeviceRGB, CGDataProviderCreateWithData, CGImageCreate,
};
