//! Windows 原生遮罩（Win32 分层窗口）。
//!
//! 移植自 shot 项目 `shot/src/main.rs`（纯 FFI、零第三方依赖、Windows 实测通过）。
//! 平台无关部分（`Sel` / `Phase` / `Op` / `App` / `normalize` / `apply_node` /
//! 节点与按钮布局）**不在此重复实现**，一律复用
//! [`crate::commands::capture::geometry`] 与 [`crate::commands::capture::state`]。
//!
//! # 物理分层
//!
//! ```text
//!   ① 冻结帧静态层（WS_EX_LAYERED|TOPMOST|TOOLWINDOW|TRANSPARENT）
//!        · UpdateLayeredWindow 画一次 frozen_bg，之后**永不重绘**
//!        · 纯显示：零输入、零业务、零命中
//!
//!   ② 遮罩控制层（WS_EX_LAYERED|TOPMOST|TOOLWINDOW，唯一输入对象）
//!        压暗底(α=0xB4) → 洞(α=0，透出下层冻结帧) → 洞外描边环 → HUD
//!        · 每次渲染只有一次 UpdateLayeredWindow
//! ```
//!
//! 洞内必须是 **alpha=0 的纯净透明**：冻结帧由下层静态层提供，洞只是「开窗」
//! 让冻结帧原样透出。若往洞里写任何半透明色，选区就会「变色」，下游按选区取
//! 冻结帧像素时永远拿不到真值。
//!
//! # 单向数据流（改代码前先看懂这条链）
//!
//! ```text
//!   [系统输入] → ① WH_MOUSE_LL / WH_KEYBOARD_LL 低级钩子（唯一输入源）
//!                  只做：状态函数 + resolve_cursor + 置脏
//!                  **禁止**绘制 / IO / 等待（回调阻塞 >300ms 会被系统静默卸载）
//!              → ② App 状态层（纯计算，不碰 GDI / 文件）
//!              → ③ WM_TIMER(16ms) tick：退出请求 → hover 探测(48ms 限频)
//!                                        → 合并渲染 → 180s 硬超时
//!              → ④ render_overlay：纯像素计算 + 一次 ULW（**渲染只在 tick**）
//!              → ⑤ run_overlay 返回 OverlayOutcome（抓屏/落盘由 session 层负责）
//! ```
//!
//! # 硬约束（违反任意一条都会重演 shot 的历史事故）
//!
//! | # | 约束 |
//! |---|------|
//! | H1 | 不引入任何第三方 crate（全部 `extern "system"` 手写声明） |
//! | H2 | 洞内零写入；边框/发光/节点/按钮一律画在洞**外** |
//! | H3 | DIB 的 `biHeight` 必须为**负**（top-down）：y=0 是屏幕最上一行 |
//! | H4 | 输入只能走低级钩子：分层+置顶+非激活窗口收不到窗口鼠标/键盘消息 |
//! | H5 | 渲染只在 tick；输入路径零同步 ULW（否则消息洪泛、桌面锁死） |
//! | H6 | 钩子内零 IO；日志一律走 `tracing`（不写盘、不缓冲刷盘） |
//!
//! # 与 shot 的移植差异
//!
//! 1. **无全局 `static mut APP`**：`App` 由本模块的 `OverlayState` 持有
//!    （见 `static mut OVERLAY` 的 SAFETY 说明）。
//! 2. **不抓屏**：`capture_original` / `BitBlt` 不移植，冻结帧由参数传入；
//!    屏幕尺寸同样取自参数（不再 `GetSystemMetrics`），避免 DPI 虚拟化误差。
//! 3. **不落盘**：裁剪 PNG / GDI+ / 输出目录 / 后台写盘线程全部移除，
//!   确认后只把选区交给调用方（session 层用 xcap 冻结帧裁剪编码）。
//! 4. **不退出宿主进程**：shot 走 `ExitProcess`，这里只销毁本模块的窗口、
//!    卸载钩子并返回 —— 宿主 Tauri 进程必须继续存活。
//! 5. **日志用 `tracing`**：移除 shot 的内存缓冲 + 文件写盘日志。
//! 6. **无兜底热键**：`RegisterHotKey` 不移植（ESC 钩子 + 180s 硬超时已覆盖）。
//! 7. 移除 shot 的 `SHOT_DEBUG_BOUNDS` 环境变量调试转储，改为 `tracing::debug`。
//! 8. 新增 `WM_SETCURSOR` 直接应答：光标完全由输入钩子里的 `SetCursor` 驱动，
//!    禁止 `DefWindowProc` 用类光标覆盖（类光标显式置 NULL）。
#![allow(dead_code)]

// 绘制相关（Rect / 节点 / 命中判定）已随渲染下沉到 `surface::paint`；
// 本模块只保留窗口/输入需要的东西。
use crate::commands::capture::geometry::{Kind, Point, Rect, Sel};
use crate::commands::capture::state::{Action, App, CursorKind, Phase};
// 契约类型（OverlayParams / OverlayOutcome）定义在 `surface::mod`（平台无关），
// 本模块只提供 Windows 实现 —— 见 `surface/mod.rs` 的解耦边界说明。
use super::{OverlayOutcome, OverlayParams};
// 视觉规格与像素渲染都在平台无关的 `surface::paint`（本文件按 `super::paint::xxx` 引用）。

use tracing::{debug, warn};

// ═══════════════════════════ §1 FFI 声明（零依赖） ═══════════════════════════

type Hwnd = *mut core::ffi::c_void;
type Hdc = *mut core::ffi::c_void;
type Hbitmap = *mut core::ffi::c_void;
type Hgdiobj = *mut core::ffi::c_void;
type Hcursor = *mut core::ffi::c_void;
type Hmodule = *mut core::ffi::c_void;
type Hhook = *mut core::ffi::c_void;
type WndProc = extern "system" fn(Hwnd, u32, usize, isize) -> isize;
type HookProc = extern "system" fn(i32, usize, isize) -> isize;

/// `POINT`（屏幕坐标，物理像素）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WinPoint {
    x: i32,
    y: i32,
}

impl WinPoint {
    const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// `RECT`（left/top 含，right/bottom 不含）。
///
/// 刻意**不复用** `geometry::Rect`：后者是纯 Rust 结构（无 `#[repr(C)]`），
/// 不能直接当 FFI 出参。只在 GetWindowRect / DwmGetWindowAttribute 用。
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl WinRect {
    /// 转成选区；宽高非正视为无效窗口。
    fn to_sel(&self) -> Option<Sel> {
        let w = self.right - self.left;
        let h = self.bottom - self.top;
        if w <= 0 || h <= 0 {
            None
        } else {
            Some(Sel::new(self.left, self.top, w, h))
        }
    }
}

/// `BITMAPINFOHEADER`（`height` 传负值 = top-down DIB）。
#[repr(C)]
#[derive(Clone, Copy)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_ppm: i32,
    y_ppm: i32,
    clr_used: u32,
    clr_important: u32,
}

