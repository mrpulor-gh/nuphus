pub mod annotations;
pub mod approval;
pub mod canvas_export;
pub mod cdp_window;
// 原生遮罩截图链路（capture/）：**仅 Windows 编译**。主链路是 Windows 原生遮罩；
// 非 Windows 平台截图走既有的 WebView overlay 兜底（toolbar.rs 的旧分支），
// 从不引用本模块。若不门控，非 Windows 上本模块的全部消费者随 windows.rs 一起
//消失，81 处 dead_code 会被 CI 的 clippy -D warnings 直接判死（v0.2.26 实测）。
// macOS / Linux 原生实现完成并接回时（capture/wip/），同步去掉本门控。
#[cfg(windows)]
pub mod capture;
pub mod changelog;
pub mod chat_agent;
pub mod config;
pub mod custom_agent;
pub mod desktop;
pub mod dict_ocr;
pub mod export_log;
pub mod external;
pub mod hud;
pub mod knowledge;
pub mod mcp;
pub mod memory;
pub mod preload;
pub mod preview;
pub mod process;
pub mod security;
pub mod skill;
pub mod tenet;
pub mod toolbar;
pub mod tools;
pub mod user_assets;
pub mod user_input;
pub mod workflow;
pub mod workflow_debug;
pub mod workflow_edit;

pub use self::annotations::*;
pub use self::approval::*;
pub use self::canvas_export::*;
pub use self::cdp_window::*;
pub use self::changelog::*;
pub use self::chat_agent::*;
pub use self::config::*;
pub use self::custom_agent::*;
pub use self::export_log::*;
pub use self::external::*;
pub use self::knowledge::*;
pub use self::mcp::*;
pub use self::memory::*;
pub use self::preload::*;
pub use self::preview::*;
pub use self::process::*;
pub use self::security::*;
pub use self::skill::*;
pub use self::tenet::*;
pub use self::toolbar::*;
pub use self::tools::*;
pub use self::user_assets::*;
pub use self::user_input::*;
pub use self::workflow::*;
pub use self::workflow_debug::*;
