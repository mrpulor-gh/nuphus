//! 通用 OAuth2 接入（Authorization Code + PKCE + 本地回调）— 核心层。
//!
//! 职责边界：本模块承载「段内 OAuth 状态的磁盘读写 + 令牌端点协议交互 +
//! 过期判定与自动刷新」。授权回调服务器（axum 本地监听）与 Tauri 事件属于
//! 桌面层（src-tauri `commands/config/oauth.rs`），不在这里。
//!
//! 安全不变量：
//! - 令牌落盘一律经 DPAPI 加密（`enc:` 前缀，与 api_key 同一套
//!   [`crate::cookies::encrypt_secret`] 口径），明文只存在于进程内存。
//! - 令牌不进日志、不进错误信息：本模块所有 `Err(String)` 只含 HTTP 状态、
//!   OAuth 协议 `error`/`error_description`（RFC 6749 §5.2 定义的服务端枚举）
//!   与可读引导文案。
//!
//! 演进关系（见 `model::ProviderOAuth` 文档）：P3 若引入 credentials 服务，
//! 本模块的 `ensure_fresh_oauth_token` 是唯一需要更换内部实现的出口。

use super::model::{ModelRegistry, ProviderOAuth};

/// 过期裕量：`expires_at` 距 now 不足 60s 即视为需要刷新，
/// 避免令牌在长请求途中过期（请求本身可能耗时数十秒）。
pub const EXPIRY_SKEW_SECS: i64 = 60;

/// token 端点响应解析产物（尚未落盘）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenIssuance {
    pub access_token: String,
    /// 响应缺 refresh_token（部分服务只在首次下发）→ None，刷��时保留旧值。
    pub refresh_token: Option<String>,
    pub expires_in: Option<i64>,
}

/// 纯函数：access token 当前是否可直接使用（未过期且非空）。
///
/// `now` 为 unix 秒。`expires_at = None` 视为未知时效 → 不可信（需刷新），
/// 避免「不知道什么时候过期」被当成「永不过期」。
pub fn token_is_fresh(expires_at: Option<i64>, access_token: &str, now: i64) -> bool {
    !access_token.is_empty()
        && matches!(expires_at, Some(exp) if now < exp.saturating_sub(EXPIRY_SKEW_SECS))
}

/// 纯函数：内存中的 OAuth 段是否持有可直接使用的令牌
/// （transport 构造链的快速路径——命中则零磁盘 IO、零网络）。
pub fn in_memory_fresh_token(oauth: &ProviderOAuth, now: i64) -> Option<String> {
    token_is_fresh(oauth.expires_at, &oauth.access_token, now).then(|| oauth.access_token.clone())
}

fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 当前 unix 秒（状态判定与 expires_at 计算共用的时间源）。
pub fn now_unix() -> i64 {
    unix_now_secs()
}

/// 纯函数：解析 token 端点响应体（JSON）。
///
/// 三类输入：
/// - 标准成功响应（access_token 必备；expires_in/refresh_token 可缺省）
/// - RFC 6749 §5.2 错误响应（`error` 字段）→ Err（error + error_description）
/// - 非 JSON / 缺 access_token → Err 可读错误
pub fn parse_token_response(body: &str) -> Result<TokenIssuance, String> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "令牌端点返回了无法解析的响应（非 JSON）".to_string())?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        let desc = v
            .get("error_description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        return Err(if desc.is_empty() {
            format!("授权服务器拒绝了请求: {err}")
        } else {
            format!("授权服务器拒绝了请求: {err} — {desc}")
        });
    }
    let access_token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    if access_token.is_empty() {
        return Err("令牌端点响应缺少 access_token 字段".to_string());
    }
    Ok(TokenIssuance {
        access_token,
        refresh_token: v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .map(str::to_string),
        expires_in: v.get("expires_in").and_then(|e| e.as_i64()),
    })
}

/// 读指定段的 OAuth 配置（令牌字段已透明解密）。
///
/// 段不存在 / 未配 oauth → None；配置文件缺失/损坏 → None（调用方按
/// ���尚未配置」语义处理，与 api_key 读取路径口径一致）。
pub fn read_oauth_segment(config_path: &std::path::Path, provider: &str) -> Option<ProviderOAuth> {
    let registry = ModelRegistry::from_toml(config_path.to_str()?).ok()?;
    registry
        .providers
        .iter()
        .find(|p| p.name == provider)
        .and_then(|p| p.oauth.clone())
}

