//! CookieVault 实现：Chrome Cookies DB 数据源层 + 内存缓存（TTL）。
//!
//! 数据源层（`read_chrome_cookies` / `find_chrome_cookies_path` /
//! `decrypt_chrome_cookie` / `dpapi_decrypt`）上移自 `browser::client`，
//! 逻辑保持逐字节一致，仅可见性提升。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::cdp_source;

/// 内存缓存 TTL：命中期间不重复读取 Chrome DB。
const CACHE_TTL: Duration = Duration::from_secs(300);

/// Chrome `expires_utc` 是 1601-01-01（Windows epoch）起的微秒数；
/// Netscape cookies.txt 要求 Unix 秒，差值固定为 11644473600 秒。
pub(crate) const CHROME_EPOCH_OFFSET_SECS: f64 = 11_644_473_600.0;

/// A cookie entry read from Chrome's SQLite database.
///
/// `expires` 保留数据源原始语义：Windows-epoch 秒（f64），供 CDP 出口
/// 原样消费；Netscape 导出时由 `to_netscape` 换算为 Unix 秒。
#[derive(Clone)]
pub struct CookieEntry {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires: Option<f64>,
}

// 安全约束：cookie value 永不进入日志/调试输出。
impl std::fmt::Debug for CookieEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieEntry")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("expires", &self.expires)
            .finish()
    }
}

/// Chrome cookie 数据源签名（可注入 mock 以便单测）。
type CookieLoader = fn(Option<&str>) -> Result<Vec<CookieEntry>, String>;

struct CacheEntry {
    cookies: Vec<CookieEntry>,
    fetched_at: Instant,
}

/// 统一 cookie 来源：内存缓存（按域过滤键分桶，TTL 过期自动重读）。
///
/// - `get` —— 命中缓存直接返回，未命中/过期则读 Chrome DB；
/// - `refresh` —— 绕过缓存强制重读（用于 cookie 失效后的重试路径）。
pub struct CookieVault {
    cache: Mutex<HashMap<String, CacheEntry>>,
    ttl: Duration,
    loader: CookieLoader,
}