/// `BLENDFUNCTION`（ULW_ALPHA 用）。
#[repr(C)]
#[derive(Clone, Copy)]
struct BlendFunction {
    op: u8,
    flags: u8,
    src_alpha: u8,
    format: u8,
}

/// `WNDCLASSW`。
#[repr(C)]
#[derive(Clone, Copy)]
struct WndClassW {
    style: u32,
    wnd_proc: Option<WndProc>,
    cls_extra: i32,
    wnd_extra: i32,
    inst: Hmodule,
    icon: Hcursor,
    cursor: Hcursor,
    background: Hgdiobj,
    menu_name: *const u16,
    class_name: *const u16,
}

/// `MSG`。
#[repr(C)]
#[derive(Clone, Copy)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: WinPoint,
}

/// `MSLLHOOKSTRUCT`。
#[repr(C)]
#[derive(Clone, Copy)]
struct MouseLLHook {
    pt: WinPoint,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

/// `KBDLLHOOKSTRUCT`。
#[repr(C)]
#[derive(Clone, Copy)]
struct KbdLLHook {
    vk: u32,
    scan: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassW(wc: *const WndClassW) -> u16;
    fn UnregisterClassW(name: *const u16, inst: Hmodule) -> i32;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        menu: Hmodule,
        inst: Hmodule,
        param: *mut core::ffi::c_void,
    ) -> Hwnd;
    fn DestroyWindow(hwnd: Hwnd) -> i32;
    fn DefWindowProcW(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize;
    fn PeekMessageW(m: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(m: *const Msg) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn PostQuitMessage(code: i32);
    fn SetWindowPos(hwnd: Hwnd, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn GetCursorPos(p: *mut WinPoint) -> i32;
    /// 鼠标键当前是否按下（返回值高位 0x8000 = 按下）。退出前用它确认按键已松开。
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetWindowLongW(hwnd: Hwnd, index: i32) -> i32;
    fn GetWindowRect(hwnd: Hwnd, r: *mut WinRect) -> i32;
    fn GetClassNameW(hwnd: Hwnd, buf: *mut u16, max: i32) -> i32;
    fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
    fn GetTopWindow(hwnd: Hwnd) -> Hwnd;
    fn GetWindow(hwnd: Hwnd, cmd: u32) -> Hwnd;
    fn GetAncestor(hwnd: Hwnd, flags: u32) -> Hwnd;
    fn IsWindowVisible(hwnd: Hwnd) -> i32;
    fn IsIconic(hwnd: Hwnd) -> i32;
    fn LoadCursorW(inst: Hmodule, name: *const u16) -> Hcursor;
    fn SetCursor(c: Hcursor) -> Hcursor;
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, dc: Hdc) -> i32;
    fn UpdateLayeredWindow(
        hwnd: Hwnd,
        dst_dc: Hdc,
        dst_pt: *const WinPoint,
        size: *const WinPoint,
        src_dc: Hdc,
        src_pt: *const WinPoint,
        color_key: u32,
        blend: *const BlendFunction,
        flags: u32,
    ) -> i32;
    fn SetTimer(
        hwnd: Hwnd,
        id: usize,
        elapse: u32,
        proc: Option<extern "system" fn(Hwnd, u32, usize, u32)>,
    ) -> usize;
    fn KillTimer(hwnd: Hwnd, id: usize) -> i32;
    fn SetWindowsHookExW(id: i32, proc: Option<HookProc>, hmod: Hmodule, tid: u32) -> Hhook;
    fn UnhookWindowsHookEx(hhk: Hhook) -> i32;
    fn CallNextHookEx(hhk: Hhook, code: i32, w: usize, l: isize) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount() -> u32;
    fn GetLastError() -> u32;
    fn GetModuleHandleW(name: *const u16) -> Hmodule;
    fn GetProcAddress(inst: Hmodule, name: *const u8) -> *mut core::ffi::c_void;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateDIBSection(
        dc: Hdc,
        info: *const BitmapInfoHeader,
        usage: u32,
        bits: *mut *mut core::ffi::c_void,
        section: Hmodule,
        offset: u32,
    ) -> Hbitmap;
    fn CreateCompatibleDC(dc: Hdc) -> Hdc;
    fn DeleteDC(dc: Hdc) -> i32;
    fn SelectObject(dc: Hdc, obj: Hgdiobj) -> Hgdiobj;
    fn DeleteObject(obj: Hgdiobj) -> i32;
    // 按钮文字用系统字体渲染（自绘 5x7 点阵画不出中文；且点阵只有 O/K 两个字模，
    // 之前 K 的位序错位还会渲染成「躺着的字」）。文字画在按钮底色之上，之后把按钮
    // 矩形的 alpha 统一补成不透明（GDI 不写 alpha 通道）。
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> Hgdiobj;
    fn SetBkMode(dc: Hdc, mode: i32) -> i32;
    fn SetTextColor(dc: Hdc, color: u32) -> u32;
    fn DrawTextW(dc: Hdc, text: *const u16, count: i32, rect: *mut WinRect, format: u32) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetWindowAttribute(hwnd: Hwnd, attr: u32, out: *mut core::ffi::c_void, size: u32) -> i32;
}

#[link(name = "shcore")]
extern "system" {
    fn SetProcessDpiAwareness(value: i32) -> i32;
}

// ═══════════════════════════ §2 常量 ═══════════════════════════

// ---- 消息 ----
const WM_DESTROY: u32 = 0x0002;
const WM_TIMER: u32 = 0x0113;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;

const HWND_TOPMOST: isize = -1;

/// `WM_MOUSEACTIVATE` 返回值：不激活本窗口，也不吞掉鼠标消息。
const MA_NOACTIVATE: isize = 3;

// ---- 虚拟键 ----
const VK_ESCAPE: u32 = 0x1B;
// GetAsyncKeyState 用：退出冷却必须等这三个键都松开（见 run_tick ①）
const VK_LBUTTON: i32 = 0x01;
const VK_RBUTTON: i32 = 0x02;
const VK_MBUTTON: i32 = 0x04;
const VK_RETURN: u32 = 0x0D;
const VK_SPACE: u32 = 0x20;

// ---- 窗口样式 ----
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const PM_REMOVE: u32 = 0x0001;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SWP_NOACTIVATE: u32 = 0x0010;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;

// ---- 节拍 ----
const TIMER_TICK: usize = 1;
const TICK_MS: u32 = 16; // ≈60fps 合并渲染节拍
const HOVER_POLL_MS: u32 = 48; // hover 探测限频（仅 Hover 相位且未按下）
const HARD_TIMEOUT_MS: u32 = 180_000; // 无论如何都能自退出，绝不要求用户杀进程
const EXIT_COOLDOWN_MS: u32 = 400; // 点 OK/X 后遮罩保留时长（吞余点击，防误触下层）
const LOOP_SLEEP_MS: u64 = 1;

// ---- DWM 属性 ----
const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
const DWMWA_CLOAKED: u32 = 14;
/// GetWindowRect 比 DWM 边界大出超过该值 → 判 DWM 漏非客户区，改用 GetWindowRect。
const SHADOW_SLACK: i32 = 24;

// ---- 钩子 / DIB / 窗口枚举 ----
const WH_MOUSE_LL: i32 = 14;
const WH_KEYBOARD_LL: i32 = 13;
const HC_ACTION: i32 = 0;
const ULW_ALPHA: u32 = 2;
const AC_SRC_OVER: u8 = 0;
const AC_SRC_ALPHA: u8 = 1;
const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;
const GW_HWNDNEXT: u32 = 2;
const GA_ROOT: u32 = 2;
const GW_OWNER: u32 = 4;
const GWL_EXSTYLE: i32 = -20;

// ---- DPI ----
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
const DPI_AWARENESS_SYSTEM_AWARE: i32 = 1;

// ---- 光标形状（LoadCursorW 的预置 id，即 MAKEINTRESOURCE 值）----
const IDC_CROSS: u32 = 32515;
const IDC_HAND: u32 = 32649;
const IDC_SIZEALL: u32 = 32646;
const IDC_SIZENWSE: u32 = 32642;
const IDC_SIZENESW: u32 = 32643;
const IDC_SIZEWE: u32 = 32644;
const IDC_SIZENS: u32 = 32645;

// ---- 视觉规格（颜色/尺寸/放大镜/HUD 参数）----
// 全部来自平台无关的 `surface::paint`（单一来源）；其 `use` 在文件顶部。

// ---- 窗口类名（唯一，避免与宿主其它窗口类冲突）----
const CLASS_STATIC: &str = "NuphusCaptureStatic";
const CLASS_OVERLAY: &str = "NuphusCaptureOverlay";

// ═══════════════════════════ §3 资源与全局状态 ═══════════════════════════

/// 一个分层窗口 + 它的 DIB section（像素由我们直接写）。
struct Layer {
    hwnd: Hwnd,
    dc: Hdc,
    bmp: Hbitmap,
    bits: *mut u32,
}

/// 退出动作（决定最终返回值；遮罩本身不落盘）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExitAct {
    /// X / ESC / 右键 / 硬超时
    Cancel,
    /// OK / 回车 / 空格
    Confirm,
}

/// 遮罩运行期的全部可变状态（唯一真源）。
struct OverlayState {
    app: App,
    screen_w: i32,
    screen_h: i32,
    static_layer: Option<Layer>,
    overlay_layer: Option<Layer>,
    mouse_hook: Hhook,
    kbd_hook: Hhook,
    exit_req: bool,
    exit_act: ExitAct,
    /// 退出冷却截止 tick（到点才真正销毁窗口，期间遮罩吞掉一切输入）。
    exit_at: Option<u32>,
    /// 单点拾取模式（取色 / 鼠标坐标）：单击=确认、选区=该点 1x1（见 ll_mouse 的 LUP）。
    point_pick: bool,
    /// 取色模式（point_pick 的子集）：额外绘制放大镜 + 实时色值条。
    pick_color: bool,
    /// 按钮文字字体句柄（GDI `HFONT`，懒创建、`finish` 时删除）。0 = 尚未创建。
    font: Hgdiobj,
    /// 启动时刻（硬超时基准）。
    t0: u32,
    /// `finish()` 幂等标志。
    done: bool,
    /// ULW 失败只告警一次（60fps 下不刷屏）。
    render_warned: bool,
}

/// 全局遮罩状态。
///
/// # 为什么是 `static mut` 而不是 `Mutex`
///
/// 1. **访问面天然单线程**：`WH_MOUSE_LL` / `WH_KEYBOARD_LL` 的回调由系统投递到
///    *安装钩子的那个线程* 的消息队列，因此钩子、`WM_TIMER` tick 与消息循环
///    必在同一条线程上**串行**执行 —— 与 shot 的 `static mut APP` 语义完全一致。
/// 2. **钩子内禁止一切可能阻塞的操作**：`Mutex::lock` 是一次潜在等待
///    （锁竞争 / 优先级反转），而 `std::sync::Mutex` 不可重入 —— 低级钩子会在
///    任意消息泵边界插入执行，一旦出现「持锁期间泵消息 → 钩子重入 → 再取同一把锁」
///    就是死锁。无锁静态变量从结构上排除该风险。
/// 3. 不使用直接 `&mut OVERLAY`（触发 `static_mut_refs` lint，CI 以
///    `-D warnings` 门禁），统一经 `core::ptr::addr_of_mut!` 取指针后解引用。
///
/// SAFETY：只允许在遮罩线程（安装钩子的线程）访问；`run_overlay` 返回前清空。
static mut OVERLAY: Option<OverlayState> = None;

/// 取全局状态的独占引用（见 `OVERLAY` 的 SAFETY 说明）。
unsafe fn overlay() -> Option<&'static mut OverlayState> {
    (*core::ptr::addr_of_mut!(OVERLAY)).as_mut()
}

