//! CDP live 数据源：Chrome Cookies DB 不可用（运行中独占锁 / v20 App-Bound
//! 加密）时的回退源。
//!
//! 路径：共享嵌入式浏览器（browser_profile_v2 持久 profile，含用户登录态）
//! 访问目标域 → CDP `get_cookies` 取回当前页适用的解密 cookie → 转换为
//! [`CookieEntry`]。仅由 `cookies_for_host` / `refresh_host` 的 fallback
//! 调用；`get` / `refresh` 原始 API 不经过此源（import_cookies 自指免疫）。
//!
//! 安全约束：cookie value 永不进入日志/测试输出（只记域 + 条数）。

use std::time::Duration;

use super::vault::{CookieEntry, CHROME_EPOCH_OFFSET_SECS};

/// 页面导航后等待页面稳定（JS 种 cookie）的时间。
const PAGE_SETTLE_MS: u64 = 1800;

/// 同步入口：取指定注册域（如 `douyin.com`）当前适用的 cookie。
///
/// 桥接模式同 `browser_tools::set_browser_client`：spawn 独立线程 + 在
/// 进程级常驻 browser runtime 上 block_on（调用方可能处于 tokio 上下文，
/// 且 CDP handler 不能随临时 runtime 销毁）。
pub fn fetch_via_cdp(domain: &str) -> Result<Vec<CookieEntry>, String> {
    let domain = domain.to_string();
    std::thread::spawn(move || crate::browser::runtime().block_on(fetch_async(&domain)))
        .join()
        .map_err(|_| "cdp fetch thread panicked".to_string())?
}

/// 先 headless 试取；失败或取回 0 条则回退有界面模式重试一次
/// （headless 可能被反爬识别，双模式保底）。
async fn fetch_async(domain: &str) -> Result<Vec<CookieEntry>, String> {
    let headless_result = try_fetch(domain, true).await;
    if let Ok(cookies) = &headless_result {
        if !cookies.is_empty() {
            return Ok(cookies.clone());
        }
    }
    let headed_result = try_fetch(domain, false).await;
    match headed_result {
        Ok(cookies) if !cookies.is_empty() => Ok(cookies),
        Ok(_) => Err(format!("cdp: no cookies for {}", domain)),
        Err(e) => match headless_result {
            Err(e1) => Err(format!("headless: {}; headed: {}", e1, e)),
            _ => Err(e),
        },
    }
}

/// 单次取数：get-or-launch 共享 client（持锁独占）→ 导航（www 优先，失败
/// 回退裸域）→ 等页面稳定 → get_cookies → 转换。
async fn try_fetch(domain: &str, headless: bool) -> Result<Vec<CookieEntry>, String> {
    let mut guard = crate::browser::get_or_launch(headless).await?;
    let client = guard.as_mut().ok_or("browser client unavailable")?;

    let www = format!("https://www.{}/", domain);
    if let Err(e1) = client.navigate(&www).await {
        let bare = format!("https://{}/", domain);
        client
            .navigate(&bare)
            .await
            .map_err(|e2| format!("navigate failed: {}; {}", e1, e2))?;
    }

    tokio::time::sleep(Duration::from_millis(PAGE_SETTLE_MS)).await;

    let raw = client.cookies_get().await.map_err(|e| e.to_string())?;
    Ok(raw.iter().filter_map(cdp_cookie_to_entry).collect())
}