impl CookieVault {
    fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            ttl: CACHE_TTL,
            loader: read_chrome_cookies,
        }
    }

    #[cfg(test)]
    fn with_loader(loader: CookieLoader, ttl: Duration) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            ttl,
            loader,
        }
    }

    /// 读取指定域的 cookie（带缓存）。`domain_filter` 语义与 Chrome DB 的
    /// `host_key.contains(filter)` 一致；`None` 表示不过滤。
    pub fn get(&self, domain_filter: Option<&str>) -> Result<Vec<CookieEntry>, String> {
        let key = domain_filter.unwrap_or("").to_string();
        if let Ok(cache) = self.cache.lock() {
            if let Some(entry) = cache.get(&key) {
                if entry.fetched_at.elapsed() < self.ttl {
                    return Ok(entry.cookies.clone());
                }
            }
        }
        self.load_and_cache(key, domain_filter)
    }

    /// 绕过缓存强制重读，并刷新缓存。
    pub fn refresh(&self, domain_filter: Option<&str>) -> Result<Vec<CookieEntry>, String> {
        let key = domain_filter.unwrap_or("").to_string();
        self.load_and_cache(key, domain_filter)
    }

    fn load_and_cache(
        &self,
        key: String,
        domain_filter: Option<&str>,
    ) -> Result<Vec<CookieEntry>, String> {
        let cookies = (self.loader)(domain_filter)?;
        self.cache_insert(key, cookies.clone());
        Ok(cookies)
    }

    fn cache_insert(&self, key: String, cookies: Vec<CookieEntry>) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(
                key,
                CacheEntry {
                    cookies,
                    fetched_at: Instant::now(),
                },
            );
        }
    }

    /// 取某个请求 host 适用的 cookie（缓存路径）。
    ///
    /// 先以注册域启发式（末两段标签，如 `www.bilibili.com` → `bilibili.com`）
    /// 做 DB 过滤，再按 cookie 域规则精确匹配，避免子域查询漏命中
    /// （`.bilibili.com` 不含子串 `www.bilibili.com`）。
    pub fn cookies_for_host(&self, host: &str) -> Vec<CookieEntry> {
        self.cookies_for_host_inner(host, false)
    }

    /// 同 `cookies_for_host`，但强制刷新数据源（重试路径）。
    pub fn refresh_host(&self, host: &str) -> Vec<CookieEntry> {
        self.cookies_for_host_inner(host, true)
    }

    fn cookies_for_host_inner(&self, host: &str, force_refresh: bool) -> Vec<CookieEntry> {
        let filter = registrable_domain(host);
        let key = filter.to_string();

        // 1. 缓存命中（含 CDP 回填的条目）直接使用。
        if !force_refresh {
            if let Ok(cache) = self.cache.lock() {
                if let Some(entry) = cache.get(&key) {
                    if entry.fetched_at.elapsed() < self.ttl {
                        return entry
                            .cookies
                            .iter()
                            .filter(|c| domain_matches(&c.domain, host))
                            .cloned()
                            .collect();
                    }
                }
            }
        }

        // 2. Chrome DB 源。非空即返回；Err（运行中 Chrome 独占锁 / v20
        //    App-Bound 加密解不开）或过滤后为空 → CDP live 回退。
        if let Ok(all) = (self.loader)(Some(filter)) {
            if !all.is_empty() {
                tracing::info!(
                    "[cookies] source=chrome_db domain={} count={}",
                    filter,
                    all.len()
                );
                self.cache_insert(key.clone(), all.clone());
                return all
                    .into_iter()
                    .filter(|c| domain_matches(&c.domain, host))
                    .collect();
            }
        }

        // 3. CDP live 源：共享嵌入式浏览器访问目标域后 get_cookies。
        //    结果（含空结果）走同一 cache，避免每次请求都重复启动浏览器；
        //    refresh_host 绕过缓存，重试路径不受影响。
        match cdp_source::fetch_via_cdp(filter) {
            Ok(cookies) if !cookies.is_empty() => {
                tracing::info!(
                    "[cookies] source=cdp domain={} count={}",
                    filter,
                    cookies.len()
                );
                self.cache_insert(key, cookies.clone());
                cookies
                    .into_iter()
                    .filter(|c| domain_matches(&c.domain, host))
                    .collect()
            }
            Ok(_) => {
                tracing::warn!("[cookies] source=cdp domain={} count=0", filter);
                self.cache_insert(key, Vec::new());
                Vec::new()
            }
            Err(e) => {
                tracing::warn!("[cookies] cdp fallback failed domain={}: {}", filter, e);
                self.cache_insert(key, Vec::new());
                Vec::new()
            }
        }
    }
}

/// 全局单例。
pub fn vault() -> &'static CookieVault {
    static INSTANCE: OnceLock<CookieVault> = OnceLock::new();
    INSTANCE.get_or_init(CookieVault::new)
}

/// 注册域启发式：取 host 的末两段标签（`www.bilibili.com` → `bilibili.com`）。
/// 对 `co.uk` 类公共后缀不精确，但对内置白名单域（bilibili.com / douyin.com
/// 等）足够；调用方仅以此缩小 DB 读取范围，精确性由 `domain_matches` 保证。
fn registrable_domain(host: &str) -> &str {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() >= 2 {
        let offset = labels[labels.len() - 2].as_ptr() as usize - host.as_ptr() as usize;
        &host[offset..]
    } else {
        host
    }
}

/// cookie 域匹配规则：cookie 域 `.example.com` / `example.com` 均匹配
/// `example.com` 及其任意子域。
pub fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = cookie_domain.trim_start_matches('.');
    host == domain || host.ends_with(&format!(".{}", domain))
}

