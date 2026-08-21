//! Cookie Vault — 全应用唯一的 cookie 来源与策略中心
//!
//! 数据源：用户 Chrome 默认配置的 Cookies SQLite 数据库（Windows 上经 DPAPI
//! 解密）。三个消费出口共享同一来源：
//!   1. CDP 出口 —— `browser::BrowserClient::import_cookies` 经 vault 取数后
//!      通过 `Network.setCookie` 注入当前页面；
//!   2. yt-dlp 出口 —— `to_netscape` 导出 Netscape cookies.txt（临时文件，
//!      用完即删），video pipeline 以 `--cookies` 注入；
//!   3. reqwest 出口 —— `tools::builtin::web` 按域白名单取 cookie 拼
//!      `Cookie` header。
//!
//! 安全约束：cookie value 永不进入日志（`CookieEntry` 的 `Debug` 已脱敏），
//! 不明文持久化到磁盘配置，内存缓存带 TTL。

mod cdp_source;
mod vault;

pub use vault::{
    decrypt_secret, domain_matches, encrypt_plaintext_provider_keys, encrypt_secret, to_header,
    to_netscape, vault, CookieEntry, CookieVault,
};
