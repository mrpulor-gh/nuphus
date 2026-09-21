//! custom 实例通用 OAuth2 接入（Authorization Code + PKCE + 本地回调）— 命令层。
//!
//! 三个命令：
//! - [`oauth_begin`]：拉起本地回调 server 并返回授权 URL（浏览器打开由前端
//!   既有 `openExternal` 完成，本模块不碰 UI）；回调→换令牌→加密落盘→发事件。
//! - [`oauth_status`]：登录状态查询（绝不回传令牌本身）。
//! - [`oauth_logout`]：清空令牌字段。
//!
//! 令牌协议交互与磁盘读写全部委托核心层 `nuphus::config::oauth`；本层只做
//! 回调 server（axum，仅绑 127.0.0.1，模式对齐 handoff_server.rs）、随机数、
//! PKCE 与事件分发。安全不变量与核心层一致：令牌不进日志 / 事件 / 错误文案。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use tauri::Emitter;

use nuphus::config::model::ProviderOAuth;
use nuphus::config::oauth as core_oauth;

use super::toml_ops::get_config_path;

/// 同一 provider 重复 begin 时旧会话的最长残留：回调超时上限。
const CALLBACK_TIMEOUT_SECS: u64 = 300;

// ============================================================================
// 随机数与 PKCE（RFC 7636）
// ============================================================================

/// CSPRNG 字节源：复用 uuid v4（getrandom 系统熵）。desktop 依赖树已有 uuid，
/// 不为随机数新增 crate 声明；单次 UUID 载荷 16 字节，其中 122 位为随机位
/// （6 位为 version/variant 固定位），对下述用途远超安全裕量。
fn random_uuid_string() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// OAuth state：32 位小写十六进制（一个 UUID v4 simple）。
fn random_state() -> String {
    random_uuid_string()
}

/// PKCE code_verifier：96 字符。三个 UUID v4 simple 拼接——字符全部落在
/// RFC 7636 §4.1 unreserved 集（ALPHA/DIGIT），熵 ≈ 366 位（规范建议 ≥256）。
fn pkce_verifier() -> String {
    let mut v = String::with_capacity(96);
    for _ in 0..3 {
        v.push_str(&random_uuid_string());
    }
    v
}

/// PKCE code_challenge：BASE64URL(SHA256(verifier)) 无填充（S256 方法）。
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// RFC 3986 query 值百分号编码：unreserved 集原样保留，其余逐字节 %XX。
/// 仅依赖标准库——authorize_url 的 query 拼接够用，不为一个函数引入 url crate。
fn percent_encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 拼接授权端点 URL：response_type=code / client_id / redirect_uri / state 必带；
/// scope 非空才带（RFC 6749 §4.1.1）；PKCE 开启时带 code_challenge + S256。
fn build_authorize_url(
    oauth: &ProviderOAuth,
    redirect_uri: &str,
    state: &str,
    challenge: Option<&str>,
) -> String {
    let mut pairs: Vec<(&str, String)> = vec![
        ("response_type", "code".to_string()),
        ("client_id", oauth.client_id.clone()),
        ("redirect_uri", redirect_uri.to_string()),
        ("state", state.to_string()),
    ];
    let scopes = oauth.scopes.trim();
    if !scopes.is_empty() {
        pairs.push(("scope", scopes.to_string()));
    }
    if let Some(challenge) = challenge {
        pairs.push(("code_challenge", challenge.to_string()));
        pairs.push(("code_challenge_method", "S256".to_string()));
    }
    let query = pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode_query_value(v)))
        .collect::<Vec<_>>()
        .join("&");
    let base = oauth.authorize_url.trim();
    if base.contains('?') {
        format!("{base}&{query}")
    } else {
        format!("{base}?{query}")
    }
}

// ============================================================================
// 回调分类（纯函数，便于 state 校验单测）
// ============================================================================

/// /callback 查询参数的裁决结果。
#[derive(Debug, Clone, PartialEq)]
enum CallbackOutcome {
    /// state 不符（CSRF 防线）：400，不终止会话。
    StateMismatch,
    /// 授权页关闭 / 用户拒绝（回调带 error 参数）：会话结束，ok:false。
    Denied(String),
    /// 拿到授权码：换令牌流程启动。
    Authorized(String),
}