/// 将 cookie 列表拼为 HTTP `Cookie` header 值（`k=v; k=v`）。
/// 无适用 cookie 时返回 `None`。
pub fn to_header(cookies: &[CookieEntry]) -> Option<String> {
    if cookies.is_empty() {
        return None;
    }
    Some(
        cookies
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// 导出 Netscape cookies.txt 格式（供 yt-dlp `--cookies` 消费）。
///
/// 每行：`domain<TAB>include_subdomains<TAB>path<TAB>secure<TAB>expiry<TAB>name<TAB>value`
/// - expiry 为 Unix 秒；会话 cookie（无 expires）写 0；
/// - name/value 中的制表符与换行会被剥离（保证单行一条记录）。
pub fn to_netscape(cookies: &[CookieEntry]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let include_subdomains = if c.domain.starts_with('.') {
            "TRUE"
        } else {
            "FALSE"
        };
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        let expiry = match c.expires {
            Some(e) if e > 0.0 => {
                let unix = e - CHROME_EPOCH_OFFSET_SECS;
                if unix > 0.0 {
                    unix as i64
                } else {
                    0
                }
            }
            _ => 0,
        };
        let name = sanitize_field(&c.name);
        let value = sanitize_field(&c.value);
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.domain, include_subdomains, c.path, secure, expiry, name, value
        ));
    }
    out
}

/// Netscape 格式为制表符分隔单行记录，剥离会破坏结构的控制字符。
fn sanitize_field(s: &str) -> String {
    s.chars()
        .filter(|ch| *ch != '\t' && *ch != '\n' && *ch != '\r')
        .collect()
}

// ═══════════════════════════════════════════════════
// Chrome cookie 数据源层（上移自 browser::client，逻辑保持一致）
// ═══════════════════════════════════════════════════

/// Read cookies from the user's Chrome default profile.
///
/// On Windows, attempts to decrypt encrypted cookie values via DPAPI.
/// Falls back to reading only unencrypted cookies if decryption fails.
fn read_chrome_cookies(domain_filter: Option<&str>) -> Result<Vec<CookieEntry>, String> {
    let cookies_path = find_chrome_cookies_path().ok_or("Chrome Cookies database not found")?;

    let conn = rusqlite::Connection::open(&cookies_path)
        .map_err(|e| format!("Failed to open Chrome Cookies DB: {}", e))?;

    let mut stmt = conn
        .prepare(
            "SELECT host_key, name, encrypted_value, path, expires_utc, is_secure, is_httponly, samesite FROM cookies",
        )
        .map_err(|e| format!("Failed to prepare SQL: {}", e))?;

    let rows = stmt
        .query_map([], |row| {
            let host_key: String = row.get(0)?;
            let name: String = row.get(1)?;
            let encrypted_value: Vec<u8> = row.get(2)?;
            let path: String = row.get(3)?;
            let expires_utc: i64 = row.get(4)?;
            let is_secure: bool = row.get(5)?;
            let is_httponly: bool = row.get(6)?;
            let samesite: i64 = row.get(7)?;

            Ok((
                host_key,
                name,
                encrypted_value,
                path,
                expires_utc,
                is_secure,
                is_httponly,
                samesite,
            ))
        })
        .map_err(|e| format!("Failed to query cookies: {}", e))?;

    let mut cookies = Vec::new();
    for row in rows {
        let (host_key, name, encrypted_value, path, expires_utc, is_secure, is_httponly, samesite) =
            row.map_err(|e| format!("Row error: {}", e))?;

        // Apply domain filter
        if let Some(filter) = domain_filter {
            if !host_key.contains(filter) {
                continue;
            }
        }

        // Try to decrypt the value
        let value = match decrypt_chrome_cookie(&encrypted_value) {
            Some(v) => v,
            None => {
                // If encrypted, skip (can't read); if plaintext, use as-is
                if encrypted_value.len() < 3 || encrypted_value[0] == 0x01 {
                    continue; // v10 encrypted, skip
                }
                // Try as UTF-8 plaintext
                String::from_utf8(encrypted_value).unwrap_or_default()
            }
        };

        if value.is_empty() {
            continue;
        }

        let samesite_str = match samesite {
            -1 => Some("unspecified".to_string()),
            0 => Some("no_restriction".to_string()), // None → Lax in CDP terms
            1 => Some("lax".to_string()),
            2 => Some("strict".to_string()),
            _ => None,
        };

        cookies.push(CookieEntry {
            name,
            value,
            domain: host_key,
            path,
            secure: is_secure,
            http_only: is_httponly,
            same_site: samesite_str,
            expires: if expires_utc > 0 {
                Some(expires_utc as f64 / 1_000_000.0) // microseconds → seconds
            } else {
                None
            },
        });
    }

    Ok(cookies)
}