/// 清空全局状态（`run_overlay` 收尾）。
unsafe fn clear_overlay() {
    *core::ptr::addr_of_mut!(OVERLAY) = None;
}

/// UTF-16 零结尾字符串。
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

// 说明：像素绘制工具（fill_rect / stroke_rect / fill_round_rect / draw_node / 字形 / 按钮标签）
// 与取色 HUD（hud_glyph / draw_hud_text / draw_color_picker_hud）已下沉到平台无关的
// `surface::paint`（三平台共用同一份实现）；本文件只保留 Windows 的窗口 / 呈现 / 输入。

// ═══════════════════════════ §5 渲染（纯像素 + 一次 ULW） ═══════════════════════════

/// 把像素缓冲一次性呈现到分层窗口（唯一的绘制机制）。
///
/// 严禁与 `SetLayeredWindowAttributes` 混用（二者互斥）。
unsafe fn ulw(layer: &Layer, w: i32, h: i32, warned: &mut bool) {
    let dst = WinPoint::new(0, 0);
    let size = WinPoint::new(w, h);
    let src_zero = WinPoint::new(0, 0);
    let blend = BlendFunction {
        op: AC_SRC_OVER,
        flags: 0,
        src_alpha: 255,
        format: AC_SRC_ALPHA,
    };
    let ret = UpdateLayeredWindow(
        layer.hwnd,
        core::ptr::null_mut(),
        &dst,
        &size,
        layer.dc,
        &src_zero,
        0,
        &blend,
        ULW_ALPHA,
    );
    if ret == 0 && !*warned {
        *warned = true;
        warn!(
            error = GetLastError(),
            "[capture] UpdateLayeredWindow 失败（后续不再重复告警）"
        );
    }
}

