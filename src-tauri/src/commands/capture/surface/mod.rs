//! 遮罩表面抽象 —— **平台无关契约** + **平台相关实现**。
//!
//! ```text
//! 平台无关（三平台共用，可单测）
//!   ├── capture/geometry.rs   Sel/Rect/Point + 节点与按钮布局
//!   ├── capture/state.rs      Phase 状态机 + Action 输出
//!   └── surface/mod.rs        契约：OverlayParams / OverlayOutcome / run_overlay
//!
//! 平台实现（同一契约签名，各自原生实现）
//!   ├── windows.rs   Win32 分层窗口（UpdateLayeredWindow，像素精确）
//!   ├── macos.rs     AppKit 无边框窗口 + CGEventTap + CGBitmapContext（实现中）
//!   ├── linux.rs     X11 override-redirect + x11rb（实现中；Wayland 受限）
//!   └── fallback.rs  其它平台：回退 WebView overlay 链路
//! ```
//!
//! # 解耦边界（本次重构的目的）
//!
//! - `session.rs`（编排）与 `commands/toolbar.rs`（路由）**只依赖本文件的契约类型**，
//!   不再引用任何平台的窗口/输入/呈现细节；
//! - 各平台实现只依赖 `geometry` / `state` + 自身 FFI（Windows: user32/gdi32；
//!   macOS: objc2-app-kit 等；Linux: x11rb）；
//! - 契约签名唯一：`run_overlay(OverlayParams) -> OverlayOutcome`。
//!
//! # 退役计划
//!
//! macOS / Linux 原生实现**真机验证通过**后：删除本文件对 `fallback` 的分派、
//! `commands/toolbar.rs` 的旧 WebView 分支与前端 `frontend/src/capture-overlay/`。
//! 在此之前保留 WebView 链路作为可用性兜底（无真机验证前不切主链路）。

// ── 平台无关：视觉规格（颜色/尺寸/放大镜/HUD 参数），三平台共用 ──
pub mod paint;

// ── 平台实现（各自原生，同一契约签名）──

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub mod fallback;

// ── 统一契约（平台无关，三平台共享；改动此处即改动三平台的行为边界）──

/// 遮罩运行参数。
pub struct OverlayParams {
    /// 全屏冻结帧（top-down BGRA，长度必须 = `screen_w * screen_h`）。
    /// 由 session 层用 xcap 抓取后传入；遮罩本身只负责显示，不抓屏。
    pub frozen_bg: Vec<u32>,
    /// 屏幕物理像素宽（不是逻辑像素，DIP 缩放后的值会导致遮罩错位）。
    pub screen_w: i32,
    /// 屏幕物理像素高。
    pub screen_h: i32,
    /// 单点拾取模式（取色 / 鼠标坐标）：点哪算哪 —— 单击即确认、选区 = 该点 1x1，
    /// 不画 8 节点/OK-X（没有矩形可编辑）。矩形模式（截图 / 选区 / OCR）为 false。
    pub point_pick: bool,
    /// 取色模式（point_pick 的子集）：跟随鼠标绘制放大镜 + 实时色值条。
    /// 鼠标坐标模式为 false。
    pub pick_color: bool,
    /// 宿主 AppHandle —— **仅 macOS 使用**：AppKit 的所有 UI 操作必须在主线程执行，
    /// macOS 实现通过 `AppHandle::run_on_main_thread` 建窗并用 NSTimer 驱动渲染。
    /// Windows / Linux 忽略该字段（它们的窗口/消息循环自给自足），故在那两个平台上
    /// 该字段永远不会被读 —— 显式 allow，避免 CI 的 `-D warnings` 把它当死代码。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub app: Option<tauri::AppHandle>,
}

/// 遮罩运行结果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayOutcome {
    /// 用户点 OK / 回车 / 空格。
    ///
    /// `sel` 是**原始选区**（屏幕物理像素，可能越界/为负）—— 与
    /// `geometry::Sel` 的语义一致，「夹紧」是输出侧的事，由调用方按冻结帧边界裁剪。
    Confirmed { sel: (i32, i32, i32, i32) },
    /// X / ESC / 右键 / 硬超时 / 初始化失败。
    Cancelled,
}

// ── 跨平台统一入口（各平台实现同一签名；阻塞直到用户确认或取消）──

#[cfg(windows)]
pub use windows::run_overlay;

#[cfg(target_os = "macos")]
pub use macos::run_overlay;

#[cfg(target_os = "linux")]
pub use linux::run_overlay;
