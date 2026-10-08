//! Browser automation module (Rust native CDP)
//!
//! 自 nuphus-browser 独立 crate 重导出（原 `src/browser` 模块整体抽离，
//! 供 nuphus-mcp 与主 crate 共用）。对外 API 保持不变：
//! `BrowserClient` / `find_chrome` / `ChromeError` / `get_or_launch` /
//! `runtime` / `shared_client`。

pub use nuphus_browser::{
    find_chrome, get_or_launch, managed_profile_dir, runtime, shared_client, BrowserClient,
    BrowserError, ChromeError, ExternalIdentity,
};

// 标注 overlay 脚本：preview:// 宿主（本地 HTML 预览页）用它做页面标注。
// 真源在 nuphus-browser，Tauri 壳经本模块取，无需让壳直接依赖 browser crate。
pub use nuphus_browser::ANNOTATION_OVERLAY_SCRIPT;