/// 在两个按钮上绘制中文文案（系统字体，GDI）+ 修正按钮区域 alpha。
///
/// 为什么必须补 alpha：`paint` 写的是 `UpdateLayeredWindow` 用的逐像素 alpha 缓冲，
/// 而 GDI 绘制**不写 alpha 字节** → 文字 alpha=0，在 ULW 下完全透明（看不见）。
/// 按钮底色本来就是不透明色，所以把按钮矩形整体 alpha 置 0xFF 是安全的。
unsafe fn draw_btn_text(
    font: &mut Hgdiobj,
    dc: Hdc,
    ok: Rect,
    x_rect: Rect,
    buf: &mut [u32],
    bw: i32,
    bh: i32,
) {
    if font.is_null() {
        // 微软雅黑 15px（负值 = 字符高度口径）；DEFAULT_CHARSET 保证中文可渲染。
        let face = wide("Microsoft YaHei UI");
        *font = CreateFontW(-15, 0, 0, 0, 500, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
        if font.is_null() {
            warn!("[capture] 按钮字体创建失败，按钮将只有底色");
            return;
        }
    }
    let old = SelectObject(dc, *font);
    SetBkMode(dc, 1); // TRANSPARENT：不填背景，只画字形
                      // 文字颜色与三平台统一来源（`paint::COLOR_BTN_EDGE`，0xAARRGGBB）
                      // → GDI 要的 COLORREF 是 0x00BBGGRR，这里做一次字节序转换。
    let c = super::paint::COLOR_BTN_EDGE;
    SetTextColor(dc, ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF));
    draw_btn_text_one(dc, ok, super::paint::BTN_OK_LABEL);
    draw_btn_text_one(dc, x_rect, super::paint::BTN_X_LABEL);
    SelectObject(dc, old);

    // 按钮区域 alpha 补成不透明（GDI 不写 alpha；底色本就不透明，无副作用）
    for r in [ok, x_rect] {
        let l = r.left.max(0);
        let t = r.top.max(0);
        let ri = r.right.min(bw);
        let bt = r.bottom.min(bh);
        for y in t..bt {
            let row = (y as usize) * (bw as usize);
            for x in l..ri {
                buf[row + x as usize] |= 0xFF00_0000;
            }
        }
    }
}

/// 单个按钮：`DrawTextW` 居中绘制（DT_CENTER | DT_VCENTER | DT_SINGLELINE）。
unsafe fn draw_btn_text_one(dc: Hdc, r: Rect, label: &str) {
    let text = wide(label);
    let mut rc = WinRect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    };
    DrawTextW(dc, text.as_ptr(), -1, &mut rc, 0x0001 | 0x0004 | 0x0020);
}

/// 重绘控制层：取缓冲 → 平台无关渲染（`paint::render`）→ 一次 ULW。
///
/// 渲染规则（压暗底 / 洞 / 描边环 / 节点按钮 / 取色 HUD）全部在 `surface::paint`，
/// 本函数只负责「拿本平台的两块 DIB → 交给共用渲染 → 呈现」，这也是 macOS / Linux
/// 实现需要各自完成的最后一步。
unsafe fn render_overlay(ov: &mut OverlayState) {
    let bw = ov.screen_w;
    let bh = ov.screen_h;
    if bw <= 0 || bh <= 0 {
        return;
    }
    let Some(layer) = ov.overlay_layer.as_ref() else {
        return;
    };
    let n = (bw as usize) * (bh as usize);
    let buf = core::slice::from_raw_parts_mut(layer.bits, n);
    // 冻结帧（取色 HUD 放大镜的采样源）：与 overlay 层是两块独立 DIB，无别名冲突。
    let frozen: &[u32] = match ov.static_layer.as_ref() {
        Some(l) => core::slice::from_raw_parts(l.bits, n),
        None => &[],
    };
    super::paint::render(
        buf,
        bw,
        bh,
        &mut ov.app,
        ov.point_pick,
        ov.pick_color,
        frozen,
    );

    // 按钮文字（确定 / 取消）：系统字体画在按钮底色之上，随后补 alpha（见函数说明）。
    // 字段分离借用：`ov.app` 的按钮矩形是读、`ov.font` 可变、`layer.dc` 借自 overlay_layer。
    if !ov.point_pick
        && matches!(
            ov.app.phase,
            Phase::Confirm | Phase::Moving | Phase::Adjusting
        )
    {
        let (ok_rect, x_rect) = (ov.app.btn_ok, ov.app.btn_x);
        draw_btn_text(&mut ov.font, layer.dc, ok_rect, x_rect, buf, bw, bh);
    }

    // 一次 ULW（唯一的呈现动作）
    let mut warned = ov.render_warned;
    ulw(layer, bw, bh, &mut warned);
    ov.render_warned = warned;
}

// ═══════════════════════════ §6 hover 探测（窗口枚举） ═══════════════════════════

/// 桌面外壳窗口类：命中它们等于「没有窗口」，一律过滤（否则 hover 会框住整个桌面）。
const SHELL_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Windows.UI.Core.CoreWindow",
    "Shell_TrayWnd",
];

fn is_shell_class(name: &str) -> bool {
    SHELL_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(name))
}

fn class_name_of(hwnd: Hwnd) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), 256);
        if n <= 0 {
            return None;
        }
        let mut s = String::with_capacity(n as usize);
        for c in &buf[..n as usize] {
            s.push(char::from_u32(*c as u32).unwrap_or('?'));
        }
        Some(s)
    }
}

/// 该窗口是否属于本进程（本模块的两个分层窗口 + 宿主主窗都必须跳过）。
unsafe fn is_self_window(hwnd: Hwnd) -> bool {
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    pid == GetCurrentProcessId()
}

/// 窗口是否被 DWM cloaked（UWP 幽灵窗口 / 虚拟桌面切换残留）。
unsafe fn is_cloaked(hwnd: Hwnd) -> bool {
    let mut cloaked: u32 = 0;
    let r = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut core::ffi::c_void,
        core::mem::size_of::<u32>() as u32,
    );
    r == 0 && cloaked != 0
}

/// 取窗口真实边界：DWM 优先，DWM 漏标题栏时回退 GetWindowRect。
///
/// 判据（实测）：DWM 的 EXTENDED_FRAME_BOUNDS 对标准窗口会**多算**不可见阴影
/// （每侧约 7px），但对 Tauri/Chromium 这类自绘窗口又会**少算**非客户区。
/// 因此两者都取：若 GetWindowRect 比 DWM 大出超过 SHADOW_SLACK(24px)，
/// 判 DWM 漏血 → 采用 GetWindowRect；否则采用 DWM（避开阴影）。
unsafe fn window_bounds(hwnd: Hwnd) -> Option<Sel> {
    let mut gr = WinRect::default();
    let got_gr = GetWindowRect(hwnd, &mut gr) != 0;
    let mut fr = WinRect::default();
    let got_fr = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut fr as *mut WinRect as *mut core::ffi::c_void,
        core::mem::size_of::<WinRect>() as u32,
    ) == 0;
    if !got_gr && !got_fr {
        return None;
    }
    let r = if got_gr && got_fr {
        let grow = (gr.right - gr.left).max(gr.bottom - gr.top);
        let frow = (fr.right - fr.left).max(fr.bottom - fr.top);
        if grow - frow > SHADOW_SLACK {
            gr
        } else {
            fr
        }
    } else if got_gr {
        gr
    } else {
        fr
    };
    r.to_sel()
}