/// Find the default Chrome profile's Cookies file.
fn find_chrome_cookies_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let local_appdata = std::env::var("LOCALAPPDATA").ok()?;
        let path = std::path::PathBuf::from(local_appdata)
            .join("Google")
            .join("Chrome")
            .join("User Data")
            .join("Default")
            .join("Network")
            .join("Cookies");
        if path.exists() {
            return Some(path);
        }
        // Fallback: Cookies directly in Default
        let fallback = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").ok()?)
            .join("Google")
            .join("Chrome")
            .join("User Data")
            .join("Default")
            .join("Cookies");
        if fallback.exists() {
            return Some(fallback);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        let path = std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Google")
            .join("Chrome")
            .join("Default")
            .join("Cookies");
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").ok()?;
        let path = std::path::PathBuf::from(home)
            .join(".config")
            .join("google-chrome")
            .join("Default")
            .join("Cookies");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// Decrypt a Chrome-encrypted cookie value using DPAPI (Windows only).
///
/// Chrome v10+ uses AES-256-GCM with a key stored in Local State,
/// encrypted with DPAPI. This implementation handles DPAPI (old format).
/// v10/v20 (AES-GCM) cookies are skipped — they require the `aes-gcm` crate.
#[cfg(target_os = "windows")]
fn decrypt_chrome_cookie(encrypted: &[u8]) -> Option<String> {
    if encrypted.len() < 3 {
        return None;
    }

    // v10/v20 prefix — AES-256-GCM encrypted, not yet supported
    if encrypted[0] == b'v' && (encrypted[1] == b'1' || encrypted[1] == b'2') {
        return None;
    }

    // Old format: DPAPI-encrypted
    dpapi_decrypt(encrypted)
}

/// Decrypt data using Windows DPAPI.
#[cfg(target_os = "windows")]
fn dpapi_decrypt(data: &[u8]) -> Option<String> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };

    let mut blob_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        let result = CryptUnprotectData(
            &blob_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut blob_out,
        );

        if result.is_ok() && blob_out.cbData > 0 && !blob_out.pbData.is_null() {
            let decrypted = std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize);
            let s = String::from_utf8(decrypted.to_vec()).ok();
            // Free the buffer — LocalFree takes HLOCAL
            let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(
                blob_out.pbData as *mut std::ffi::c_void,
            ));
            return s;
        }
    }

    None
}

// Non-Windows: DPAPI not available
#[cfg(not(target_os = "windows"))]
fn decrypt_chrome_cookie(encrypted: &[u8]) -> Option<String> {
    // On non-Windows, try plaintext
    if encrypted.is_empty() {
        return None;
    }
    if encrypted[0] == b'v' {
        return None; // v10/v20 encrypted, can't decrypt without OS keychain
    }
    String::from_utf8(encrypted.to_vec()).ok()
}

// ============================================================================
// 敏感配置加密（API key 落盘保护）
// ============================================================================
// 格式：`enc:v1:<base64>`。Windows 用 DPAPI（CryptProtectData）加密，绑定
// 当前 Windows 用户；非 Windows 无系统级安全 enclave → 保持明文（诚实降级，
// 不虚构防护）。读取端 `decrypt_secret` 对无前缀值按明文兼容（旧配置迁移）。

/// Windows DPAPI 加密（与 dpapi_decrypt 对称）。
#[cfg(target_os = "windows")]
pub fn dpapi_encrypt(data: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut blob_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    unsafe {
        let result = CryptProtectData(
            &blob_in,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut blob_out,
        );

        if result.is_ok() && !blob_out.pbData.is_null() {
            let out =
                std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec();
            // Free the buffer — LocalFree takes HLOCAL
            let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(
                blob_out.pbData as *mut std::ffi::c_void,
            ));
            return Some(out);
        }
    }

    None
}