fn classify_callback(query: &HashMap<String, String>, expected_state: &str) -> CallbackOutcome {
    let state = query.get("state").map(String::as_str).unwrap_or("");
    if state != expected_state {
        return CallbackOutcome::StateMismatch;
    }
    if let Some(err) = query
        .get("error")
        .map(String::as_str)
        .filter(|e| !e.is_empty())
    {
        // error_description 是授权服务器的可读说明（协议字段，不含令牌）
        let desc = query
            .get("error_description")
            .map(String::as_str)
            .unwrap_or("");
        return CallbackOutcome::Denied(if desc.is_empty() {
            format!("授权被拒绝: {err}")
        } else {
            format!("授权被拒绝: {err} — {desc}")
        });
    }
    match query
        .get("code")
        .map(String::as_str)
        .filter(|c| !c.is_empty())
    {
        Some(code) => CallbackOutcome::Authorized(code.to_string()),
        None => CallbackOutcome::StateMismatch,
    }
}

// ============================================================================
// 会话注册表（同一 provider 重复 begin：先关旧会话）
// ============================================================================

struct LoginSession {
    /// true = 请求关闭（被新会话顶替）。回调到达经 Notify 通道另行终止 serve。
    cancel: tokio::sync::watch::Sender<bool>,
}

fn sessions() -> &'static Mutex<HashMap<String, LoginSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, LoginSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 关闭该 provider 的旧登录会话（存在则返回 true）。
fn cancel_existing_session(provider: &str) -> bool {
    let existing = sessions()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(provider));
    match existing {
        Some(session) => {
            let _ = session.cancel.send(true);
            true
        }
        None => false,
    }
}

// ============================================================================
// 命令参数 / 返回 DTO
// ============================================================================

/// create/update_custom_provider 的可选 OAuth 配置入参（表单五项，camelCase）。
/// 令牌字段不在此出现——它们由登录/刷新路径独占维护。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OauthConfigDto {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    /// 空格分隔；空 = 授权请求不带 scope。
    pub scopes: String,
    /// 缺省 true（与 ProviderOAuth TOML 缺省语义一致；旧前端调用不传该字段时同样成立）。
    #[serde(default = "dto_default_use_pkce")]
    pub use_pkce: bool,
    pub redirect_port: Option<u16>,
}

fn dto_default_use_pkce() -> bool {
    true
}

/// oauth_begin 返回：授权 URL（前端经 openExternal 打开）与本地回调端口。
#[derive(Debug, serde::Serialize)]
pub struct OauthBeginResult {
    pub authorize_url: String,
    pub port: u16,
}

/// oauth_status 返回。蛇形命名与 ProviderInfo 现有字段口径一致。
#[derive(Debug, serde::Serialize)]
pub struct OauthStatus {
    pub configured: bool,
    pub logged_in: bool,
    pub expires_at: Option<i64>,
    pub needs_login: bool,
}

/// ProviderInfo.oauth 摘要：登录状态 + 配置五项（编辑表单回显），
/// **令牌字段永不出现**（状态里也只有过期时间，没有 token 值）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct OauthSummaryDto {
    pub configured: bool,
    pub logged_in: bool,
    pub expires_at: Option<i64>,
    /// 需重新登录：配置完整但未登录，或令牌已过期且无 refresh_token 可自愈。
    pub needs_login: bool,
    // ── 配置五项（camelCase 与 OauthConfigDto 一致，前端直接回填表单）──
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scopes: String,
    pub use_pkce: bool,
    pub redirect_port: Option<u16>,
}

impl OauthSummaryDto {
    pub fn from_oauth(oauth: &ProviderOAuth) -> Self {
        let logged_in = oauth.is_logged_in();
        let configured = oauth.config_complete();
        // 与 oauth_status 同一判定：无 refresh_token 可自愈且令牌已过期 → 需重新登录
        let now = core_oauth::now_unix();
        let fresh = core_oauth::token_is_fresh(oauth.expires_at, &oauth.access_token, now);
        let can_self_heal = !oauth.refresh_token.is_empty();
        OauthSummaryDto {
            configured,
            logged_in,
            expires_at: oauth.expires_at,
            needs_login: configured && (!logged_in || (!can_self_heal && !fresh)),
            // 配置五项原样带出（表单回显）；令牌字段到此为止，不进摘要。
            authorize_url: oauth.authorize_url.clone(),
            token_url: oauth.token_url.clone(),
            client_id: oauth.client_id.clone(),
            scopes: oauth.scopes.clone(),
            use_pkce: oauth.use_pkce,
            redirect_port: oauth.redirect_port,
        }
    }
}

