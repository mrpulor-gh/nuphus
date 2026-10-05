//! 截图遮罩模块 —— 平台分派的原生遮罩实现。
//!
//! 目标：为桌面工具栏（ctrl+U）的模式提供原生遮罩交互：冻结帧 + hover 跟窗
//! 自动框选 + 拖拽重划 + 确认态平移 + 8 节点调整 + OK/X + 单点拾取 + 无残影。
//! Windows 为主链路，`commands/toolbar.rs` 把 screenshot / picker /
//! color_picker / mouse_pos / ocr 五个模式统一路由进来（单点拾取模式见
//! `session::run_screenshot` 的 `point_pick` / `pick_color`）。
//!
//! 架构：
//! ```text
//! capture/
//! ├── geometry.rs   平台无关：Sel/Point/Rect + normalize/apply_node + 节点与按钮布局
//! ├── state.rs      平台无关：Phase 状态机 + Action 输出（纯计算，不碰 IO）
//! ├── session.rs    平台无关：编排（抓屏排除自身主窗 → 冻结帧 → 跑遮罩 → 恢复）
//! └── surface/      平台相关：遮罩表面（同一 run_overlay 契约签名）
//!     ├── windows.rs  Win32 分层窗口（UpdateLayeredWindow，精确物理像素）
//!     └── fallback.rs 上述三平台之外的兜底
//! wip/              未完成的平台实现（macos.rs / linux.rs，从未编译通过，
//!                   未接入编译；非 Windows 截图走 WebView overlay 兜底，详见 wip/README.md）
//! ```
//!
//! 硬约束：
//!   1. **跨平台**：非 Windows 平台由 `session::run_screenshot` 的
//!      `#[cfg(not(windows))]` 分支直接报错，调用方回退现有 WebView overlay
//!      链路，不破坏编译（`#[cfg]` 分派）。
//!   2. **确定性**：不用轮询、不用硬 sleep；等待用合成器同步（DwmFlush）或事件握手。
//!   3. **主窗恢复兜底**：无论哪一步失败，主窗可见性与抓屏排除都由 guard 还原。

pub mod geometry;
pub mod session;
pub mod state;
pub mod surface;