/// hover 候选过滤（shot 备份 V1 的判定集，8 条一条不改）：
/// 自身窗口 / 不可见 / 最小化 / 有所有者 / 工具窗口 / 非根窗口 / 被 cloaked / shell 类。
/// 少任意一条都会出现「鼠标明明在窗口上却 fallback 全屏」。
unsafe fn is_hover_candidate(hwnd: Hwnd) -> bool {
    if is_self_window(hwnd) {
        return false;
    }
    if IsWindowVisible(hwnd) == 0 {
        return false;
    }
    if IsIconic(hwnd) != 0 {
        return false;
    }
    if !GetWindow(hwnd, GW_OWNER).is_null() {
        return false;
    }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW != 0 {
        return false;
    }
    if GetAncestor(hwnd, GA_ROOT) != hwnd {
        return false;
    }
    if is_cloaked(hwnd) {
        return false;
    }
    if let Some(cn) = class_name_of(hwnd) {
        if is_shell_class(&cn) {
            return false;
        }
    }
    true
}

/// Z 序自顶向下找第一个「过滤通过 且 包含鼠标点」的窗口。
///
/// 返回 `(命中的选区, 遍历窗口数, 诊断串)`。诊断串只在未命中时由调用方按
/// `debug` 级别输出，用于回答「为什么 fallback 全屏」这类必须看现场的问题。
unsafe fn find_window_under_diag(pt: Point) -> (Option<Sel>, usize, String) {
    let mut scanned = 0usize;
    let mut diag = String::new();
    let mut hwnd = GetTopWindow(core::ptr::null_mut());
    while !hwnd.is_null() {
        scanned += 1;
        if is_hover_candidate(hwnd) {
            if let Some(b) = window_bounds(hwnd) {
                if b.contains(pt.x, pt.y) {
                    return (Some(b), scanned, diag);
                }
            }
        } else if !is_self_window(hwnd)
            && IsWindowVisible(hwnd) != 0
            && IsIconic(hwnd) == 0
            && !is_cloaked(hwnd)
        {
            // 只记录「看起来该命中却被过滤」的候选，避免日志爆炸
            if let Some(b) = window_bounds(hwnd) {
                if b.contains(pt.x, pt.y) {
                    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                    let owner = !GetWindow(hwnd, GW_OWNER).is_null();
                    let notroot = GetAncestor(hwnd, GA_ROOT) != hwnd;
                    let cn = class_name_of(hwnd).unwrap_or_default();
                    diag.push_str(&format!(
                        "| REJECT cls={} rect=({},{},{}x{}) owner={} toolwin={} notroot={} ex=0x{:X}",
                        cn, b.x, b.y, b.w, b.h, owner, ex & WS_EX_TOOLWINDOW != 0, notroot, ex
                    ));
                }
            }
        }
        hwnd = GetWindow(hwnd, GW_HWNDNEXT);
    }
    (None, scanned, diag)
}

// ═══════════════════════════ §7 光标语义 ═══════════════════════════

/// 节点 → 对应方向的双向缩放光标。
fn node_cursor_id(k: Kind) -> u32 {
    match k {
        Kind::NW | Kind::SE => IDC_SIZENWSE,
        Kind::NE | Kind::SW => IDC_SIZENESW,
        Kind::W | Kind::E => IDC_SIZEWE,
        Kind::N | Kind::S => IDC_SIZENS,
    }
}

unsafe fn apply_cursor(id: u32) {
    let cur = LoadCursorW(core::ptr::null_mut(), id as *const u16);
    if !cur.is_null() {
        SetCursor(cur);
    }
}

/// 依据当前相位与命中目标设置系统光标形状。
///
/// 输入走低级钩子（分层+非激活窗口收不到窗口消息），所以光标也只能在这里设 ——
/// 窗口类上那把静态箭头不会随语义变化，故类光标记为 NULL 且 `WM_SETCURSOR` 直接应答。
unsafe fn resolve_cursor(app: &App, px: i32, py: i32) {
    let id = match app.cursor_kind(px, py) {
        CursorKind::Cross => IDC_CROSS,
        CursorKind::Hand => IDC_HAND,
        CursorKind::SizeAll => IDC_SIZEALL,
        CursorKind::Resize(k) => node_cursor_id(k),
    };
    apply_cursor(id);
}

// ═══════════════════════════ §8 输入钩子（唯一输入通道） ═══════════════════════════

/// 请求退出：只置标志，真正执行在 tick（清理路径唯一）。
fn request_exit(ov: &mut OverlayState, act: ExitAct) {
    ov.exit_req = true;
    ov.exit_act = act;
}

/// 低级鼠标钩子：**只调状态函数 + 置脏**。禁止在此绘制 / 等待 / 做 IO。
extern "system" fn ll_mouse(code: i32, w: usize, l: isize) -> isize {
    unsafe {
        if code == HC_ACTION && l != 0 {
            let info = &*(l as *const MouseLLHook);
            if let Some(ov) = overlay() {
                let (px, py) = (info.pt.x, info.pt.y);
                match w as u32 {
                    WM_MOUSEMOVE => {
                        ov.app.last_mouse = Point { x: px, y: py };
                        resolve_cursor(&ov.app, px, py);
                        ov.app.on_move(px, py, ov.screen_w, ov.screen_h);
                        // 单点拾取 HUD（放大镜/色值）跟随鼠标：Hover 相位下 `on_move`
                        // 不动状态、也不置脏，这里显式置脏（重绘仍由 tick 16ms 合并）。
                        if ov.point_pick {
                            ov.app.dirty = true;
                        }
                    }
                    WM_LBUTTONDOWN => {
                        ov.app.last_mouse = Point { x: px, y: py };
                        resolve_cursor(&ov.app, px, py);
                        let act = ov.app.on_ldown(px, py, ov.screen_w, ov.screen_h);
                        match act {
                            Action::ConfirmSave => request_exit(ov, ExitAct::Confirm),
                            Action::Cancel => request_exit(ov, ExitAct::Cancel),
                            Action::None => {}
                        }
                    }
                    WM_LBUTTONUP => {
                        if ov.point_pick {
                            // 单点拾取（取色 / 鼠标坐标）：松手即确认，选区 = 该点 1x1。
                            // 不画 OK/X、不需二次点击（点哪算哪）。
                            ov.app.pick_point_at(px, py);
                            request_exit(ov, ExitAct::Confirm);
                        } else {
                            ov.app.on_lup(px, py);
                        }
                    }
                    WM_RBUTTONDOWN => request_exit(ov, ExitAct::Cancel),
                    _ => {}
                }
                // 交互中保持脏标记，tick 16ms 内必然重绘（≈60fps，输入侧零绘制）
                let busy = matches!(
                    ov.app.phase,
                    Phase::Dragging | Phase::Moving | Phase::Adjusting
                );
                if ov.app.dirty || ov.app.pending || busy {
                    ov.app.dirty = true;
                }
                // ★ 只吞「按钮 / 滚轮」消息（返回非零 = 不再传给下层窗口）。
                //
                // 为什么「窗口盖住屏幕」挡不住点击穿透：控制层是 UpdateLayeredWindow
                // 分层窗口，命中测试按**逐像素 alpha** 判定 —— alpha=0 的区域
                // （矩形模式的洞内、取色/坐标模式的整屏）对鼠标是**穿透**的
                // （MSDN: … areas whose alpha value is zero will let the mouse messages
                //  through），于是右键 DOWN/UP 直达下层应用/桌面 → 弹右键菜单。
                //
                // ⚠️ 但**鼠标移动必须放行**：全吞（含 WM_MOUSEMOVE）会让系统与应用的
                //    鼠标移动语义整体失效（实测：鼠标移动失去响应）。移动消息不会造成
                //    「点穿」，放行到下层最多只是 hover 高亮，无副作用。
                return match w as u32 {
                    WM_MOUSEMOVE => CallNextHookEx(ov.mouse_hook, code, w, l),
                    _ => 1,
                };
            }
        }
        CallNextHookEx(core::ptr::null_mut(), code, w, l)
    }
}