/// ProviderInfo 组装用的摘要读取：段未配 oauth → None。
pub fn read_oauth_summary(provider: &str) -> Option<OauthSummaryDto> {
    let path = get_config_path()?;
    read_oauth_summary_in(&path, provider)
}

fn read_oauth_summary_in(path: &std::path::Path, provider: &str) -> Option<OauthSummaryDto> {
    let oauth = core_oauth::read_oauth_segment(path, provider)?;
    Some(OauthSummaryDto::from_oauth(&oauth))
}

// ============================================================================
// oauth_begin：本地回调 server + 授权换令牌全流程
// ============================================================================

/// 回调 server 收到的裁决（handler → 流程任务的唯一通道载荷）。
struct CallbackMsg {
    outcome: CallbackOutcome,
}

#[tauri::command]
pub async fn oauth_begin(
    app: tauri::AppHandle,
    provider: String,
) -> Result<OauthBeginResult, String> {
    let config_path =
        get_config_path().ok_or_else(|| "配置文件未找到，请先完成模型配置".to_string())?;
    let oauth = core_oauth::read_oauth_segment(&config_path, &provider).ok_or_else(|| {
        format!("实例 {provider} 未配置 OAuth 授权（请先在表单填写授权端点等信息）")
    })?;
    if !oauth.config_complete() {
        return Err("OAuth 配置不完整：授权端点、令牌端点与 Client ID 均为必填项".to_string());
    }

    // 回调端口：配置指定 → 绑定该端口；未指定 → OS 分配空闲端口。
    // 先绑定再返回，保证返回的 port 与实际监听一致。
    let listener = match oauth.redirect_port {
        Some(port) => tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("回调端口 {port} 绑定失败（可能被占用）: {e}"))?,
        None => tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("本地回调端口分配失败: {e}"))?,
    };
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取回调端口失败: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let state = random_state();
    let (verifier, challenge) = if oauth.use_pkce {
        let v = pkce_verifier();
        let c = pkce_challenge(&v);
        (Some(v), Some(c))
    } else {
        (None, None)
    };
    let authorize_url = build_authorize_url(&oauth, &redirect_uri, &state, challenge.as_deref());

    // 重复 begin：先关旧会话（旧任务静默退出——结果由本会话负责向用户报告）
    cancel_existing_session(&provider);

    let (msg_tx, msg_rx) = tokio::sync::mpsc::channel::<CallbackMsg>(1);
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());

    // 注册会话（先注册再返回，保证重复 begin 能立即顶替）
    sessions()
        .lock()
        .map_err(|e| format!("会话注册失败: {e}"))?
        .insert(provider.clone(), LoginSession { cancel: cancel_tx });

    let expected_state = state.clone();
    let task_provider = provider.clone();
    let task_app = app.clone();
    let cb_notify = notify.clone();
    tokio::spawn(async move {
        run_login_flow(
            task_app,
            task_provider,
            oauth,
            listener,
            port,
            expected_state,
            verifier,
            msg_tx,
            msg_rx,
            cancel_rx,
            cb_notify,
        )
        .await;
    });

    Ok(OauthBeginResult {
        authorize_url,
        port,
    })
}