/// 把令牌写回段的 `[providers.<段>.oauth]` 表并落盘。
///
/// 写入口径与 `record_last_model` 同款：读整文档 → 就地改段 →
/// `encrypt_plaintext_provider_keys` → 一次写回。令牌写入前经 DPAPI 加密
/// （空串 = 清空，明文绝不落盘）。
pub fn write_oauth_tokens(
    config_path: &std::path::Path,
    provider: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: Option<i64>,
) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = content
        .parse()
        .map_err(|e| format!("providers.toml 解析失败: {e}"))?;
    let segment = doc
        .get_mut("providers")
        .and_then(|p| p.as_array_mut())
        .and_then(|providers| {
            providers
                .iter_mut()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(provider))
        })
        .ok_or_else(|| format!("providers.toml 中不存在段: {provider}"))?;
    let map = segment
        .as_table_mut()
        .ok_or_else(|| format!("配置段格式非法: {provider}"))?;

    let oauth = map
        .entry("oauth")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| format!("段 {provider} 的 oauth 配置不是表"))?;
    oauth.insert(
        "access_token".to_string(),
        toml::Value::String(encode_credential(access_token)),
    );
    oauth.insert(
        "refresh_token".to_string(),
        toml::Value::String(encode_credential(refresh_token)),
    );
    match expires_at {
        Some(ts) => oauth.insert("expires_at".to_string(), toml::Value::Integer(ts)),
        None => oauth.remove("expires_at"),
    };

    crate::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content = toml::to_string_pretty(&doc)
        .map_err(|e| format!("serialize providers.toml failed: {e}"))?;
    std::fs::write(config_path, new_content)
        .map_err(|e| format!("write providers.toml failed: {e}"))?;
    Ok(())
}

/// 登出：清空段的三个令牌字段并落盘（配置五项保留，便于重新授权）。
pub fn clear_oauth_tokens(config_path: &std::path::Path, provider: &str) -> Result<(), String> {
    write_oauth_tokens(config_path, provider, "", "", None)
}

/// 令牌落盘编码：空值写空串（无令牌状态），非空走 DPAPI。
/// 与桌面层 `encode_api_key` 同一语义；核心层写路径（本模块）就近复用
/// [`crate::cookies::encrypt_secret`]，不跨层调用桌面私有函数。
fn encode_credential(plain: &str) -> String {
    let v = plain.trim();
    if v.is_empty() {
        String::new()
    } else {
        crate::cookies::encrypt_secret(v)
    }
}

fn token_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client 初始化失败: {e}"))
}

/// 授权码换令牌（oauth_begin 回调后的核心一步）。
///
/// `verifier`：PKCE code_verifier（use_pkce=false 时不携带）。
/// `redirect_port`：本地回调端口（redirect_uri 与授权请求一致）。
pub fn exchange_code_blocking(
    oauth: &ProviderOAuth,
    code: &str,
    verifier: &str,
    redirect_port: u16,
) -> Result<TokenIssuance, String> {
    let client = token_http_client()?;
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        (
            "redirect_uri",
            format!("http://127.0.0.1:{redirect_port}/callback"),
        ),
        ("client_id", oauth.client_id.clone()),
    ];
    if oauth.use_pkce {
        form.push(("code_verifier", verifier.to_string()));
    }
    let resp = client
        .post(oauth.token_url.trim())
        .form(&form)
        .send()
        .map_err(|e| format!("令牌端点请求失败: {e}"))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        // 失败响应也走同一解析器：提取 OAuth error 字段（协议枚举，不含任何令牌值）
        return match parse_token_response(&body) {
            Err(e) => Err(format!("令牌端点返回错误 (HTTP {status}): {e}")),
            Ok(_) => Err(format!("令牌端点返回错误 (HTTP {status})")),
        };
    }
    parse_token_response(&body)
}

/// 用 refresh token 换新令牌。
pub fn refresh_tokens_blocking(oauth: &ProviderOAuth) -> Result<TokenIssuance, String> {
    if oauth.refresh_token.is_empty() {
        return Err("尚未授权登录".to_string());
    }
    let client = token_http_client()?;
    let form = [
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", oauth.refresh_token.clone()),
        ("client_id", oauth.client_id.clone()),
    ];
    let resp = client
        .post(oauth.token_url.trim())
        .form(&form)
        .send()
        .map_err(|e| format!("令牌端点请求失败: {e}"))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if status.as_u16() == 401 {
        return Err("授权已失效（401），请重新授权登录".to_string());
    }
    if !status.is_success() {
        return match parse_token_response(&body) {
            Err(e) => Err(format!("令牌刷新失败 (HTTP {status}): {e}，请重新授权登录")),
            Ok(_) => Err(format!("令牌刷新失败 (HTTP {status})，请重新授权登录")),
        };
    }
    parse_token_response(&body)
}