/// 低级键盘钩子：ESC 取消；回车/空格 = 确认（仅在确认/调整态生效）。
extern "system" fn ll_kbd(code: i32, w: usize, l: isize) -> isize {
    unsafe {
        if code == HC_ACTION && l != 0 {
            let info = &*(l as *const KbdLLHook);
            if let Some(ov) = overlay() {
                match info.vk {
                    VK_ESCAPE => request_exit(ov, ExitAct::Cancel),
                    VK_RETURN | VK_SPACE => {
                        if matches!(
                            ov.app.phase,
                            Phase::Confirm | Phase::Moving | Phase::Adjusting
                        ) {
                            ov.app.enter_exiting();
                            request_exit(ov, ExitAct::Confirm);
                        }
                    }
                    _ => {}
                }
                return CallNextHookEx(ov.kbd_hook, code, w, l);
            }
        }
        CallNextHookEx(core::ptr::null_mut(), code, w, l)
    }
}

/// 安装两个低级钩子；鼠标钩子是硬需求（没有它就收不到任何点击）。
unsafe fn install_hooks(ov: &mut OverlayState) -> bool {
    let hmod = GetModuleHandleW(core::ptr::null_mut());
    ov.mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(ll_mouse), hmod, 0);
    ov.kbd_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_kbd), hmod, 0);
    debug!(
        error = GetLastError(),
        mouse_ok = !ov.mouse_hook.is_null(),
        kbd_ok = !ov.kbd_hook.is_null(),
        "[capture] 低级钩子安装"
    );
    if !ov.mouse_hook.is_null() && ov.kbd_hook.is_null() {
        warn!("[capture] 键盘钩子安装失败：ESC 不可用（回车/空格亦不可用），仍有 180s 硬超时兜底");
    }
    !ov.mouse_hook.is_null()
}

unsafe fn remove_hooks(ov: &mut OverlayState) {
    if !ov.mouse_hook.is_null() {
        UnhookWindowsHookEx(ov.mouse_hook);
        ov.mouse_hook = core::ptr::null_mut();
    }
    if !ov.kbd_hook.is_null() {
        UnhookWindowsHookEx(ov.kbd_hook);
        ov.kbd_hook = core::ptr::null_mut();
    }
}

// ═══════════════════════════ §9 节拍 / 收尾 / 消息循环 ═══════════════════════════

/// 鼠标左/右/中键是否有任一个仍按下（`GetAsyncKeyState` 系统真值，不依赖钩子事件）。
///
/// 退出冷却必须以它为闸门：只要还有键按着，遮罩就不能销毁，否则随后的松手事件
/// 会落到下层窗口/桌面（右键即弹菜单）。
unsafe fn any_mouse_btn_down() -> bool {
    (GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000) != 0
        || (GetAsyncKeyState(VK_RBUTTON) as u16 & 0x8000) != 0
        || (GetAsyncKeyState(VK_MBUTTON) as u16 & 0x8000) != 0
}

/// 16ms 节拍：退出请求 → 退出冷却 → hover 探测（48ms 限频）→ 合并渲染 → 硬超时。
///
/// 顺序固定，不可调换：退出判定必须在渲染之前（撤 HUD 后还要留着遮罩吞余点击）。
unsafe fn run_tick() {
    let Some(ov) = overlay() else {
        return;
    };
    let now = GetTickCount();

    // ① 退出请求（钩子只置标志，这里执行；清理路径唯一）
    //
    // **必须等鼠标键全部松开才起冷却**：否则「按住右键超过 EXIT_COOLDOWN_MS 再松手」时，
    // 遮罩已按 400ms 到点销毁，RIGHT-UP 就落到下层窗口/桌面 → 弹出右键菜单。
    // （2026-10-05 真机实测：按住 450ms 时遮罩句柄已为 0，松手即弹桌面菜单；快速点击
    //   <400ms 时遮罩仍在、无菜单。）按键状态取 GetAsyncKeyState 系统真值，不依赖钩子事件。
    if ov.exit_req {
        ov.app.enter_exiting(); // 幂等：立即撤 HUD 并吞掉后续输入
        if !any_mouse_btn_down() {
            ov.exit_req = false;
            ov.exit_at = Some(now.wrapping_add(EXIT_COOLDOWN_MS));
            debug!(act = ?ov.exit_act, "[capture] 退出请求（鼠标键已松开），进入 {}ms 冷却", EXIT_COOLDOWN_MS);
        }
    }

    // ② 冷却到点 → 真正销毁。用带符号差值判定，避免 wrapping 方向歧义。
    //    这 400ms 内一切输入都被遮罩吞掉（Exiting 相位下状态机全部空操作），
    //    绝不把松手/余点击漏给下层应用、更不会激活下层窗口。
    if let Some(at) = ov.exit_at {
        if now.wrapping_sub(at) < 0x8000_0000 {
            finish(ov);
            return;
        }
    }

    // ③ hover 探测：仅 Hover 相位 + 未按下 + 限频（按下即冻结选区）
    //
    // 单点拾取（取色 / 鼠标坐标）**不做窗口探测**：选区固定为整屏（洞 = 整屏），
    // 交互交付物只有鼠标处那一个像素 —— 若仍按窗口框选，会退化成「选区工具」
    // （取色模式用户看到的应是原画面 + 放大镜色值，不是被框住的窗口）。
    if ov.app.phase == Phase::Hover
        && !ov.app.pending
        && !ov.point_pick
        && now.wrapping_sub(ov.app.last_hover_ms) >= HOVER_POLL_MS
    {
        ov.app.last_hover_ms = now;
        let mut pt = WinPoint::new(0, 0);
        if GetCursorPos(&mut pt) != 0
            && (pt.x != ov.app.last_mouse.x || pt.y != ov.app.last_mouse.y)
        {
            ov.app.last_mouse = Point { x: pt.x, y: pt.y };
            let (hit, scanned, diag) = find_window_under_diag(Point { x: pt.x, y: pt.y });
            if hit.is_none() && !diag.is_empty() {
                debug!(
                    "[capture] hover 未命中 ({},{}) scanned={}{}",
                    pt.x, pt.y, scanned, diag
                );
            }
            ov.app.set_hover_target(hit, ov.screen_w, ov.screen_h);
        }
    }

    // ④ 合并渲染（≤60fps；输入侧从不直接绘制）
    if ov.app.dirty {
        ov.app.dirty = false;
        render_overlay(ov);
    }

    // ⑤ 硬超时：无论如何都能自己退，绝不需要用户杀进程
    if now.wrapping_sub(ov.t0) >= HARD_TIMEOUT_MS {
        warn!("[capture] 硬超时（{}ms），强制结束遮罩", HARD_TIMEOUT_MS);
        finish(ov);
    }
}