/// CDP cookie json → CookieEntry。
///
/// - `expires`：CDP 为 Unix 秒（-1 = 会话）；vault 内统一 Windows-epoch 秒，
///   故 +11644473600 换算；-1/缺失/非正 → `None`（会话 cookie）。
/// - `same_site`：CDP 取值（"Strict"/"Lax"/"None"）映射到 vault 现有取值
///   （"strict"/"lax"/"no_restriction"）；未知/缺失 → `None`。
/// - name/value 为空 → 丢弃该条（与 Chrome DB 源跳过空值行为一致）。
fn cdp_cookie_to_entry(v: &serde_json::Value) -> Option<CookieEntry> {
    let name = v.get("name")?.as_str()?;
    let value = v.get("value")?.as_str()?;
    if name.is_empty() || value.is_empty() {
        return None;
    }

    let expires = match v.get("expires").and_then(|e| e.as_f64()) {
        Some(e) if e > 0.0 => Some(e + CHROME_EPOCH_OFFSET_SECS),
        _ => None,
    };

    let same_site = v.get("same_site").and_then(|s| s.as_str()).and_then(|s| {
        match s.to_ascii_lowercase().as_str() {
            "strict" => Some("strict".to_string()),
            "lax" => Some("lax".to_string()),
            "none" | "no_restriction" => Some("no_restriction".to_string()),
            "unspecified" => Some("unspecified".to_string()),
            _ => None,
        }
    });

    Some(CookieEntry {
        name: name.to_string(),
        value: value.to_string(),
        domain: v
            .get("domain")
            .and_then(|d| d.as_str())
            .unwrap_or_default()
            .to_string(),
        path: v
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("/")
            .to_string(),
        secure: v.get("secure").and_then(|b| b.as_bool()).unwrap_or(false),
        http_only: v
            .get("http_only")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        same_site,
        expires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cdp_json(expires: Option<f64>, same_site: Option<&str>) -> serde_json::Value {
        let mut v = serde_json::json!({
            "name": "sessionid",
            "value": "x",
            "domain": ".douyin.com",
            "path": "/",
            "secure": true,
            "http_only": true,
        });
        if let Some(e) = expires {
            v["expires"] = serde_json::json!(e);
        }
        if let Some(s) = same_site {
            v["same_site"] = serde_json::json!(s);
        }
        v
    }

    #[test]
    fn converts_unix_expires_to_windows_epoch() {
        let e = cdp_cookie_to_entry(&cdp_json(Some(1_700_000_000.0), None)).unwrap();
        assert_eq!(e.expires, Some(1_700_000_000.0 + CHROME_EPOCH_OFFSET_SECS));
    }

    #[test]
    fn session_cookie_expires_minus_one_becomes_none() {
        let e = cdp_cookie_to_entry(&cdp_json(Some(-1.0), None)).unwrap();
        assert_eq!(e.expires, None);
    }

    #[test]
    fn session_cookie_missing_expires_becomes_none() {
        let e = cdp_cookie_to_entry(&cdp_json(None, None)).unwrap();
        assert_eq!(e.expires, None);
    }

    #[test]
    fn same_site_mapping_aligns_with_vault_values() {
        let strict = cdp_cookie_to_entry(&cdp_json(None, Some("Strict"))).unwrap();
        assert_eq!(strict.same_site.as_deref(), Some("strict"));
        let lax = cdp_cookie_to_entry(&cdp_json(None, Some("Lax"))).unwrap();
        assert_eq!(lax.same_site.as_deref(), Some("lax"));
        let none = cdp_cookie_to_entry(&cdp_json(None, Some("None"))).unwrap();
        assert_eq!(none.same_site.as_deref(), Some("no_restriction"));
        let missing = cdp_cookie_to_entry(&cdp_json(None, None)).unwrap();
        assert_eq!(missing.same_site, None);
    }

    #[test]
    fn maps_flags_and_defaults() {
        let e = cdp_cookie_to_entry(&cdp_json(None, None)).unwrap();
        assert_eq!(e.domain, ".douyin.com");
        assert_eq!(e.path, "/");
        assert!(e.secure);
        assert!(e.http_only);
    }

    #[test]
    fn drops_empty_value_entries() {
        let v = serde_json::json!({"name": "k", "value": "", "domain": ".a.com"});
        assert!(cdp_cookie_to_entry(&v).is_none());
        let v = serde_json::json!({"name": "k", "domain": ".a.com"});
        assert!(cdp_cookie_to_entry(&v).is_none());
    }

    /// 集成测试：真实启动嵌入式浏览器访问抖音，验证 CDP fallback 全链路。
    /// 依赖本机 Chrome + browser_profile_v2 中已登录的抖音会话。
    /// 运行：cargo test -p nuphus cdp_fetch_douyin -- --ignored
    #[test]
    #[ignore = "真实启动浏览器，需本机 Chrome 与已登录 profile"]
    fn cdp_fetch_douyin_returns_login_cookies() {
        let cookies = fetch_via_cdp("douyin.com").expect("cdp fetch failed");
        assert!(
            cookies.len() > 10,
            "expected >10 cookies for douyin.com, got {}",
            cookies.len()
        );
        let has_login_cookie = cookies
            .iter()
            .any(|c| c.name == "ttwid" || c.name == "sid_guard");
        assert!(has_login_cookie, "missing ttwid/sid_guard login cookie");
    }
}