/// 确保持有可用的 access token，按需自动刷新（唯一令牌读取出口）。
///
/// - 未过期（距 now > 60s）且非空 → 直接返回，零网络。
/// - 过期/临期且持有 refresh_token → 刷新 → 三字段落盘 → 返回新 token。
/// - 无 refresh_token → Err「尚未授权登录」；刷新失败 → Err 引导重新授权。
pub fn ensure_fresh_oauth_token(
    config_path: &std::path::Path,
    provider: &str,
) -> Result<String, String> {
    ensure_fresh_oauth_token_at(config_path, provider, unix_now_secs())
}

/// [`ensure_fresh_oauth_token`] 的可测形态：now 由调用方注入。
pub fn ensure_fresh_oauth_token_at(
    config_path: &std::path::Path,
    provider: &str,
    now: i64,
) -> Result<String, String> {
    let oauth = read_oauth_segment(config_path, provider)
        .ok_or_else(|| "尚未配置 OAuth 授权".to_string())?;
    if let Some(token) = in_memory_fresh_token(&oauth, now) {
        return Ok(token);
    }
    if oauth.refresh_token.is_empty() {
        return Err("尚未授权登录".to_string());
    }
    let issuance = refresh_tokens_blocking(&oauth)?;
    let new_refresh = issuance
        .refresh_token
        .clone()
        .unwrap_or_else(|| oauth.refresh_token.clone());
    let expires_at = issuance.expires_in.map(|secs| now.saturating_add(secs));
    write_oauth_tokens(
        config_path,
        provider,
        &issuance.access_token,
        &new_refresh,
        expires_at,
    )?;
    Ok(issuance.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth_with(expires_at: Option<i64>, access: &str, refresh: &str) -> ProviderOAuth {
        ProviderOAuth {
            authorize_url: "https://sso.example.com/authorize".to_string(),
            token_url: "https://sso.example.com/token".to_string(),
            client_id: "nuphus".to_string(),
            scopes: String::new(),
            use_pkce: true,
            redirect_port: None,
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_at,
        }
    }

    // ── 过期判断 ──

    #[test]
    fn fresh_token_passes_directly() {
        // 距过期还有 1 小时 → fresh
        assert!(token_is_fresh(Some(1_800_003_600), "tok", 1_800_000_000));
    }

    #[test]
    fn expiring_soon_requires_refresh() {
        // 距过期 30s（< 60s 裕量）→ 需要刷新
        assert!(!token_is_fresh(Some(1_800_000_030), "tok", 1_800_000_000));
    }

    #[test]
    fn expired_and_missing_expiry_require_refresh() {
        assert!(!token_is_fresh(Some(1_799_999_000), "tok", 1_800_000_000));
        // expires_at 缺失 = 时效未知 → 不可信
        assert!(!token_is_fresh(None, "tok", 1_800_000_000));
        // 空 token 一律不 fresh
        assert!(!token_is_fresh(Some(1_800_003_600), "", 1_800_000_000));
    }

    #[test]
    fn in_memory_fast_path_returns_token_only_when_fresh() {
        let fresh = oauth_with(Some(1_800_003_600), "tok", "rt");
        assert_eq!(
            in_memory_fresh_token(&fresh, 1_800_000_000).as_deref(),
            Some("tok")
        );
        let stale = oauth_with(Some(1_799_999_000), "tok", "rt");
        assert_eq!(in_memory_fresh_token(&stale, 1_800_000_000), None);
    }

    // ── token 响应解析 ──

    #[test]
    fn parse_standard_token_response() {
        let t = parse_token_response(
            r#"{"access_token":"at-123","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-9"}"#,
        )
        .unwrap();
        assert_eq!(t.access_token, "at-123");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-9"));
        assert_eq!(t.expires_in, Some(3600));
    }

    #[test]
    fn parse_response_without_refresh_token() {
        // 部分服务刷新时不下发新 refresh_token
        let t = parse_token_response(r#"{"access_token":"at-2","expires_in":1800}"#).unwrap();
        assert_eq!(t.access_token, "at-2");
        assert_eq!(t.refresh_token, None);
        assert_eq!(t.expires_in, Some(1800));
    }

    #[test]
    fn parse_error_response_surfaces_oauth_error_without_leaking_body() {
        let err =
            parse_token_response(r#"{"error":"invalid_grant","error_description":"code expired"}"#)
                .unwrap_err();
        assert!(
            err.contains("invalid_grant"),
            "应包含 OAuth error 码: {err}"
        );
        assert!(err.contains("code expired"));
    }

    #[test]
    fn parse_rejects_non_json_and_missing_access_token() {
        assert!(parse_token_response("not json").is_err());
        assert!(parse_token_response(r#"{"token_type":"Bearer"}"#).is_err());
    }
}