/// 唯一收尾通道（幂等）：卸载钩子 → 销毁窗口 → 注销窗口类 → 结束消息循环。
unsafe fn finish(ov: &mut OverlayState) {
    if ov.done {
        return;
    }
    ov.done = true;
    remove_hooks(ov);
    // 按钮字体（GDI 对象）随遮罩一起释放，否则每次截图泄漏一个 HFONT。
    if !ov.font.is_null() {
        DeleteObject(ov.font);
        ov.font = core::ptr::null_mut();
    }
    free_layer(&mut ov.overlay_layer);
    free_layer(&mut ov.static_layer);
    // 类名唯一且只由本模块注册；窗口已全部销毁，注销一定成功（失败也无害）。
    let inst = GetModuleHandleW(core::ptr::null_mut());
    if !inst.is_null() {
        UnregisterClassW(wide(CLASS_OVERLAY).as_ptr(), inst);
        UnregisterClassW(wide(CLASS_STATIC).as_ptr(), inst);
    }
    // 结束消息循环（DestroyWindow 已同步触发 WM_DESTROY → PostQuitMessage，这里兜底）
    PostQuitMessage(0);
    debug!("[capture] 遮罩已收尾");
}

/// 控制层窗口过程：只做节拍与生命周期。**输入判定一律不在这里**（H4）。
extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    unsafe {
        match msg {
            WM_TIMER => {
                if w == TIMER_TICK {
                    run_tick();
                }
                0
            }
            WM_ERASEBKGND => 1,
            // 输入走低级钩子，光标完全由 resolve_cursor → SetCursor 管理；
            // 直接应答「已处理」，禁止 DefWindowProc 用类光标覆盖我们设的形状。
            WM_SETCURSOR => 1,
            // 点击不激活本窗口：遮罩只是盖在屏幕上的「画」，不能把用户当前应用的
            // 焦点抢走（输入本来就全走低级钩子，不依赖激活）。
            WM_MOUSEACTIVATE => MA_NOACTIVATE,
            WM_DESTROY => {
                KillTimer(hwnd, TIMER_TICK);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn run_message_loop() -> i32 {
    let mut msg: Msg = core::mem::zeroed();
    loop {
        if PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                return msg.w_param as i32;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(LOOP_SLEEP_MS));
        }
    }
}

// ═══════════════════════════ §10 窗口创建与公开入口 ═══════════════════════════

/// 创建分层窗口 + top-down DIB section。
///
/// `biHeight` 必须为**负**（top-down）：所有像素写入都按「y=0 是屏幕最上一行」
/// 计算，传正值会让整层上下翻转（遮罩错位）。
unsafe fn make_layer(
    class: &str,
    title: &str,
    inst: Hmodule,
    w: i32,
    h: i32,
    transparent: bool,
) -> Option<Layer> {
    let cls = wide(class);
    let ttl = wide(title);
    let wc = WndClassW {
        style: 0,
        wnd_proc: Some(wnd_proc),
        cls_extra: 0,
        wnd_extra: 0,
        inst,
        icon: core::ptr::null_mut(),
        // 类光标显式置 NULL：光标形状只由输入钩子里的 SetCursor 决定
        // （见 wnd_proc 的 WM_SETCURSOR 应答）。
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: cls.as_ptr(),
    };
    // 两个窗口类共用同一个窗口过程：静态层只收 WM_DESTROY/WM_ERASEBKGND。
    if RegisterClassW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
        warn!(
            class,
            error = GetLastError(),
            "[capture] RegisterClassW 失败"
        );
        return None;
    }

    let ex = WS_EX_LAYERED
        | WS_EX_TOPMOST
        | WS_EX_TOOLWINDOW
        | if transparent { WS_EX_TRANSPARENT } else { 0 };
    let hwnd = CreateWindowExW(
        ex,
        cls.as_ptr(),
        ttl.as_ptr(),
        WS_POPUP,
        0,
        0,
        w,
        h,
        core::ptr::null_mut(),
        core::ptr::null_mut(),
        inst,
        core::ptr::null_mut(),
    );
    if hwnd.is_null() {
        warn!(
            class,
            error = GetLastError(),
            "[capture] CreateWindowExW 失败"
        );
        return None;
    }
    // 置顶 + 显示，且**不激活**（不抢用户当前应用的焦点）
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        w,
        h,
        SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );

    let scr = GetDC(core::ptr::null_mut());
    if scr.is_null() {
        DestroyWindow(hwnd);
        return None;
    }
    let dc = CreateCompatibleDC(scr);
    ReleaseDC(core::ptr::null_mut(), scr);
    if dc.is_null() {
        DestroyWindow(hwnd);
        return None;
    }

    let bi = BitmapInfoHeader {
        size: core::mem::size_of::<BitmapInfoHeader>() as u32,
        width: w,
        height: -h, // 负值 = top-down（H3）
        planes: 1,
        bit_count: 32,
        compression: BI_RGB,
        size_image: 0,
        x_ppm: 0,
        y_ppm: 0,
        clr_used: 0,
        clr_important: 0,
    };
    let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
    let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, core::ptr::null_mut(), 0);
    if bmp.is_null() || bits.is_null() {
        DeleteDC(dc);
        DestroyWindow(hwnd);
        return None;
    }
    SelectObject(dc, bmp); // ULW 需要 DIB 已选入 DC
    Some(Layer {
        hwnd,
        dc,
        bmp,
        bits: bits as *mut u32,
    })
}

/// 销毁层并释放 GDI 资源。
unsafe fn free_layer(l: &mut Option<Layer>) {
    if let Some(layer) = l.take() {
        if !layer.hwnd.is_null() {
            DestroyWindow(layer.hwnd);
        }
        if !layer.bmp.is_null() {
            DeleteObject(layer.bmp);
        }
        if !layer.dc.is_null() {
            DeleteDC(layer.dc);
        }
    }
}