/// 加密敏感字符串 → `enc:v1:<base64>`。非 Windows 返回原文（诚实降级）。
pub fn encrypt_secret(plain: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        if let Some(ct) = dpapi_encrypt(plain.as_bytes()) {
            use base64::Engine as _;
            return format!(
                "enc:v1:{}",
                base64::engine::general_purpose::STANDARD.encode(ct)
            );
        }
        // 加密失败（极少见）→ 返回明文并记录，避免用户 key 丢失
        tracing::error!("[vault] DPAPI 加密失败，API key 将以明文存储 —— 请检查系统安全策略");
    }
    plain.to_string()
}

/// base64 密文 → DPAPI 解密（Windows）。非 Windows 无对应实现 → None。
#[cfg(target_os = "windows")]
fn decrypt_dpapi_b64(b64: &str) -> Option<String> {
    use base64::Engine as _;
    let ct = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    dpapi_decrypt(&ct)
}

/// 任意字节密文 → DPAPI 解密（Windows）。非 Windows 无对应实现 → None。
#[cfg(target_os = "windows")]
fn decrypt_dpapi_bytes(ct: Vec<u8>) -> Option<String> {
    dpapi_decrypt(&ct)
}

/// 任意字节密文 → 解密（非 Windows：无 DPAPI 实现，一律视为无效）。
#[cfg(not(target_os = "windows"))]
fn decrypt_dpapi_bytes(_ct: Vec<u8>) -> Option<String> {
    None
}

/// base64 密文 → 解密（非 Windows：无 DPAPI 实现，一律视为无效）。
#[cfg(not(target_os = "windows"))]
fn decrypt_dpapi_b64(_b64: &str) -> Option<String> {
    None
}

