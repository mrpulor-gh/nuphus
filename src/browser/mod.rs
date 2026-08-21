//! Browser automation module (Rust native CDP)
//!
//! 自 nuphus-browser 独立 crate 重导出（原 `src/browser` 模块整体抽离，
//! 供 nuphus-mcp 与主 crate 共用）。对外 API 保持不变：
//! `BrowserClient` / `find_chrome` / `ChromeError` / `get_or_launch` /
//! `runtime` / `shared_client`。

pub use nuphus_browser::{
    find_chrome, get_or_launch, runtime, shared_client, BrowserClient, BrowserError, ChromeError,
    ExternalIdentity,
};