/// 一次性回调 server + 令牌交换 + 落盘 + 事件。
///
/// 生命周期：serve 至「回调到达 / 被新会话顶替 / 5 分钟超时」三者先到者。
/// - 回调到达 → graceful shutdown → 换令牌（spawn_blocking，不阻塞 worker）
///   → 加密落盘 → `oauth-login-result` 事件。
/// - 被新 begin 顶替 → 静默退出（结果由新会话报告，避免双份事件）。
/// - 超时 → 事件 ok:false。
#[allow(clippy::too_many_arguments)]
async fn run_login_flow(
    app: tauri::AppHandle,
    provider: String,
    oauth: ProviderOAuth,
    listener: tokio::net::TcpListener,
    port: u16,
    expected_state: String,
    verifier: Option<String>,
    msg_tx: tokio::sync::mpsc::Sender<CallbackMsg>,
    mut msg_rx: tokio::sync::mpsc::Receiver<CallbackMsg>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    notify: std::sync::Arc<tokio::sync::Notify>,
) {
    let state_for_handler = expected_state;
    // handler 拿走 sender / notify 的副本：闭包内需要它们，函数后续还要用原件
    //（msg_tx 在 serve 结束后显式 drop 以让 recv 收敛；notify 供 shutdown 等待）。
    let msg_tx_handler = msg_tx.clone();
    let notify_handler = notify.clone();
    let router =
        axum::Router::new().route(
            "/callback",
            axum::routing::get(
                move |axum::extract::Query(params): axum::extract::Query<
                    HashMap<String, String>,
                >| async move {
                    match classify_callback(&params, &state_for_handler) {
                        CallbackOutcome::StateMismatch => {
                            // CSRF 防线：不是本会话发起的回调，拒绝但不终止等待
                            axum::http::StatusCode::BAD_REQUEST
                        }
                        outcome => {
                            // 拿到 code / 用户拒绝 → 交还结果并终止一次性 server。
                            // notify_one 带许可语义：shutdown future 未注册也能被唤醒。
                            let _ = msg_tx_handler.send(CallbackMsg { outcome }).await;
                            notify_handler.notify_one();
                            axum::http::StatusCode::OK
                        }
                    }
                },
            ),
        );

    // serve 终止条件：回调到达（notify）/ 被顶替（cancel）。
    // cancelled 标志区分两种静默语义：被顶替 → 完全静默。
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_flag = cancelled.clone();
    let wait_shutdown = {
        let notify = notify.clone();
        async move {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    shutdown_flag.store(true, std::sync::atomic::Ordering::Release);
                }
                _ = notify.notified() => {}
            }
        }
    };
    let serve = axum::serve(listener, router).with_graceful_shutdown(wait_shutdown);
    // 超时覆盖整个等待期（serve 至关停 + 消息取出）；换令牌 IO 不在此内。
    let waited = tokio::time::timeout(
        std::time::Duration::from_secs(CALLBACK_TIMEOUT_SECS),
        async move {
            let _ = serve.await;
            drop(msg_tx); // 释放原始发送端：无回调关停时 recv 才能以 None 收敛
            msg_rx.recv().await
        },
    )
    .await;

    // 会话清理：无论哪种终态，出注册表
    let _ = sessions().lock().ok().and_then(|mut m| m.remove(&provider));

    let msg = match waited {
        Ok(Some(msg)) => Some(msg),
        Ok(None) => {
            // 无回调即关停 = 被新 begin 顶替：不发事件，结果由新会话报告
            if !cancelled.load(std::sync::atomic::Ordering::Acquire) {
                emit_result(&app, &provider, false, Some("授权会话已取消".to_string()));
            }
            None
        }
        Err(_) => {
            emit_result(
                &app,
                &provider,
                false,
                Some("授权超时：5 分钟内未收到浏览器回调".to_string()),
            );
            None
        }
    };

    let Some(CallbackMsg { outcome }) = msg else {
        return;
    };
    match outcome {
        CallbackOutcome::Denied(reason) => emit_result(&app, &provider, false, Some(reason)),
        CallbackOutcome::Authorized(code) => {
            // redirect_uri 必须与授权请求逐字节一致（RFC 6749 §4.1.3）——
            // 用实际监听端口，redirect_port=None（OS 分配）时尤其如此。
            exchange_and_persist(app, provider, oauth, code, verifier, port).await;
        }
        CallbackOutcome::StateMismatch => unreachable!("classify_callback 已过滤"),
    }
}