/// 解码旧版 `enc:` 密文（HEX 优先，兼容 base64 变体）。
fn decode_legacy_enc(h: &str) -> Option<Vec<u8>> {
    if let Ok(hex) = hex::decode(h) {
        return Some(hex);
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(h).ok()
}

/// 解密 `enc:v1:<base64>` → 原文；无前缀按明文兼容返回；解密失败返回 None（不 panic）。
///
/// 兼容旧格式：早期版本曾以 `enc:<HEX>`（DPAPI 密文，无 `v1:` 版本号）写入，
/// 这里一并迁移解密；解密成功的值在下次保存时会被 `encrypt_plaintext_provider_keys`
/// 重写为 `enc:v1:` 新格式。
pub fn decrypt_secret(stored: &str) -> Option<String> {
    if let Some(b64) = stored.strip_prefix("enc:v1:") {
        return decrypt_dpapi_b64(b64);
    }
    if let Some(h) = stored.strip_prefix("enc:") {
        // 旧版 `enc:<HEX/base64>`（无 v1: 版本号）——迁移解密
        return decrypt_dpapi_bytes(decode_legacy_enc(h)?);
    }
    Some(stored.to_string())
}

/// 将 providers.toml 文档中所有明文 provider `api_key` 原地加密为 `enc:v1:`。
///
/// 幂等：已带 `enc:` 前缀（新旧格式均可）的 key 保持不变；空 key / 缺失字段跳过。
/// 供所有「整文档重序列化」的写路径调用，避免未加密写路径把存量明文 key 原样保留。
pub fn encrypt_plaintext_provider_keys(doc: &mut toml::Value) {
    let Some(providers) = doc.get_mut("providers").and_then(|p| p.as_array_mut()) else {
        return;
    };
    for provider in providers.iter_mut() {
        let Some(map) = provider.as_table_mut() else {
            continue;
        };
        let Some(key) = map.get("api_key").and_then(|k| k.as_str()) else {
            continue;
        };
        if key.is_empty() || key.starts_with("enc:") {
            continue;
        }
        map.insert(
            "api_key".to_string(),
            toml::Value::String(encrypt_secret(key)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn entry(domain: &str, name: &str, value: &str) -> CookieEntry {
        CookieEntry {
            name: name.to_string(),
            value: value.to_string(),
            domain: domain.to_string(),
            path: "/".to_string(),
            secure: false,
            http_only: false,
            same_site: None,
            expires: None,
        }
    }

    // ── Netscape 格式 ──

    #[test]
    fn netscape_format_basic() {
        let cookies = vec![entry(".bilibili.com", "SESSDATA", "abc123")];
        let out = to_netscape(&cookies);
        assert!(out.starts_with("# Netscape HTTP Cookie File\n"));
        let line = out.lines().nth(1).unwrap();
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 7);
        assert_eq!(fields[0], ".bilibili.com");
        assert_eq!(fields[1], "TRUE"); // 前导点 → 含子域
        assert_eq!(fields[2], "/");
        assert_eq!(fields[3], "FALSE"); // secure
        assert_eq!(fields[4], "0"); // 会话 cookie → expiry 0
        assert_eq!(fields[5], "SESSDATA");
        assert_eq!(fields[6], "abc123");
    }

    #[test]
    fn netscape_expiry_converts_chrome_epoch_to_unix() {
        let mut c = entry("example.com", "k", "v");
        c.secure = true;
        // 1700000000 Unix 秒 + Chrome epoch 偏移
        c.expires = Some(1_700_000_000.0 + CHROME_EPOCH_OFFSET_SECS);
        let out = to_netscape(&cookies(&c));
        let line = out.lines().nth(1).unwrap();
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields[0], "example.com");
        assert_eq!(fields[1], "FALSE"); // 无前导点
        assert_eq!(fields[3], "TRUE");
        assert_eq!(fields[4], "1700000000");
    }

    #[test]
    fn netscape_sanitizes_control_chars() {
        let c = entry("example.com", "na\tme", "va\nl\true");
        let out = to_netscape(&[c]);
        assert_eq!(out.lines().count(), 2); // 头 + 一条记录
        let line = out.lines().nth(1).unwrap();
        assert_eq!(line.split('\t').count(), 7);
        assert!(line.contains("name")); // "na\tme" → "name"
        assert!(line.contains("valrue")); // "va\nl\true" → "valrue"
    }

    fn cookies(c: &CookieEntry) -> Vec<CookieEntry> {
        vec![c.clone()]
    }

    // ── 域过滤 ──

    #[test]
    fn domain_match_rules() {
        assert!(domain_matches(".bilibili.com", "bilibili.com"));
        assert!(domain_matches(".bilibili.com", "www.bilibili.com"));
        assert!(domain_matches("bilibili.com", "api.bilibili.com"));
        assert!(!domain_matches("bilibili.com", "evilbilibili.com"));
        assert!(!domain_matches("bilibili.com", "bilibili.com.evil.com"));
        assert!(!domain_matches("example.com", "sub.example.com.evil.com"));
    }

    #[test]
    fn header_join_and_empty() {
        assert_eq!(to_header(&[]), None);
        let list = vec![entry("a.com", "k1", "v1"), entry("a.com", "k2", "v2")];
        assert_eq!(to_header(&list).unwrap(), "k1=v1; k2=v2");
    }

    // ── 缓存 TTL ──
    //
    // 每个测试使用独立的 static 计数器 + loader（测试并行执行，共享计数器
    // 会相互串扰）。

    macro_rules! counting_loader {
        ($name:ident, $counter:ident) => {
            static $counter: AtomicUsize = AtomicUsize::new(0);
            fn $name(_: Option<&str>) -> Result<Vec<CookieEntry>, String> {
                $counter.fetch_add(1, Ordering::SeqCst);
                Ok(vec![entry(".a.com", "k", "v")])
            }
        };
    }

    counting_loader!(loader_hit, HIT_COUNT);
    counting_loader!(loader_expire, EXPIRE_COUNT);
    counting_loader!(loader_refresh, REFRESH_COUNT);
    counting_loader!(loader_keys, KEYS_COUNT);

    #[test]
    fn cache_hit_within_ttl_skips_reload() {
        HIT_COUNT.store(0, Ordering::SeqCst);
        let v = CookieVault::with_loader(loader_hit, Duration::from_secs(60));
        v.get(Some("a.com")).unwrap();
        v.get(Some("a.com")).unwrap();
        assert_eq!(HIT_COUNT.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cache_expires_after_ttl() {
        EXPIRE_COUNT.store(0, Ordering::SeqCst);
        let v = CookieVault::with_loader(loader_expire, Duration::from_millis(20));
        v.get(Some("a.com")).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        v.get(Some("a.com")).unwrap();
        assert_eq!(EXPIRE_COUNT.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn refresh_bypasses_cache() {
        REFRESH_COUNT.store(0, Ordering::SeqCst);
        let v = CookieVault::with_loader(loader_refresh, Duration::from_secs(60));
        v.get(Some("a.com")).unwrap();
        v.refresh(Some("a.com")).unwrap();
        assert_eq!(REFRESH_COUNT.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cache_keys_are_per_domain() {
        KEYS_COUNT.store(0, Ordering::SeqCst);
        let v = CookieVault::with_loader(loader_keys, Duration::from_secs(60));
        v.get(Some("a.com")).unwrap();
        v.get(Some("b.com")).unwrap();
        v.get(None).unwrap();
        assert_eq!(KEYS_COUNT.load(Ordering::SeqCst), 3);
    }

    // ── cookies_for_host：注册域过滤 + 精确匹配 ──

    fn host_loader(filter: Option<&str>) -> Result<Vec<CookieEntry>, String> {
        // 模拟 DB contains 过滤：仅返回 host_key 包含 filter 的记录
        let all = vec![
            entry(".bilibili.com", "SESSDATA", "x"),
            entry("api.bilibili.com", "bili_jct", "y"),
            entry(".douyin.com", "sessionid", "z"),
        ];
        Ok(match filter {
            Some(f) => all.into_iter().filter(|c| c.domain.contains(f)).collect(),
            None => all,
        })
    }

    #[test]
    fn cookies_for_host_matches_subdomain_host() {
        let v = CookieVault::with_loader(host_loader, Duration::from_secs(60));
        // host = www.bilibili.com：注册域过滤（bilibili.com）缩小 DB 范围，
        // 精确匹配后只有 .bilibili.com 适用（api.bilibili.com 作用域的
        // cookie 对 www.bilibili.com 无效；douyin 被注册域过滤排除）。
        let got = v.cookies_for_host("www.bilibili.com");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "SESSDATA");

        // host = api.bilibili.com：两条 bilibili cookie 均适用。
        let got = v.cookies_for_host("api.bilibili.com");
        assert_eq!(got.len(), 2);
        assert!(got
            .iter()
            .all(|c| domain_matches(&c.domain, "api.bilibili.com")));
    }

    #[test]
    fn registrable_domain_heuristic() {
        assert_eq!(registrable_domain("www.bilibili.com"), "bilibili.com");
        assert_eq!(registrable_domain("bilibili.com"), "bilibili.com");
        assert_eq!(registrable_domain("localhost"), "localhost");
    }

    // ── Debug 脱敏 ──

    #[test]
    fn debug_redacts_value() {
        let c = entry("a.com", "SESSDATA", "super-secret");
        let dbg = format!("{:?}", c);
        assert!(!dbg.contains("super-secret"));
        assert!(dbg.contains("[REDACTED]"));
    }

    // ── 敏感配置加密（encrypt_secret / decrypt_secret） ──

    #[test]
    fn secret_encrypt_roundtrip_windows() {
        let plain = "sk-test-1234567890abcdef";
        let stored = encrypt_secret(plain);
        // Windows 上应加密（带 enc:v1: 前缀）；非 Windows 诚实降级为明文
        #[cfg(target_os = "windows")]
        {
            assert!(stored.starts_with("enc:v1:"), "应加密落盘: {stored}");
            assert!(!stored.contains(plain), "密文不得包含明文: {stored}");
            let back = decrypt_secret(&stored).expect("解密应成功");
            assert_eq!(back, plain);
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(stored, plain, "非 Windows 应保持明文");
            assert_eq!(decrypt_secret(&stored).as_deref(), Some(plain));
        }
    }

    #[test]
    fn secret_plaintext_backward_compat() {
        // 旧明文配置：无前缀按明文兼容
        assert_eq!(
            decrypt_secret("sk-legacy-plain").as_deref(),
            Some("sk-legacy-plain")
        );
        // 空值/异常密文：不 panic，返回 None（视为缺失）
        assert!(decrypt_secret("enc:v1:!!!not-base64!!!").is_none());
        assert!(decrypt_secret("enc:v1:").is_none());
        // 旧版 `enc:`（无版本号）空密文/非法 base64 → None
        assert!(decrypt_secret("enc:").is_none());
        assert!(decrypt_secret("enc:!!!not-base64!!!").is_none());
    }

    #[test]
    fn secret_legacy_enc_prefix_migrates() {
        // 旧版格式 `enc:<HEX>`（DPAPI 密文 HEX 编码，无 v1: 版本号）应能迁移解密。
        // Windows 上由 encrypt_secret 产出 `enc:v1:<base64>`，把 blob 改 HEX 即模拟旧格式。
        #[cfg(target_os = "windows")]
        let plain = "sk-legacy-format-key";
        #[cfg(target_os = "windows")]
        {
            use base64::Engine as _;
            let stored = encrypt_secret(plain);
            let b64 = stored.strip_prefix("enc:v1:").unwrap();
            let blob = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap();
            let legacy = format!("enc:{}", hex::encode(blob));
            assert!(legacy.starts_with("enc:"), "旧格式: {legacy}");
            let back = decrypt_secret(&legacy).expect("旧版 enc:HEX 前缀应能解密");
            assert_eq!(back, plain);
        }
        #[cfg(not(target_os = "windows"))]
        {
            // 非 Windows 无 DPAPI，enc: 密文无法解密 → None
            assert!(decrypt_secret("enc:01000000d08c9ddf").is_none());
        }
    }

    #[test]
    fn normalize_encrypts_plaintext_keys_only() {
        // 明文 key → 加密；已加密 key（enc:v1: / enc: 旧格式）→ 原样不动（幂等）。
        let plain_key = "sk-plaintext-1";
        #[cfg(target_os = "windows")]
        {
            let mut doc: toml::Value = r#"
model = "m"
[[providers]]
name = "a"
api_key = "sk-plaintext-1"
[[providers]]
name = "b"
api_key = "enc:v1:already-encrypted"
[[providers]]
name = "c"
api_key = ""
"#
            .parse()
            .unwrap();
            encrypt_plaintext_provider_keys(&mut doc);

            let providers = doc.get("providers").unwrap().as_array().unwrap();
            let key_a = providers[0].get("api_key").unwrap().as_str().unwrap();
            assert!(key_a.starts_with("enc:v1:"), "明文 key 应被加密: {key_a}");
            assert_eq!(
                decrypt_secret(key_a).as_deref(),
                Some(plain_key),
                "加密后应能解密回原文"
            );
            assert_eq!(
                providers[1].get("api_key").unwrap().as_str().unwrap(),
                "enc:v1:already-encrypted",
                "已加密 key 不重复加密"
            );
            assert_eq!(
                providers[2].get("api_key").unwrap().as_str().unwrap(),
                "",
                "空 key 保持原样"
            );
        }
        #[cfg(not(target_os = "windows"))]
        {
            // 非 Windows 加密诚实降级为明文原样 → 值不变（幂等）
            let mut doc: toml::Value = r#"
[[providers]]
name = "a"
api_key = "sk-plaintext-1"
"#
            .parse()
            .unwrap();
            encrypt_plaintext_provider_keys(&mut doc);
            assert_eq!(
                doc.get("providers").unwrap()[0]
                    .get("api_key")
                    .unwrap()
                    .as_str()
                    .unwrap(),
                plain_key
            );
        }
    }

    #[test]
    fn normalize_skips_non_provider_docs() {
        // 无 providers 数组 / 无 api_key 字段 → 不动、不 panic
        let mut empty: toml::Value = "model = \"m\"\n".parse().unwrap();
        encrypt_plaintext_provider_keys(&mut empty);
        assert_eq!(empty.get("model").unwrap().as_str().unwrap(), "m");

        let mut no_keys: toml::Value = "[[providers]]\nname = \"a\"\nbase_url = \"https://x\"\n"
            .parse()
            .unwrap();
        encrypt_plaintext_provider_keys(&mut no_keys);
        assert!(no_keys.get("providers").unwrap()[0]
            .get("api_key")
            .is_none());
    }
}