/// DPI 感知：PerMonitorV2 优先，降级 SystemAware。
///
/// 宿主（Tauri）通常在进程启动时已设置，这里再设会返回失败 —— 无害的幂等兜底，
/// 只影响窗口物理像素尺寸的准确性（屏幕尺寸本身取自参数，不受 DPI 虚拟化影响）。
unsafe fn ensure_dpi() {
    let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
    if !user32.is_null() {
        let f = GetProcAddress(user32, b"SetProcessDpiAwarenessContext\0".as_ptr());
        if !f.is_null() {
            let f: extern "system" fn(isize) -> i32 = core::mem::transmute(f);
            if f(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
                return;
            }
        }
    }
    let _ = SetProcessDpiAwareness(DPI_AWARENESS_SYSTEM_AWARE);
}

// OverlayParams / OverlayOutcome 已上提到 `surface/mod.rs`（平台无关契约，三平台共享）。

/// 在当前线程运行遮罩直到用户确认或取消（**阻塞**；内部跑消息循环）。
///
/// 调用方负责在独立线程调用它，并在调用前完成「隐藏主窗 + 抓屏」，
/// 以避免把自己的窗口冻进冻结帧。
///
/// 返回值与清理：无论何种退出路径，返回前一定已销毁本模块的全部窗口、
/// 卸载钩子并清空全局状态；宿主进程**不会**被退出。
pub fn run_overlay(params: OverlayParams) -> OverlayOutcome {
    unsafe { run(params) }
}

/// `run_overlay` 的实现体（全部 FFI 调用集中于此路径内）。
unsafe fn run(params: OverlayParams) -> OverlayOutcome {
    let (w, h) = (params.screen_w, params.screen_h);
    if w <= 0 || h <= 0 {
        warn!("[capture] 屏幕尺寸非法：{}x{}", w, h);
        return OverlayOutcome::Cancelled;
    }
    if params.frozen_bg.len() != (w as usize) * (h as usize) {
        warn!(
            "[capture] 冻结帧长度与屏幕尺寸不符：{} != {}x{}={}",
            params.frozen_bg.len(),
            w,
            h,
            (w as usize) * (h as usize)
        );
        return OverlayOutcome::Cancelled;
    }
    // 同一时刻只允许一个遮罩（OVERLAY 是单例）
    if overlay().is_some() {
        warn!("[capture] 已有遮罩在运行，拒绝重复启动");
        return OverlayOutcome::Cancelled;
    }

    ensure_dpi();
    let inst = GetModuleHandleW(core::ptr::null_mut());
    if inst.is_null() {
        warn!("[capture] GetModuleHandleW 失败");
        return OverlayOutcome::Cancelled;
    }

    // ① 静态层：冻结帧（纯显示、不抢输入），画一次，之后永不重绘
    let stat = match make_layer(CLASS_STATIC, "nuphus capture static", inst, w, h, true) {
        Some(l) => l,
        None => {
            warn!("[capture] 静态层创建失败");
            return OverlayOutcome::Cancelled;
        }
    };
    {
        let n = (w as usize) * (h as usize);
        let dst = core::slice::from_raw_parts_mut(stat.bits, n);
        dst.copy_from_slice(&params.frozen_bg);
    }
    let mut warned = false;
    ulw(&stat, w, h, &mut warned);

    // ② 控制层：压暗 + 洞 + HUD（洞内 alpha=0 透出下层冻结帧）
    let ovl = match make_layer(CLASS_OVERLAY, "nuphus capture overlay", inst, w, h, false) {
        Some(l) => l,
        None => {
            warn!("[capture] 控制层创建失败");
            let mut stat = Some(stat);
            free_layer(&mut stat);
            return OverlayOutcome::Cancelled;
        }
    };

    *core::ptr::addr_of_mut!(OVERLAY) = Some(OverlayState {
        app: App::new(w, h),
        screen_w: w,
        screen_h: h,
        static_layer: Some(stat),
        overlay_layer: Some(ovl),
        mouse_hook: core::ptr::null_mut(),
        kbd_hook: core::ptr::null_mut(),
        exit_req: false,
        exit_act: ExitAct::Cancel,
        exit_at: None,
        point_pick: params.point_pick,
        pick_color: params.pick_color,
        font: core::ptr::null_mut(),
        t0: GetTickCount(),
        done: false,
        render_warned: false,
    });

    // ③ 首帧（此后渲染只在 tick 发生）
    if let Some(ov) = overlay() {
        render_overlay(ov);
    }

    // ④ 输入通道（本模块唯一可靠输入源）
    let hooks_ok = match overlay() {
        Some(ov) => install_hooks(ov),
        None => false,
    };
    if !hooks_ok {
        warn!("[capture] 低级鼠标钩子安装失败，无法接收输入，直接取消");
        if let Some(ov) = overlay() {
            finish(ov);
        }
        clear_overlay();
        return OverlayOutcome::Cancelled;
    }

    // ⑤ 节拍。渲染与硬超时全由 tick 驱动，装不上定时器必须立刻收尾，
    //    否则遮罩会「永远停在首帧、既不能点也不能退」。
    let timer_ok = match overlay() {
        Some(ov) => match ov.overlay_layer.as_ref() {
            Some(l) => SetTimer(l.hwnd, TIMER_TICK, TICK_MS, None) != 0,
            None => false,
        },
        None => false,
    };
    if !timer_ok {
        warn!("[capture] SetTimer 失败，遮罩无法自驱动，直接取消");
        if let Some(ov) = overlay() {
            finish(ov);
        }
        clear_overlay();
        return OverlayOutcome::Cancelled;
    }

    // ⑥ 消息循环（阻塞至 finish 发出 WM_QUIT）
    //
    // panic 兜底（关键）：低级鼠标/键盘钩子在**安装线程**上被系统调用。若遮罩线程因
    // panic 提前退出而钩子没卸载，系统会把后续鼠标/键盘输入一直卡在这个钩子上 ——
    // 用户看到的现象就是「鼠标不能移动 / 系统输入卡死」。所以 panic 必须先完成收尾
    // （卸载钩子 + 销毁窗口）再向上抛。
    let loop_result =
        unsafe { std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_message_loop())) };
    if loop_result.is_err() {
        warn!("[capture] 遮罩消息循环 panic，强制执行收尾（卸载钩子 + 销毁窗口）");
        if let Some(ov) = overlay() {
            if !ov.done {
                finish(ov);
            }
        }
    }

    // ⑦ 收尾（幂等）+ 结果
    let outcome = match overlay() {
        Some(ov) => {
            if !ov.done {
                finish(ov);
            }
            if ov.exit_act == ExitAct::Confirm {
                OverlayOutcome::Confirmed {
                    sel: (ov.app.sel.x, ov.app.sel.y, ov.app.sel.w, ov.app.sel.h),
                }
            } else {
                OverlayOutcome::Cancelled
            }
        }
        None => OverlayOutcome::Cancelled,
    };
    clear_overlay();
    outcome
}