/// code 换令牌 → 三字段加密写回 → 事件。阻塞 IO 全部 spawn_blocking。
async fn exchange_and_persist(
    app: tauri::AppHandle,
    provider: String,
    oauth: ProviderOAuth,
    code: String,
    verifier: Option<String>,
    port: u16,
) {
    let exchange = tokio::task::spawn_blocking(move || {
        core_oauth::exchange_code_blocking(&oauth, &code, verifier.as_deref().unwrap_or(""), port)
    })
    .await;
    let issuance = match exchange {
        Ok(Ok(i)) => i,
        Ok(Err(e)) => {
            emit_result(&app, &provider, false, Some(e));
            return;
        }
        Err(e) => {
            emit_result(
                &app,
                &provider,
                false,
                Some(format!("令牌交换任务失败: {e}")),
            );
            return;
        }
    };

    // provider 还要用于事件回传：写盘任务用独立克隆，避免闭包 move 后不可用。
    let provider_for_persist = provider.clone();
    let persist = tokio::task::spawn_blocking(move || {
        let path = get_config_path().ok_or_else(|| "配置文件未找到".to_string())?;
        core_oauth::write_oauth_tokens(
            &path,
            &provider_for_persist,
            &issuance.access_token,
            issuance.refresh_token.as_deref().unwrap_or(""),
            issuance
                .expires_in
                .map(|secs| core_oauth::now_unix() + secs),
        )
    })
    .await;
    match persist {
        Ok(Ok(())) => emit_result(&app, &provider, true, None),
        Ok(Err(e)) => emit_result(&app, &provider, false, Some(e)),
        Err(e) => emit_result(
            &app,
            &provider,
            false,
            Some(format!("令牌写盘任务失败: {e}")),
        ),
    }
}

/// 登录结果事件（`oauth-login-result`）：payload 只含 provider / ok / error，
/// 任何路径都不携带令牌或令牌片段。
fn emit_result(app: &tauri::AppHandle, provider: &str, ok: bool, error: Option<String>) {
    let payload = serde_json::json!({ "provider": provider, "ok": ok, "error": error });
    if let Err(e) = app.emit("oauth-login-result", payload) {
        tracing::warn!("[oauth] {provider} 结果事件发送失败: {e}");
    }
    tracing::info!(
        "[oauth] {provider} 授权登录{}",
        if ok { "成功" } else { "失败" }
    );
}

// ============================================================================
// oauth_status / oauth_logout
// ============================================================================

/// 查询登录状态。读盘 + 纯判定，不触发网络刷新。
#[tauri::command]
pub fn oauth_status(provider: String) -> Result<OauthStatus, String> {
    let path = get_config_path().ok_or_else(|| "配置文件未找到".to_string())?;
    oauth_status_in(&path, &provider)
}

fn oauth_status_in(path: &std::path::Path, provider: &str) -> Result<OauthStatus, String> {
    let Some(oauth) = core_oauth::read_oauth_segment(path, provider) else {
        return Ok(OauthStatus {
            configured: false,
            logged_in: false,
            expires_at: None,
            needs_login: false,
        });
    };
    let now = core_oauth::now_unix();
    let logged_in = oauth.is_logged_in();
    let configured = oauth.config_complete();
    let fresh = core_oauth::token_is_fresh(oauth.expires_at, &oauth.access_token, now);
    let can_self_heal = !oauth.refresh_token.is_empty();
    Ok(OauthStatus {
        configured,
        logged_in,
        expires_at: oauth.expires_at,
        // 无法自愈才需要重新登录：未登录，或已过期且无 refresh_token
        needs_login: configured && (!logged_in || (!can_self_heal && !fresh)),
    })
}

/// 登出：清空三个令牌字段落盘（配置五项保留）。段未配 oauth → 幂等成功。
#[tauri::command]
pub fn oauth_logout(provider: String) -> Result<(), String> {
    let path = get_config_path().ok_or_else(|| "配置文件未找到".to_string())?;
    core_oauth::clear_oauth_tokens(&path, &provider)?;
    tracing::info!("[oauth] {provider} 已登出（令牌已清空）");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth_fixture() -> ProviderOAuth {
        ProviderOAuth {
            authorize_url: "https://sso.example.com/authorize".to_string(),
            token_url: "https://sso.example.com/token".to_string(),
            client_id: "nuphus-cli".to_string(),
            scopes: String::new(),
            use_pkce: true,
            redirect_port: None,
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: None,
        }
    }

    // ── PKCE ──

    #[test]
    fn pkce_verifier_length_and_charset() {
        let v = pkce_verifier();
        assert_eq!(v.len(), 96, "verifier 长度应固定 96（43..=128 合法区间内）");
        assert!(
            v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')),
            "verifier 字符必须全部属于 RFC7636 unreserved 集"
        );
        // 两次生成都不同（随机性存在）
        assert_ne!(v, pkce_verifier());
    }

    #[test]
    fn pkce_challenge_is_recomputable_s256() {
        let v = pkce_verifier();
        let c1 = pkce_challenge(&v);
        let c2 = pkce_challenge(&v);
        assert_eq!(c1, c2, "同一 verifier 的 challenge 必须可复算");
        assert!(!c1.contains('='), "BASE64URL 无填充");
        assert!(
            !c1.contains('+') && !c1.contains('/'),
            "必须使用 URL-safe 字母表"
        );
        // S256 公式复核：challenge == BASE64URL_NO_PAD(SHA256(verifier))
        let digest = Sha256::digest(v.as_bytes());
        assert_eq!(
            c1,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
        );
    }

    #[test]
    fn state_is_32_hex_chars() {
        let s = random_state();
        assert_eq!(s.len(), 32);
        assert!(s
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(s, random_state());
    }

    // ── 授权 URL 拼接 ──

    #[test]
    fn authorize_url_includes_pkce_and_omits_empty_scope() {
        let mut oauth = oauth_fixture();
        oauth.scopes = "read write".to_string();
        let challenge = pkce_challenge("some-verifier");
        let url = build_authorize_url(
            &oauth,
            "http://127.0.0.1:19110/callback",
            "st-1",
            Some(&challenge),
        );
        assert!(url.starts_with("https://sso.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=nuphus-cli"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A19110%2Fcallback"));
        assert!(url.contains("scope=read%20write"));
        assert!(url.contains(&format!("code_challenge={challenge}")));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=st-1"));

        // scope 空 → 不带 scope；PKCE 关 → 不带 challenge
        let mut oauth = oauth_fixture();
        oauth.use_pkce = false;
        let url = build_authorize_url(&oauth, "http://127.0.0.1:1/callback", "s", None);
        assert!(!url.contains("scope="));
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn authorize_url_appends_with_ampersand_when_base_has_query() {
        let mut oauth = oauth_fixture();
        oauth.authorize_url = "https://sso.example.com/authorize?prompt=login".to_string();
        let url = build_authorize_url(&oauth, "http://127.0.0.1:1/callback", "s", None);
        assert!(url.starts_with("https://sso.example.com/authorize?prompt=login&"));
    }

    // ── state 校验 / 回调分类 ──

    fn query(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn callback_state_mismatch_is_rejected() {
        let q = query(&[("state", "other"), ("code", "abc")]);
        assert_eq!(
            classify_callback(&q, "expected"),
            CallbackOutcome::StateMismatch
        );
        // 缺 state 同样视为不匹配
        let q = query(&[("code", "abc")]);
        assert_eq!(
            classify_callback(&q, "expected"),
            CallbackOutcome::StateMismatch
        );
        // state 对但既无 code 也无 error（授权页被直接关闭的畸形回调）→ 不匹配
        let q = query(&[("state", "expected")]);
        assert_eq!(
            classify_callback(&q, "expected"),
            CallbackOutcome::StateMismatch
        );
    }

    #[test]
    fn callback_error_param_maps_to_denied() {
        let q = query(&[("state", "expected"), ("error", "access_denied")]);
        assert_eq!(
            classify_callback(&q, "expected"),
            CallbackOutcome::Denied("授权被拒绝: access_denied".to_string())
        );
        let q = query(&[
            ("state", "expected"),
            ("error", "access_denied"),
            ("error_description", "user clicked cancel"),
        ]);
        assert_eq!(
            classify_callback(&q, "expected"),
            CallbackOutcome::Denied("授权被拒绝: access_denied — user clicked cancel".to_string())
        );
    }

    #[test]
    fn callback_valid_code_maps_to_authorized() {
        let q = query(&[("state", "expected"), ("code", "auth-code-1")]);
        assert_eq!(
            classify_callback(&q, "expected"),
            CallbackOutcome::Authorized("auth-code-1".to_string())
        );
    }

    // ── percent 编码 ──

    #[test]
    fn percent_encode_keeps_unreserved_only() {
        assert_eq!(percent_encode_query_value("abcXYZ09-._~"), "abcXYZ09-._~");
        assert_eq!(percent_encode_query_value("a b"), "a%20b");
        assert_eq!(
            percent_encode_query_value("http://x/y?z=1"),
            "http%3A%2F%2Fx%2Fy%3Fz%3D1"
        );
    }
}
