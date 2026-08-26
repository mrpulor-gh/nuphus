//! 移动端局域网 HTTP+WS server（axum，绑 0.0.0.0，默认关闭）
//!
//! 手机 = 同一会话的第二块屏：POST /message 走 P0 抽取的共享入口
//! `commands::process::submit_user_message`（source="mobile"），与桌面共用
//! 同一 leader_agent / busy 锁 / 去重逻辑；GET /ws 把 NuphusEvent 实时推给手机。
//!
//! 模式对齐 handoff_server.rs（TcpListener::bind + axum::serve + token 鉴权 +
//! tokio 生命周期），差异：绑 0.0.0.0（手机在局域网另一台设备上），独立默认端口。
//!
//! 生命周期：默认不启动。设置页命令 mobile_server_start/stop 控制；配置
//! （enabled/port/token）持久化到 config_dir/mobile_server.json，enabled=true 时
//! 应用 setup 阶段自动恢复启动。server 停止 = AppState.mobile_ws_tx 置 None，
//! CompoundEmitter 退化为纯 Tauri 推送（与桌面单端行为完全等价）。

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Query, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, RwLock};
use tauri::{Emitter, Manager};

use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};

/// 默认端口（避开 handoff 门铃 18771）
pub const DEFAULT_PORT: u16 = 18772;

/// WS broadcast channel 容量（满时慢客户端 lagged 丢帧，不阻塞桌面事件路径）
const WS_BROADCAST_CAPACITY: usize = 256;

/// WS 心跳间隔：iOS/微信 WebView 切后台会静默挂起 TCP，onclose 永不触发——
/// 定期向本连接下发心跳帧，send 失败即判定连接假死并回收
const WS_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// POST /message 请求体上限。前端最多 9 张图 × 500KB（压缩后，Composer.tsx
/// MAX_IMAGES/MAX_IMAGE_BYTES），data URL base64 膨胀 4/3 ≈ 6MB，再加 message
/// 文本与 JSON 结构，设 8MB 覆盖并留余量。对齐前端常量，勿单边改动。
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

// ============================================================================
// 配置持久化（沿用 tool_permissions.json 模式：config_dir 下的 JSON 文件）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileServerConfig {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    /// 配对密码哈希 "{salt}:{sha256_hex}"；空串 = 未设置密码。
    /// #[serde(default)] 保证旧配置文件无此字段时仍能反序列化。
    #[serde(default)]
    pub password_hash: String,
}

impl Default for MobileServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
            password_hash: String::new(),
        }
    }
}

fn config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("nuphus")
        .join("mobile_server.json")
}

/// 剥离 UTF-8 BOM：外部编辑器（记事本等）保存配置常带 BOM，
/// serde_json 对首字节 BOM 直接解析失败——静默回退默认值会清掉 enabled/token
/// （实测事故：配对二维码永远不出现）。加载器必须全部免疫。
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

fn load_config_from(path: &std::path::Path) -> MobileServerConfig {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<MobileServerConfig>(strip_bom(&raw)) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::error!(
                    "[Mobile] mobile_server.json 解析失败（回退默认值，原文件保留）: {e}"
                );
                MobileServerConfig::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => MobileServerConfig::default(),
        Err(e) => {
            tracing::error!("[Mobile] mobile_server.json 读取失败（回退默认值）: {e}");
            MobileServerConfig::default()
        }
    }
}

fn save_config_to(path: &std::path::Path, cfg: &MobileServerConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let data = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    std::fs::write(path, data).map_err(|e| format!("写入配置失败: {e}"))
}

pub fn load_config() -> MobileServerConfig {
    load_config_from(&config_path())
}

fn save_config(cfg: &MobileServerConfig) -> Result<(), String> {
    save_config_to(&config_path(), cfg)
}

/// 密码学安全随机 token（UUIDv4 simple = 32 hex 字符，≥32 要求）
fn generate_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 配对密码哈希：返回 "{salt}:{hex}"。salt=UUIDv4 simple（32 hex），
/// hash=SHA256(salt:password) 的 hex。不存明文；同密码每次 salt 不同。
fn hash_password(password: &str) -> String {
    let salt = uuid::Uuid::new_v4().simple().to_string();
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    format!("{salt}:{}", hex::encode(hasher.finalize()))
}

/// 校验密码与存储的 "{salt}:{hex}" 是否匹配。格式非法（无 ':' 或分段为空）→ false。
fn verify_password(password: &str, stored: &str) -> bool {
    let Some((salt, expected_hex)) = stored.split_once(':') else {
        return false;
    };
    if salt.is_empty() || expected_hex.is_empty() {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize()) == expected_hex
}

/// 配对密码强度校验：≥6 位，且同时含 ASCII 字母和数字。
fn validate_password(password: &str) -> Result<(), String> {
    if password.len() < 6 {
        return Err("密码长度至少 6 位".into());
    }
    let has_letter = password.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    if !has_letter || !has_digit {
        return Err("密码需同时包含字母和数字".into());
    }
    Ok(())
}

// ============================================================================
// 状态与鉴权
// ============================================================================

/// server 运行状态（供 status/start 命令返回，P3 设置页二维码用 token + lan_url）
#[derive(Debug, Clone, Serialize)]
pub struct MobileServerStatus {
    pub running: bool,
    pub port: u16,
    pub token: String,
    pub lan_url: Option<String>,
    /// 是否已设置配对密码（设置页据此展示"修改密码"而非"设置密码"）
    pub password_set: bool,
}

/// /pair 防暴力破解状态（内存态，不持久化）：
/// 连续 MAX_PAIR_FAILURES 次密码错误 → 锁 LOCK_SECS 秒。
#[derive(Debug, Clone, Copy, Default)]
struct PairThrottle {
    failures: u32,
    locked_until: Option<std::time::Instant>,
}

/// 连续失败多少次后触发锁定
const MAX_PAIR_FAILURES: u32 = 5;
/// 锁定持续秒数
const PAIR_LOCK_SECS: u64 = 60;

/// axum handler 共享上下文（Runtime 泛化：生产 Wry，测试 MockRuntime）
/// Clone 手写实现——derive 会额外要求 R: Clone（MockRuntime 不满足）。
struct MobileCtx<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    /// 与 AppState.mobile_token 同一 Arc——regenerate 即时生效，无需重启 server
    token: Arc<RwLock<String>>,
    ws_tx: tokio::sync::broadcast::Sender<String>,
    /// 实际监听端口（绑 0.0.0.0；端口占用时可退化为 OS 分配）。/relay-hint 下发 lan_url 用
    port: u16,
    /// /pair 防暴力破解状态（Arc 共享给 router clone 出的 ctx 副本）
    pair_throttle: Arc<std::sync::Mutex<PairThrottle>>,
    /// Workflow 引擎引用（与 AppState.workflow_engine 同一 Arc）：
    /// WS 连接级订阅 WorkflowEvent 转发 + 控制端点复用引擎控制命令（pause/resume/cancel）
    workflow_engine: Arc<tokio::sync::RwLock<nuphus::workflow::WorkflowEngine>>,
}

impl<R: tauri::Runtime> Clone for MobileCtx<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            token: self.token.clone(),
            ws_tx: self.ws_tx.clone(),
            port: self.port,
            pair_throttle: self.pair_throttle.clone(),
            workflow_engine: self.workflow_engine.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AuthQuery {
    token: Option<String>,
    /// 可选 mode（leader/workflow/custom）：/model-config 按 mode 返回生效模型
    mode: Option<String>,
}

/// 鉴权：Header `X-Mobile-Token` 或 query `?token=`（浏览器 WebSocket API
/// 无法设置自定义 Header，故 WS 走 query；两种渠道对所有鉴权端点等效）
fn token_valid(headers: &HeaderMap, query: &AuthQuery, expected: &RwLock<String>) -> bool {
    let provided = headers
        .get("X-Mobile-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| query.token.clone());
    let provided = match provided {
        Some(p) if !p.is_empty() => p,
        _ => return false,
    };
    let expected = match expected.read() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };
    !expected.is_empty() && provided == expected
}

// ============================================================================
// Handlers
// ============================================================================

/// 连通性自检，无需鉴权
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "nuphus-mobile" }))
}

/// POST /pair 请求体：配对密码
#[derive(Debug, Deserialize)]
struct MobilePairRequest {
    password: String,
}

/// POST /pair：配对密码换取访问 token（唯一无 token 鉴权的业务端点，但防暴力破解）。
///
/// 分支与状态码：
/// - 锁定期内（连续失败 ≥5 次后 60s）→ 429 `{"error":"尝试过多，请 N 秒后重试"}`
/// - 桌面端未设置配对密码 → 503 `{"error":"桌面端未设置配对密码"}`
/// - 密码错误 → 401 `{"error":"密码错误"}`（failures+1；达到 5 次锁定 60s）
/// - 密码正确 → 200 `{"token":"<当前 token>"}`
///
/// 锁（std::sync::Mutex）临界区只做计数/时间检查与递增，校验密码不持锁。
async fn post_pair<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    Json(payload): Json<MobilePairRequest>,
) -> Response {
    // 1) 防暴力破解：锁定期内拒绝；已过期则清零重计（短临界区，无 await）
    let now = std::time::Instant::now();
    {
        let mut throttle = ctx.pair_throttle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(locked_until) = throttle.locked_until {
            if now < locked_until {
                let remain = locked_until.duration_since(now).as_secs();
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({ "error": format!("尝试过多，请 {remain} 秒后重试") })),
                )
                    .into_response();
            }
            // 锁定已过期：清零重新计数
            throttle.failures = 0;
            throttle.locked_until = None;
        }
    }

    // 2) 密码校验（纯 CPU，不持锁）
    let cfg = load_config();
    if cfg.password_hash.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "桌面端未设置配对密码" })),
        )
            .into_response();
    }
    if !verify_password(&payload.password, &cfg.password_hash) {
        // 3) 失败计数（短临界区，无 await）
        let mut throttle = ctx.pair_throttle.lock().unwrap_or_else(|e| e.into_inner());
        throttle.failures += 1;
        if throttle.failures >= MAX_PAIR_FAILURES {
            throttle.locked_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(PAIR_LOCK_SECS));
            throttle.failures = 0;
        }
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "密码错误" })),
        )
            .into_response();
    }

    // 4) 成功：签发当前 token（读法对齐 token_valid）
    let token = match ctx.token.read() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };
    (StatusCode::OK, Json(serde_json::json!({ "token": token }))).into_response()
}

/// GET /history：拉取当前 leader_agent session 的对话历史（与桌面 get_chat_history
/// 同一份数据、同一过滤逻辑——手机打开页面必须先看到与桌面一致的历史）。
/// agent 不存在或 session 为空 → 空列表。
async fn get_history<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::session::chat_history(state.inner()) {
        Ok(messages) => (StatusCode::OK, Json(messages)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /confirm 请求体：危险操作确认回执
#[derive(Debug, Deserialize)]
struct MobileConfirm {
    action_id: String,
    approved: bool,
    /// true = 此对话内同工具不再询问（对齐桌面 approve_session_security）
    session: Option<bool>,
    tool: Option<String>,
}

/// 危险操作确认上行：手机端确认卡提交入口。
/// 解析链路与桌面 security.rs 命令完全一致（SharedSignals 信号队列，
/// agent 侧 check_security_result 轮询消费），核心安全 crate 零改动。
/// 处理后向桌面窗口广播 mobile-security-resolved，桌面弹窗同步关闭。
async fn post_confirm<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileConfirm>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.action_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "action_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    if payload.approved && payload.session == Some(true) {
        if let Some(ref tool) = payload.tool {
            nuphus::security::approve_session_tool(&state.signals, tool);
        }
    }
    nuphus::security::set_security_result(&state.signals, &payload.action_id, payload.approved);
    if let Ok(mut pending) = state.execution.lock() {
        if let Some(entry) = pending.pending_security.get_mut(&payload.action_id) {
            entry.approved = Some(payload.approved);
        }
    }
    // 桌面弹窗同步关闭（桌面内部事件，非 NuphusEvent 协议）
    let _ = ctx.app.emit(
        "mobile-security-resolved",
        serde_json::json!({ "action_id": payload.action_id }),
    );
    tracing::info!(
        "[Mobile] 安全确认回执: action={} approved={} session={:?}",
        payload.action_id,
        payload.approved,
        payload.session
    );
    StatusCode::OK.into_response()
}

/// POST /user-input 请求体：request_user_input 回执（手机端输入提交）
#[derive(Debug, Deserialize)]
struct MobileUserInput {
    action_id: String,
    value: String,
}

/// 手机端输入提交：对齐桌面 submit_user_input（user_input::submit 写入 response，
/// agent 侧 poll_response 消费后继续执行）。提交成功后广播 mobile-user-input-resolved，
/// 桌面 UserInputPrompt 弹窗同步关闭（与 /confirm → mobile-security-resolved 同模式）。
async fn post_user_input<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileUserInput>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.action_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "action_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    let exists = nuphus::security::user_input::get(&state.signals, &payload.action_id).is_some();
    if !exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "输入请求不存在或已过期" })),
        )
            .into_response();
    }
    nuphus::security::user_input::submit(&state.signals, &payload.action_id, payload.value);
    let _ = ctx.app.emit(
        "mobile-user-input-resolved",
        serde_json::json!({ "action_id": payload.action_id }),
    );
    tracing::info!(
        "[Mobile] user-input submitted: action={}",
        payload.action_id
    );
    StatusCode::OK.into_response()
}

/// 手机端输入取消：对齐桌面 reject_user_input（写入 __CANCELLED__，agent 立即醒来继续）。
async fn post_user_input_reject<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileUserInput>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.action_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "action_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    nuphus::security::user_input::cancel(&state.signals, &payload.action_id);
    let _ = ctx.app.emit(
        "mobile-user-input-resolved",
        serde_json::json!({ "action_id": payload.action_id }),
    );
    tracing::info!(
        "[Mobile] user-input cancelled: action={}",
        payload.action_id
    );
    StatusCode::OK.into_response()
}

/// POST /rating 请求体：手机端点评提交（对齐桌面 submit_execution_rating）
#[derive(Debug, Deserialize)]
struct MobileRating {
    goal: String,
    rating: u8,
    comment: String,
    tools_summary: String,
    steps_json: String,
    session_id: String,
}

/// 手机端点评提交：复用桌面 submit_execution_rating（纯函数，无 Tauri 依赖）。
async fn post_rating<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileRating>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::commands::memory::submit_execution_rating(
        payload.goal.clone(),
        payload.rating,
        payload.comment,
        payload.tools_summary,
        payload.steps_json,
        payload.session_id,
    ) {
        Ok(_) => {
            tracing::info!(
                "[Mobile] rating submitted: rating={} goal={}",
                payload.rating,
                payload.goal.chars().take(40).collect::<String>()
            );
            StatusCode::OK.into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /refine：手机端触发会话提炼（对齐桌面 execute_session_refine）。
/// 复用 refine.rs 双槽逻辑；事件经 CompoundEmitter 双推（桌面 + 手机 WS）。
/// 与桌面端 refine 互斥：refine_active 原子锁保证并发安全。
async fn post_refine<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::refine::execute_session_refine(ctx.app.clone(), state).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "refined", "message": msg })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /refine-skip：一端跳过提炼 → 广播 RefineSkipped（双端同步关闭 refine 弹窗，
/// 避免「手机点了跳过、电脑端弹窗还在」的状态残留）
async fn post_refine_skip<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    crate::commands::process::refine::broadcast_refine_skip(ctx.app.clone(), state.inner());
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "skipped" })),
    )
        .into_response()
}

// ============================================================================
// 执行控制（暂停 / 继续 / 终止 / 优雅停止）
//
// 复用桌面 lifecycle.rs 命令逻辑（禁止改其语义）：
// - /resume、/terminate、/stop 直接调用已 pub 的桌面命令——签名只含
//   tauri::State<'_, AppState>，与 Runtime 泛型无关，ctx.app.state::<AppState>()
//   返回的 State 可直接传入。
// - /pause 例外：桌面 pause_execution 签名带 tauri::AppHandle（Wry 具体类型），
//   泛型 R handler 中类型不匹配，故内联等价逻辑并用 CompoundEmitter 双推
//   ExecutionPaused（桌面 + 手机 WS 同时收到，两端 UI 一致进入暂停态）。
// ============================================================================

/// /resume、/terminate 请求体：回传桌面 /pause 广播时携带的 action_id
#[derive(Debug, Deserialize)]
struct MobilePauseDecision {
    action_id: String,
}

/// POST /pause：暂停执行。等价桌面 pause_execution：
/// set_pause_action_id + pause_flag.store(true) + 立即发射 ExecutionPaused
/// （CompoundEmitter 双推，Agent 循环到检查点复用同一 action_id 等待决策）。
async fn post_pause<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    let action_id = uuid::Uuid::new_v4().to_string();
    nuphus::agent::pause::set_pause_action_id(&state.signals, &action_id);
    state
        .pause_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);

    // 立即双推 ExecutionPaused——桌面弹暂停菜单、手机切暂停态，两端同步
    let emitter = crate::emitter::CompoundEmitter::new(ctx.app.clone(), state.inner());
    emitter.emit(NuphusEvent::ExecutionPaused {
        action_id: action_id.clone(),
    });

    tracing::info!("[Mobile] PAUSE: action_id={}", action_id);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "paused", "action_id": action_id })),
    )
        .into_response()
}

/// POST /resume：继续执行（复用桌面 continue_execution）
async fn post_resume<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobilePauseDecision>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.action_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "action_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::lifecycle::continue_execution(state, payload.action_id) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "resumed" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /terminate：终止执行（复用桌面 terminate_execution）
async fn post_terminate<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobilePauseDecision>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.action_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "action_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::lifecycle::terminate_execution(state, payload.action_id) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "terminated" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// POST /stop：优雅停止（复用桌面 graceful_stop——预置 Terminate 决策 + pause_flag，
/// 跳过暂停菜单，Agent 循环直接走优雅退出路径）。不广播 ExecutionPaused，
/// 与桌面 graceful_stop 行为完全一致。
async fn post_stop<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::lifecycle::graceful_stop(state) {
        Ok(action_id) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "stopping", "action_id": action_id })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

// ── 工作流遥控（WorkflowEngine 控制命令，等价桌面 wf_pause / wf_resume / wf_stop）──

/// POST /workflow-* 请求体：目标工作流 id
#[derive(Debug, Deserialize)]
struct WorkflowControlPayload {
    workflow_id: String,
}

/// POST /workflow-pause：暂停工作流执行（复用 engine.pause_workflow，等价桌面 wf_pause）。
async fn post_workflow_pause<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<WorkflowControlPayload>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.workflow_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "workflow_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    let engine = state.workflow_engine.read().await;
    engine.pause_workflow(&payload.workflow_id).await;
    tracing::info!(
        "[Mobile] workflow-pause: workflow_id={}",
        payload.workflow_id
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "paused" })),
    )
        .into_response()
}

/// POST /workflow-resume：继续工作流执行（复用 engine.resume_workflow，等价桌面 wf_resume）。
async fn post_workflow_resume<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<WorkflowControlPayload>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.workflow_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "workflow_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    let engine = state.workflow_engine.read().await;
    engine.resume_workflow(&payload.workflow_id).await;
    tracing::info!(
        "[Mobile] workflow-resume: workflow_id={}",
        payload.workflow_id
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "resumed" })),
    )
        .into_response()
}

/// POST /workflow-stop：终止工作流执行（复用 engine.cancel_workflow + mark_user_cancelled，
/// 与桌面 wf_stop 完全一致——后者额外标记用户主动取消，避免被当作失败重试）。
async fn post_workflow_stop<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<WorkflowControlPayload>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.workflow_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "workflow_id cannot be empty" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    let engine = state.workflow_engine.read().await;
    engine.cancel_workflow(&payload.workflow_id).await;
    nuphus::workflow::hud_control::mark_user_cancelled();
    tracing::info!(
        "[Mobile] workflow-stop: workflow_id={}",
        payload.workflow_id
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "stopped" })),
    )
        .into_response()
}

// ============================================================================
// SPA 静态资源伺服（移动端 Web 客户端）
// ============================================================================

/// 资源解析顺序：开发期文件系统 frontend/dist（npm run build 新鲜产物）→
/// 生产期 Tauri 内嵌资源（frontendDist 构建期嵌入二进制，asset_resolver 读取）。
/// 两种模式共用同一解析入口，路径差异在此收敛。
fn resolve_asset<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    rel: &str,
) -> Option<(Vec<u8>, String)> {
    // 路径穿越防护
    if rel.split('/').any(|seg| seg == ".." || seg.contains('\\')) {
        return None;
    }
    // 开发期：文件系统 dist
    let fs_path = crate::commands::process::workspace_root()
        .join("frontend")
        .join("dist")
        .join(rel);
    if let Ok(bytes) = std::fs::read(&fs_path) {
        return Some((bytes, mime_for(rel).to_string()));
    }
    // 生产期：内嵌资源（Asset 自带 mime_type）
    app.asset_resolver()
        .get(rel.to_string())
        .map(|a| (a.bytes, a.mime_type))
}

fn mime_for(rel: &str) -> &'static str {
    match rel.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json; charset=utf-8",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn serve_index<R: tauri::Runtime>(State(ctx): State<MobileCtx<R>>) -> Response {
    serve_asset_with_rel(&ctx, "mobile.html")
}

/// 静态子资源归属标记改写：HTML 内相对根引用 "./x" → "/d/<device_id>/x"。
/// 背景：浏览器子资源（script/css/icon）既不继承导航 URL 的 ?device=，也无法带
/// 自定义头——公共中继多设备在线时无标记资产被 Ambiguous 引导页顶替（HTML 当
/// JS 执行失败 → 白屏）。前缀由中继提取归属后原样转发，本服务 fallback 剥前缀
/// 按普通资产伺服；URL 含稳定 device_id + 内容 hash → HTTP 缓存键不受影响。
fn mark_html_assets(html: &str, device_id: &str) -> String {
    if device_id.is_empty() {
        return html.to_string();
    }
    html.replace("\"./", &format!("\"/d/{device_id}/"))
}

/// SPA 静态资源兜底（Router fallback：无显式路由时按路径伺服移动端资源）。
///
/// 原实现用 `/*path` 通配路由，但 matchit 0.7 的通配优先级会**遮蔽**更具体的
/// `/plugins/*rest` 插件路由（实测 /plugins/{id}/ 落到全局通配）。改 fallback
/// 语义等价：所有未匹配路径 → serve_asset_with_rel（内部 trim 前导斜杠，
/// 缓存/CSP 行为与移动端 assets/ 策略完全一致，未改路由语义）。
async fn serve_asset_fallback<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    uri: axum::extract::OriginalUri,
) -> Response {
    // 归属前缀剥离：/d/<device_id>/assets/x.js → /assets/x.js。归属判定已在
    // 中继隧道侧完成（能到达本连接即路由正确），此处只按资产语义处理剩余路径。
    let path = match uri.path().strip_prefix("/d/") {
        Some(rest) => match rest.split_once('/') {
            Some((_, after)) => format!("/{after}"),
            None => "/".to_string(),
        },
        None => uri.path().to_string(),
    };
    serve_asset_with_rel(&ctx, &path)
}

fn serve_asset_with_rel<R: tauri::Runtime>(ctx: &MobileCtx<R>, rel: &str) -> Response {
    let rel = rel.trim_start_matches('/');
    let rel = if rel.is_empty() { "mobile.html" } else { rel };
    match resolve_asset(&ctx.app, rel) {
        Some((bytes, mime)) => {
            // 移动端入口 HTML：注入子资源归属前缀（见 mark_html_assets 文档）
            let bytes = if rel == "mobile.html" && mime.starts_with("text/html") {
                let device_id = crate::relay_client::load_config().device_id;
                match std::str::from_utf8(&bytes) {
                    Ok(s) => mark_html_assets(s, &device_id).into_bytes(),
                    Err(_) => bytes,
                }
            } else {
                bytes
            };
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                mime.parse().unwrap_or(axum::http::HeaderValue::from_static(
                    "application/octet-stream",
                )),
            );
            headers.insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static(if rel.starts_with("assets/") {
                    "public, max-age=31536000, immutable"
                } else {
                    "no-cache"
                }),
            );
            // CSP 纵深防御（配合前端链接协议白名单）：script-src 'self' 使
            // javascript: 伪协议在浏览器层直接被拒；connect-src 放行 ws/http
            // 是因为手机端会跨源探测桌面局域网地址（probeLanDirect /health）
            if mime.starts_with("text/html") {
                headers.insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    axum::http::HeaderValue::from_static(
                        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                         img-src 'self' data: blob: https:; connect-src 'self' ws: wss: http: https:; \
                         worker-src 'self' blob:; font-src 'self' data:",
                    ),
                );
            }
            (StatusCode::OK, headers, bytes).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "not found",
                "hint": "移动端资源未构建：frontend/ 下执行 npm run build"
            })),
        )
            .into_response(),
    }
}

/// 插件 HTML 的 CSP 纵深防御：插件运行在 opaque origin（iframe sandbox
/// 无 allow-same-origin），此 CSP 约束其自身页面——脚本/style 允许内联
/// （插件模板惯例），connect-src 仅同源（127.0.0.1:port 的 /plugins 与
/// /plugins-shared），静态资源经同源 fetch 可访问。
const PLUGIN_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self' data:";

/// GET /plugins/*rest → 插件静态资源（rest = "{id}[/相对路径]"）。
///
/// 用单 catch-all 而非 `/plugins/:id/*path`：后者（axum 0.7 + matchit 0.7）不匹配
/// 带尾斜杠的 iframe 入口 URL `/plugins/{id}/`，catch-all 手动 split 覆盖三种形态：
///   /plugins/{id}                → index.html
///   /plugins/{id}/               → index.html（iframe 默认形态，设计文档 §4.2）
///   /plugins/{id}/assets/app.js  → 相对文件
async fn serve_plugin<R: tauri::Runtime>(
    State(_ctx): State<MobileCtx<R>>,
    axum::extract::Path(rest): axum::extract::Path<String>,
) -> Response {
    let rest = rest.trim_start_matches('/');
    let (id, path) = match rest.split_once('/') {
        Some((id, p)) => (id.to_string(), p.to_string()),
        None => (rest.to_string(), String::new()),
    };
    serve_plugin_file(&id, &path)
}

/// 插件文件伺服（与 serve_asset_with_rel 同策略：HTML no-cache + CSP，
/// 其余资源短缓存；走既有压缩层，不裸写 Response）
fn serve_plugin_file(id: &str, rel: &str) -> Response {
    if !crate::plugin_apps::valid_plugin_id(id) {
        return (StatusCode::BAD_REQUEST, "invalid plugin id").into_response();
    }
    let rel = rel.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    // 路径穿越防护：拒绝 .. / 绝对路径 / Windows 盘符（与 resolve_asset 同风格）
    if rel
        .split('/')
        .any(|seg| seg == ".." || seg.contains('\\') || seg.is_empty())
        || std::path::Path::new(rel).is_absolute()
    {
        return (StatusCode::BAD_REQUEST, "path traversal rejected").into_response();
    }
    let fs_path = crate::plugin_apps::apps_root().join(id).join(rel);
    let Ok(bytes) = std::fs::read(&fs_path) else {
        return (StatusCode::NOT_FOUND, "plugin asset not found").into_response();
    };
    let mime = mime_for(rel);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        mime.parse().unwrap_or(axum::http::HeaderValue::from_static(
            "application/octet-stream",
        )),
    );
    if mime.starts_with("text/html") {
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        headers.insert(
            axum::http::header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(PLUGIN_CSP),
        );
    } else {
        // 插件资源无哈希版本约束，统一短缓存（升级后 no-cache 的 HTML 会
        // 带新版本 query，子资源仍可命中，行为与移动端 assets/ 策略对齐）
        headers.insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("public, max-age=3600"),
        );
    }
    (StatusCode::OK, headers, bytes).into_response()
}

/// GET /plugins-shared/tokens.css —— 语义 token 全量（编译期内嵌，单一来源）
///
/// 耦合说明：tokens.css 源文件位于 frontend/src/styles/tokens.css，Vite 打包进主 CSS
/// 后 dist 中不可单独寻址；此处 include_str! 在编译期将其内嵌进二进制，插件模板
/// `<link href="/plugins-shared/tokens.css">` 获得与主窗口逐像素一致的设计 token。
async fn serve_tokens_css() -> Response {
    let body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../frontend/src/styles/tokens.css"
    ));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    (StatusCode::OK, headers, body).into_response()
}

/// GET /plugins-shared/base.css —— 插件共享基础样式（编译期内嵌：细滚动条等，对齐主应用）
async fn serve_plugin_base_css() -> Response {
    let body = include_str!("plugin_apps/base.css");
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    (StatusCode::OK, headers, body).into_response()
}

/// GET /plugins-shared/theme.css —— 当前生效主题快照（内存渲染，no-cache；
/// 主题切换后插件 iframe 重取即获得最新值）
async fn serve_theme_css<R: tauri::Runtime>(State(ctx): State<MobileCtx<R>>) -> Response {
    let state = ctx.app.state::<AppState>();
    let snap = state
        .theme_snapshot
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut body = format!("/* base: {} */\n:root {{\n", snap.base);
    for (k, v) in &snap.overrides {
        body.push_str(&format!("  {k}: {v};\n"));
    }
    body.push_str("}\n");
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    (StatusCode::OK, headers, body).into_response()
}

/// GET /plugins-shared/bridge.js —— 插件侧 Bridge SDK（宿主统一版本，插件免打包）
async fn serve_bridge_js() -> Response {
    let body = include_str!("plugin_apps/bridge.js");
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"),
    );
    (StatusCode::OK, headers, body).into_response()
}

/// POST /message 请求体——与 send_message_cmd 参数一一对应（references 结构复用）
#[derive(Debug, Deserialize)]
struct MobileMessage {
    message: String,
    images: Option<Vec<String>>,
    history: Option<Vec<crate::state::HistoryMessage>>,
    relation: Option<nuphus::agent::goal_types::RelationConfig>,
    mode: Option<String>,
    references: Option<Vec<crate::commands::process::ChatReference>>,
    send_id: Option<String>,
}

/// 手机消息入口：走共享入口 submit_user_message，source="mobile"。
/// busy 锁 / 消息去重 / session backup 全部复用共享入口内置逻辑，此处零重复实现。
async fn post_message<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileMessage>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    // 执行中（busy 锁占用）：手机消息不再 409 拒绝，转为追加指令。
    // 分两种情况：
    // 1) 暂停等待决策中（pause_flag=true）：agent 阻塞在 wait_for_pause_decision_global，
    //    不会进入下一轮迭代，mobile_append 队列不会被 drain——必须直接写 PauseDecision::Append，
    //    等价桌面 append_instruction，让暂停检查点立即返回 Append 并继续执行。
    // 2) 非暂停 busy：入 mobile_append 队列，react_loop 轮次边界 drain 注入下一迭代。
    if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
        if payload.message.trim().is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "Message cannot be empty" })),
            )
                .into_response();
        }
        // 追加通道不携带图片（PauseDecision::Append / mobile_append 均为纯文本），
        // 显式告知前端图片被丢弃——避免用户以为带图发送成功（审计 P3-1）
        let images_dropped = payload.images.as_ref().is_some_and(|imgs| !imgs.is_empty());
        // 去重防线：与最近受理内容相同且在 30s 内 → 丢弃（防刷新/重试导致的重复提交）
        let dup_vs_last = state
            .session
            .lock()
            .ok()
            .map(|guard| {
                guard.last_message == payload.message && state.elapsed_since_process_start() < 30
            })
            .unwrap_or(false);
        let paused = state.pause_flag.load(std::sync::atomic::Ordering::SeqCst);
        if paused {
            let action_id = nuphus::agent::pause::get_pause_action_id(&state.signals)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let instr = payload.message.clone();
            if dup_vs_last {
                tracing::info!(
                    "[Mobile] PAUSED append dedup（30s 内已受理）: {}",
                    payload.message.chars().take(60).collect::<String>()
                );
            } else {
                nuphus::agent::pause::set_pause_decision(
                    &state.signals,
                    &action_id,
                    nuphus::agent::pause::PauseDecision::Append(instr),
                );
            }
            tracing::info!(
                "[Mobile] PAUSED append: action_id={} instr={}",
                action_id,
                payload.message.chars().take(60).collect::<String>()
            );
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "append",
                    "message": payload.message,
                    "images_dropped": images_dropped
                })),
            )
                .into_response();
        }
        if !dup_vs_last {
            nuphus::mobile_append::push(payload.message.clone());
        } else {
            tracing::info!(
                "[Mobile] busy append dedup（30s 内已受理）: {}",
                payload.message.chars().take(60).collect::<String>()
            );
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "append",
                "message": payload.message,
                "images_dropped": images_dropped
            })),
        )
            .into_response();
    }
    // 竞态兜底备用：submit 会 move payload.message，Err 分支还需原文判断/入队
    let message_fallback = payload.message.clone();
    // 身份兜底：手机端无配置通道（localStorage 隔离），relation 为空时
    // 用桌面端最近一次传入的 relation_cache——保证手机端触发执行的身份与桌面端一致
    let relation = payload
        .relation
        .clone()
        .or_else(|| state.relation_cache.read().ok().and_then(|c| c.clone()));
    match crate::commands::process::submit_user_message(
        ctx.app.clone(),
        state.inner(),
        payload.message,
        payload.images,
        payload.history,
        relation,
        payload.mode,
        payload.references,
        payload.send_id,
        Some("mobile".to_string()),
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            let status = if e.contains("already running") {
                // 竞态兜底：busy 刚被占用（检查与提交之间被抢）→ 同样转追加指令
                if !message_fallback.trim().is_empty() {
                    let dup = state
                        .session
                        .lock()
                        .ok()
                        .map(|guard| {
                            guard.last_message == message_fallback
                                && state.elapsed_since_process_start() < 30
                        })
                        .unwrap_or(false);
                    if dup {
                        tracing::info!(
                            "[Mobile] race append dedup（30s 内已受理）: {}",
                            message_fallback.chars().take(60).collect::<String>()
                        );
                        return (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "status": "append",
                                "message": message_fallback
                            })),
                        )
                            .into_response();
                    } else {
                        let fb = message_fallback.clone();
                        nuphus::mobile_append::push(message_fallback);
                        return (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "status": "append",
                                "message": fb
                            })),
                        )
                            .into_response();
                    }
                }
                StatusCode::CONFLICT
            } else if e.contains("cannot be empty") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({ "error": e }))).into_response()
        }
    }
}

/// 提取客户端请求的 WS 子协议（auth.<token>），用于握手回显——浏览器 WebSocket 要求
/// 服务器必须从请求的 protocols 中回显一个，否则握手失败（close 1006）。
/// 移动端 ws.ts 在中继（公网）路径必带子协议 `auth.<token>`：该子协议只是中继
/// 三通道鉴权之一（Authorization / 子协议 / query token），隧道转发到本端点时 query
/// 鉴权已足够（token_valid 读 query 或 X-Mobile-Token），子协议纯为满足浏览器握手
/// 契约。不回显 → 公网手机 WS 全部 1006 断开（实测定位：refine 弹窗/实时消息失效）。
fn extract_ws_subprotocol(headers: &HeaderMap) -> Option<String> {
    headers
        .get("sec-websocket-protocol")?
        .to_str()
        .ok()?
        .split(',')
        .map(|p| p.trim())
        .find(|p| p.starts_with("auth."))
        .map(|p| p.to_string())
}

/// WS 事件推流：鉴权通过后 upgrade，把 broadcast channel 中的 NuphusEvent JSON
/// 实时转发给本连接。多手机客户端各自订阅同一 channel，互不影响。
async fn ws_handler<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let tx = ctx.ws_tx.clone();
    let engine = ctx.workflow_engine.clone();
    // 会话快照（连接即推送，替代手机端启动被动拉取）：握手前构造当前电脑端状态
    // 的**轻量**权威镜像——欢迎标志 + 执行状态 + 会话信息，**不含完整历史消息**。
    // ⚠️ 2026-08-26 实测：完整历史（数百 KB）经中继隧道 base64 在手机数据流量
    // 国际链路上大帧传输必失败 → WS 断开循环（「已连接→已断开」）。轻量状态帧
    // 稳定送达；历史内容由手机端经 HTTP 短连接拉取（归属头修复后多设备也能路由）。
    let snapshot: Option<String> = {
        let st = ctx.app.state::<AppState>();
        match crate::commands::process::session::chat_history(st.inner()) {
            Ok(messages) => Some(
                serde_json::json!({
                    "type": "session_snapshot",
                    "welcome": messages.is_empty(),
                    "running": st.busy.load(std::sync::atomic::Ordering::SeqCst),
                    "message_count": messages.len(),
                })
                .to_string(),
            ),
            Err(e) => {
                tracing::warn!("[Mobile] session snapshot build failed: {}", e);
                None
            }
        }
    };
    // 浏览器子协议鉴权：服务器必须回显所选协议，否则握手失败（close 1006）。
    // 移动端公网（中继）路径必带 auth.<token>，不回显则手机 WS 永久断开。
    let ws = if let Some(p) = extract_ws_subprotocol(&headers) {
        ws.protocols([p])
    } else {
        ws
    };
    ws.on_upgrade(move |socket| handle_ws(socket, tx, engine, snapshot))
}

async fn handle_ws(
    socket: WebSocket,
    tx: tokio::sync::broadcast::Sender<String>,
    engine: Arc<tokio::sync::RwLock<nuphus::workflow::WorkflowEngine>>,
    snapshot: Option<String>,
) {
    handle_ws_with_heartbeat(socket, tx, engine, snapshot, WS_HEARTBEAT_INTERVAL).await
}

/// 心跳间隔参数化仅供集成测试直接调小（非配置项，生产入口固定走上面的包装）
async fn handle_ws_with_heartbeat(
    socket: WebSocket,
    tx: tokio::sync::broadcast::Sender<String>,
    engine: Arc<tokio::sync::RwLock<nuphus::workflow::WorkflowEngine>>,
    snapshot: Option<String>,
    heartbeat_interval: std::time::Duration,
) {
    let mut rx = tx.subscribe();
    // 连接级 WorkflowEvent 订阅：EventBus 广播可多次 subscribe，每连接一份 receiver，
    // 连接断开 select 退出即回收（broadcast receiver Drop 自动退订）——零泄漏、无全局转发任务。
    let mut wf_rx = engine.read().await.event_bus().subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();
    // 就绪帧：订阅激活后立刻下发——客户端收到它即可确信后续事件不会漏
    // （broadcast 不为迟到订阅者补发历史，101 握手完成 ≠ 订阅已就位）
    if ws_tx
        .send(Message::Text(r#"{"type":"ws_connected"}"#.to_string()))
        .await
        .is_err()
    {
        return;
    }
    // 会话快照：连接即推送电脑端当前状态（欢迎界面/会话历史）的权威镜像——
    // 手机端收到直接呈现，无需再被动拉取（中继慢时拉取失败→重试循环的历史问题）。
    if let Some(snapshot) = snapshot {
        if ws_tx.send(Message::Text(snapshot)).await.is_err() {
            return;
        }
    }
    tracing::info!("[Mobile] WS client connected");
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.tick().await; // interval 首次立即 tick：跳过，避免与就绪帧争序
    loop {
        tokio::select! {
            // 事件 → 手机
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if ws_tx.send(Message::Text(text)).await.is_err() {
                            break; // 客户端断开
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // 丢事件不可静默继续：若被丢的是 execution_completed/paused，
                        // 客户端 running 永卡 true（后续发送全被分流为追加指令），
                        // 且心跳正常 → 僵尸看门狗不触发，无法自愈。
                        // 主动断开，走客户端既有「重连 → ws_connected → loadHistory
                        // + fetchAgentStatus 补齐」闭环（成本最低、路径已验证）。
                        tracing::warn!(
                            "[Mobile] WS client lagged, dropped {n} event(s) — closing to force resync"
                        );
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            // 工作流事件 → 手机：序列化拍平为 {"type":"workflow_event",...原 WorkflowEvent 字段}，
            // 与桌面 workflow-event payload 结构一致（useExecutionUI 解析字段完全对齐）。
            wf = wf_rx.recv() => {
                match wf {
                    Ok(event) => {
                        // internally tagged（tag="event"）→ {"event":"run_started",...}
                        let mut json = match serde_json::to_value(&event) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!("[Mobile] WorkflowEvent serialize failed: {e}");
                                break;
                            }
                        };
                        if let serde_json::Value::Object(map) = &mut json {
                            map.insert(
                                "type".to_string(),
                                serde_json::Value::String("workflow_event".to_string()),
                            );
                        }
                        let text = json.to_string();
                        if ws_tx.send(Message::Text(text)).await.is_err() {
                            break; // 客户端断开
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // WorkflowEvent 被丢弃不影响 NuphusEvent 主通道；
                        // 手机端从后续 run_started 事件重建步骤列表，无需断连强制重同步。
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            // 心跳 → 本连接（per-connection，绝不走 broadcast，否则 N 客户端各收 N 份）：
            // 切后台被系统静默挂起的假死连接 onclose 永不触发，靠 send 失败感知并回收
            _ = heartbeat.tick() => {
                if ws_tx
                    .send(Message::Text(r#"{"type":"heartbeat"}"#.to_string()))
                    .await
                    .is_err()
                {
                    break; // 连接假死/已断开 → 回收
                }
            }
            // 手机 → 服务端：当前仅用于检测断开（Close/错误/EOF），上行消息走 POST /message
            incoming = ws_rx.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // ping/pong 由 axum 自动应答，文本帧忽略
                    Some(Err(_)) => break,
                }
            }
        }
    }
    tracing::info!("[Mobile] WS client disconnected");
}

/// GET /identity — 手机端获取当前生效的身份显示名
/// （桌面端 soul 配置经 relation_cache 下发；手机端 localStorage 隔离拿不到配置）
async fn get_identity<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    Json(identity_json(state.inner())).into_response()
}

/// identity JSON 构建（get_identity / get_boot 共用）
fn identity_json(state: &AppState) -> serde_json::Value {
    let cache = state.relation_cache.read().ok().and_then(|c| c.clone());
    let (assistant_name, user_label) = match cache {
        Some(r) => (
            if r.assistant_name.is_empty() {
                "Nuphus".to_string()
            } else {
                r.assistant_name
            },
            if r.user_label.is_empty() {
                "用户".to_string()
            } else {
                r.user_label
            },
        ),
        None => ("Nuphus".to_string(), "用户".to_string()),
    };
    serde_json::json!({ "assistantName": assistant_name, "userLabel": user_label })
}

/// GET /custom-agents — 手机端获取 Custom Agent 卡片列表 + 当前激活卡片。
/// 用于手机端模式菜单显示 Custom 档（卡片名）；卡片的创建/编辑/切换在桌面端进行。
async fn get_custom_agents<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let agents: Vec<_> = nuphus::custom_agents::CustomAgentStore::list()
        .into_iter()
        .map(|c| serde_json::json!({ "id": c.id, "name": c.name }))
        .collect();
    let active = nuphus::custom_agents::CustomAgentStore::get_active()
        .map(|c| serde_json::json!({ "id": c.id, "name": c.name }));
    Json(serde_json::json!({ "agents": agents, "active": active })).into_response()
}

/// GET /agent-status — 手机端查询桌面端当前执行状态。
/// 用于刷新/重连后恢复 running：broadcast 事件不为迟到订阅者补发，刷新间隙的
/// execution_started/delta/completed 会丢失，故重连后以此恢复执行状态，保证
/// 后续 delta 能正常累积 streaming 气泡、execution_completed 正常落最终结果。
async fn get_agent_status<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    let busy = state.busy.load(std::sync::atomic::Ordering::SeqCst);
    Json(serde_json::json!({ "running": busy })).into_response()
}

/// GET /relay-hint — 手机端获取中继配置（外网模式下经中继发送消息）。
/// 仅当桌面 relay_client.json enabled=true 且配置完整时返回 enabled:true；
/// 返回 caller_token 供手机端外网 POST /task 鉴权（局域网内经 X-Mobile-Token 保护）。
async fn get_relay_hint<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let cfg = crate::relay_client::load_config();
    Json(relay_hint_json(&cfg, ctx.port)).into_response()
}

/// relay-hint JSON 构建（get_relay_hint / get_boot 共用）
fn relay_hint_json(cfg: &crate::relay_client::RelayClientConfig, port: u16) -> serde_json::Value {
    if cfg.enabled
        && !cfg.url.is_empty()
        && !cfg.device_id.is_empty()
        && !cfg.caller_token.is_empty()
    {
        // lan_url：桌面局域网直连地址。手机外网（中继）页面借此探测局域网可达性——
        // 回到同一 WiFi 后自动切回本地直连，中继仅在无局域网时兜底。
        let lan_url = primary_lan_ip().map(|ip| format!("http://{ip}:{port}"));
        serde_json::json!({
            "enabled": true,
            "url": cfg.url,
            "device_id": cfg.device_id,
            "caller_token": cfg.caller_token,
            "lan_url": lan_url,
            // 隧道公网入口（https://r.example.com 或 http://host:18081）：局域网 origin
            // 页面离开 WiFi 后页面内故障转移到中继用——非凭据，可缓存（P3-5 只禁 caller_token）。
            "tunnel_url": crate::relay_client::public_tunnel_url(cfg),
            "state": crate::relay_client::relay_conn_state(),
        })
    } else {
        serde_json::json!({
            "enabled": false,
            "state": crate::relay_client::relay_conn_state(),
        })
    }
}

/// GET /boot — 中继模式启动聚合端点：一次往返拿齐 identity + agent-status + relay-hint。
/// 动机（2026-08-17 实测）：中继国际链路 45% 丢包、TTFB 秒级抖动，启动期每个 HTTP
/// 往返都暴露一次重传风险；合并为 1 次显著降低首屏时间方差。
/// 仅中继（wan）路径使用——局域网延迟 <10ms，保持原分请求模式（合并不得影响
/// 局域网即时性）。history 不并入：体积大（全量对话+traceItems），独立拉取，
/// 不拖慢 boot 小载荷。
async fn get_boot<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    let running = state.busy.load(std::sync::atomic::Ordering::SeqCst);
    let cfg = crate::relay_client::load_config();
    Json(serde_json::json!({
        "identity": identity_json(state.inner()),
        "agentStatus": { "running": running },
        "relayHint": relay_hint_json(&cfg, ctx.port),
        // 会话清单投影（只读）：手机「会话」抽屉数据源——桌面当前视图的镜像，
        // 手机不维护独立会话状态（含 can_switch：busy/追加挂起时切换被禁）
        "sessions": crate::commands::process::shelf::list_shelf_sessions_inner(state.inner()).ok(),
    }))
    .into_response()
}

/// GET /sessions —— 桌面展示台会话清单投影（只读）。与 /boot.sessions 同源；
/// 局域网分请求模式单独拉取用。手机不维护独立会话状态，此为纯镜像端点。
async fn get_sessions<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::shelf::list_shelf_sessions_inner(state.inner()) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct SwitchSessionPayload {
    id: String,
}

/// POST /session/switch —— 手机遥控切换桌面当前会话。
/// 镜像模型：切的就是电脑端正显示的视图（桌面 rail 同步跟随），非移动端独立选态。
/// 成功后 switch_session_inner 已向 WS 广播 SessionChanged，双端各自刷新呈现。
/// 错误码为稳定字符串（busy / append_pending / mode_mismatch / not_found）→ 409 {"error": code}。
async fn post_switch_session<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<SwitchSessionPayload>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::shelf::switch_session_inner(state.inner(), payload.id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(code) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": code })),
        )
            .into_response(),
    }
}

/// POST /new-chat —— 手机遥控桌面新建对话（单一路径：复用桌面 new_chat_session_cmd
/// 同一权威入口，非移动端独立建会话）。成功后 new_chat_session_cmd 经 CompoundEmitter
/// 双推 SessionChanged（桌面 + 手机 WS），手机收到事件跟随显示欢迎页。
/// 错误码为稳定字符串（busy / append_pending）→ 409 {"error": code}。
async fn post_new_chat<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::shelf::new_chat_session_with_event(&ctx.app, state.inner()) {
        Ok(id) => Json(serde_json::json!({ "ok": true, "session_id": id })).into_response(),
        Err(e) => {
            let status = if e == "busy" || e == "append_pending" {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(serde_json::json!({ "error": e }))).into_response()
        }
    }
}

/// GET /model-config — 手机端读取桌面端模型配置（与桌面端 list_models / get_default_model 同源）
/// 返回：current（主模型）+ models（全部已配置模型 ModelInfo[]，不含 api_key / base_url）
async fn get_model_config<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    use nuphus::config::ModelRegistry;
    // 按当前 mode 解析生效模型需要读 agent_models 配置（providers.toml）
    let state = ctx.app.state::<AppState>();
    // 与 llm.rs list_models 一致：优先 config.toml，无文件回退环境变量
    let registry = if let Some(path) = crate::commands::config::get_config_path() {
        ModelRegistry::from_toml(path.to_str().unwrap_or("config.toml"))
    } else {
        ModelRegistry::from_env()
    };
    let (current, models, context_window) = match registry {
        Ok(registry) => {
            let mut models = Vec::new();
            let builtin = nuphus::config::registry::ProviderRegistry::builtin();
            for provider in &registry.providers {
                for model in &provider.models {
                    // Reasoning-effort 与桌面端 list_models 同一兜底链：
                    // 配置时持久化的 per-model 元数据 > builtin ModelDef > 空
                    let reasoning_efforts = if !model.reasoning_efforts.is_empty() {
                        model.reasoning_efforts.clone()
                    } else {
                        builtin
                            .find_model(&model.id)
                            .map(|(_, m)| {
                                m.reasoning_efforts.iter().map(|s| s.to_string()).collect()
                            })
                            .unwrap_or_default()
                    };
                    let default_effort = if model.default_effort.is_some() {
                        model.default_effort.clone()
                    } else {
                        builtin
                            .find_model(&model.id)
                            .and_then(|(_, m)| m.default_effort.map(|s| s.to_string()))
                    };
                    models.push(nuphus::api::ModelInfo {
                        id: model.id.clone(),
                        provider: provider.name.clone(),
                        alias: model.alias.clone(),
                        supports_streaming: model.supports_streaming,
                        supports_vision: model.supports_vision,
                        supports_audio: model.supports_audio,
                        supports_image_generation: model.supports_image_generation,
                        context_window: model.context_window.map(|c| c as u64).or_else(|| {
                            builtin
                                .find_model(&model.id)
                                .map(|(_, m)| m.context_window as u64)
                        }),
                        reasoning_efforts,
                        default_effort,
                    });
                }
            }
            // 按当前 mode 解析生效模型（agent_models：leader/workflow/custom 各自模型，
            // 空 = 跟随 default → leader → registry.model 锚点）。切换 mode 后模型卡
            // 「当前模型」应跟随该 mode 的生效模型，而非全局 registry.model。
            let current = crate::commands::config::llm::effective_model(
                &state.llm_config_path,
                &registry,
                query.mode.as_deref().unwrap_or("leader"),
            );
            // 当前模型上下文窗口（fallback 128000，与桌面端 contextLimit 默认一致）：
            // 手机端上下文用量百分比 = 会话累计 input_tokens / context_window
            let context_window = registry
                .providers
                .iter()
                .flat_map(|p| p.models.iter())
                .find(|m| m.id == current)
                .and_then(|m| m.context_window)
                .unwrap_or(128000);
            (current, models, context_window)
        }
        Err(_) => (String::new(), Vec::new(), 128000),
    };
    // 只回传模型标识与能力标记，绝不携带 api_key / base_url 等敏感字段
    Json(serde_json::json!({ "current": current, "models": models, "contextWindow": context_window }))
        .into_response()
}

/// POST /switch-model 请求体：切换当前模型（provider 必填；base_url/context_window 走桌面默认）
#[derive(Debug, Deserialize)]
struct MobileSwitchModel {
    model: String,
    provider: String,
    /// 当前 mode（leader/workflow/custom/global）——切换写入对应 agent 模型配置
    mode: Option<String>,
}

/// POST /switch-model — 手机端切换当前模型。
/// 对齐桌面 switch_model：provider-driven——API key 从 config.toml 读取，
/// 前端绝不传密钥；base_url 由 builtin ProviderRegistry 解析默认值。
async fn post_switch_model<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileSwitchModel>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.model.trim().is_empty() || payload.provider.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "model 与 provider 不能为空" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::config::llm::switch_model_impl(
        ctx.app.clone(),
        state,
        payload.model.clone(),
        payload.provider.clone(),
        None,
        None,
        payload.mode.clone(),
    )
    .await
    {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({ "ok": true, "message": msg })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": e })),
        )
            .into_response(),
    }
}

/// POST /switch-mode 请求体：切换运行模式（leader/workflow/custom）
#[derive(Debug, Deserialize)]
struct MobileSwitchMode {
    mode: String,
}

/// POST /switch-mode — 手机端切换运行模式。
/// 复用桌面 set_mode 命令：后端是 mode 的唯一权威源，切换后经 CompoundEmitter
/// 广播 ModeChanged 双端同步（手机自己 + 桌面端实时一致）。
async fn post_switch_mode<R: tauri::Runtime>(
    State(ctx): State<MobileCtx<R>>,
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<MobileSwitchMode>,
) -> Response {
    if !token_valid(&headers, &query, &ctx.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if payload.mode.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": "mode 不能为空" })),
        )
            .into_response();
    }
    let state = ctx.app.state::<AppState>();
    match crate::commands::process::mode::set_mode_impl(
        ctx.app.clone(),
        state,
        payload.mode.clone(),
    )
    .await
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "error": e })),
        )
            .into_response(),
    }
}

/// 跨域支持：外网手机页面（中继公网 origin）需跨域探测局域网直连 `/health`。
/// 白名单回显（审计 P2-3）：Origin 仅放行「中继隧道公网入口」与 localhost 开发源；
/// 其余跨源 Origin 不下发 ACAO——恶意网页无法经浏览器探测服务或对 /pair 发起
/// 远程爆破触发节流锁门 DoS。mobile_server 的 token 鉴权不依赖 CORS。
async fn cors_middleware(request: Request, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    if request.method() == Method::OPTIONS {
        let mut resp = Response::new(Body::empty());
        *resp.status_mut() = StatusCode::OK;
        add_cors_headers(resp.headers_mut(), origin.as_deref());
        return resp;
    }
    let mut response = next.run(request).await;
    add_cors_headers(response.headers_mut(), origin.as_deref());
    response
}

fn add_cors_headers(headers: &mut HeaderMap, origin: Option<&str>) {
    if let Some(o) = cors_allowed_origin(origin) {
        if let Ok(v) = HeaderValue::from_str(o) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
            // 回显具体 origin 时必须带 Vary，防共享缓存串 origin
            headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        // X-Tunnel-Device：中继多租户归属标记（api.ts 统一注入），局域网直连跨域预检需放行
        HeaderValue::from_static("Content-Type, X-Mobile-Token, X-Relay-Token, X-Tunnel-Device"),
    );
}

/// CORS Origin 白名单：中继隧道公网入口（外网手机页）+ localhost/127.0.0.1（开发）。
/// None/不匹配 → 不下发 ACAO（浏览器拦截跨源读取）。
fn cors_allowed_origin(origin: Option<&str>) -> Option<&str> {
    let o = origin?;
    // 中继公网入口（如 https://r.example.com 或 http://host:18081）——同 host 任意端口放行：
    // 手机可能经 https://r.example.com（443）、http://r.example.com:18081（隧道明文口）、
    // http://r.example.com:18080（HTTP API）任一入口进入，页面内 fetch 局域网 /health 均需跨域。
    // 攻击者无法在 r.example.com 域名下开任意端口服务（除非控制中继服务器），且 mobile_server
    // 全端点有 token 鉴权兜底——同 host 放行不扩大实际攻击面。
    let cfg = crate::relay_client::load_config();
    let relay_hosts: Vec<String> = {
        let mut hosts = Vec::new();
        for url in [&cfg.public_url, &cfg.url] {
            if let Some(h) = origin_host_of(url) {
                if !hosts.contains(&h) {
                    hosts.push(h);
                }
            }
        }
        hosts
    };
    if !relay_hosts.is_empty() {
        if let Some(h) = origin_host_of(o) {
            if relay_hosts.iter().any(|rh| rh == &h) {
                return Some(o);
            }
        }
    }
    // 本地开发源
    if o.starts_with("http://localhost") || o.starts_with("http://127.0.0.1") {
        return Some(o);
    }
    None
}

/// 从 URL 提取 host（含 IPv6 字面量 [] 段；无 scheme 时按裸 host 处理）。失败返回 None。
fn origin_host_of(url: &str) -> Option<String> {
    // 注意：不能 trim_end_matches('/')——会把 "://" 破坏成 ":"（split_once 失配），
    // 且路径斜杠本就被下方 split('/') 截断，顶层去斜杠是多余且有害的。
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let rest = trimmed.split_once("://").map(|(_, r)| r).unwrap_or(trimmed);
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    if let Some(end) = authority.strip_prefix('[').and_then(|a| a.find(']')) {
        // end 是相对去掉 '[' 后的子串的索引，需要 +1 才包含 ']' 本身
        Some(authority[..=end + 1].to_string())
    } else {
        Some(authority.split(':').next()?.to_string())
    }
}

/// 构建路由（提取为独立函数：start_server 与集成测试共用同一路由定义）
fn create_router<R: tauri::Runtime>(ctx: MobileCtx<R>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/pair", post(post_pair))
        .route("/history", get(get_history))
        .route("/identity", get(get_identity))
        .route("/custom-agents", get(get_custom_agents))
        .route("/agent-status", get(get_agent_status))
        .route("/relay-hint", get(get_relay_hint))
        .route("/boot", get(get_boot))
        .route("/sessions", get(get_sessions))
        .route("/session/switch", post(post_switch_session))
        .route("/new-chat", post(post_new_chat))
        .route("/model-config", get(get_model_config))
        .route("/switch-model", post(post_switch_model))
        .route("/switch-mode", post(post_switch_mode))
        .route("/message", post(post_message))
        .route("/confirm", post(post_confirm))
        .route("/user-input", post(post_user_input))
        .route("/user-input-reject", post(post_user_input_reject))
        .route("/rating", post(post_rating))
        .route("/refine", post(post_refine))
        .route("/refine-skip", post(post_refine_skip))
        .route("/pause", post(post_pause))
        .route("/resume", post(post_resume))
        .route("/terminate", post(post_terminate))
        .route("/stop", post(post_stop))
        .route("/workflow-pause", post(post_workflow_pause))
        .route("/workflow-resume", post(post_workflow_resume))
        .route("/workflow-stop", post(post_workflow_stop))
        .route("/ws", get(ws_handler))
        // 静态资源不再压缩（2026-08-26 白屏根因修复）：压缩层把资源响应变成
        // Transfer-Encoding: chunked（无 Content-Length），中继隧道对 chunked 无法
        // 精确判定响应完成——旧逻辑头后 body 滞留 pending，30s 空闲回收 abort 丢失
        // 尾部 → 大文件随机截断 → 公网手机 JS 加载失败 → 白屏（实测 73787/49688 字节）。
        // 去掉压缩后 axum bytes → Body::Full 自动带 Content-Length，中继逐响应精确
        // 关闭；资源体积 JS 102KB 直传（局域网毫秒级，公网经 base64 隧道后约 137KB
        // 仍可接受），正确性优先于带宽优化。
        // 只挂资源路由：WS upgrade / 小 JSON API 不过压缩层（无谓 CPU 开销）；
        // axum 0.7 的 .layer() 烘焙进路由服务，merge 后压缩层仍只作用于资源路由
        // （含应用插件 /plugins/* 与 /plugins-shared/* 静态伺服）。
        .merge(
            Router::new()
                .route("/", get(serve_index))
                // 移动端 SPA 兜底改 fallback：matchit 0.7 通配优先级会遮蔽
                // /plugins/*rest 插件路由（实测 /plugins/{id}/ 被 /*path 吃掉），
                // fallback 语义等价且不遮蔽显式路由。
                // 应用插件伺服（设计文档 §4.1；与移动端静态资源同层，不压缩）
                .route("/plugins/*rest", get(serve_plugin))
                .route("/plugins-shared/tokens.css", get(serve_tokens_css))
                .route("/plugins-shared/base.css", get(serve_plugin_base_css))
                .route("/plugins-shared/theme.css", get(serve_theme_css))
                .route("/plugins-shared/bridge.js", get(serve_bridge_js))
                .fallback(serve_asset_fallback),
        )
        .with_state(ctx)
        .layer(middleware::from_fn(cors_middleware))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE_BYTES))
}

// ============================================================================
// 生命周期
// ============================================================================

/// 启动 server（幂等：已运行则直接返回当前状态）。
/// 绑 0.0.0.0:{port}；端口占用时退化绑 0（OS 分配）并返回实际端口。
/// 成功后 AppState.mobile_ws_tx = Some → CompoundEmitter 开始双推。
pub async fn start_server<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    port: u16,
) -> Result<MobileServerStatus, String> {
    // 幂等：已在运行 → 返回现状
    if state
        .mobile_server_shutdown
        .lock()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(current_status(state));
    }

    // 确保 token 存在（首次启用时生成并持久化）
    let token = {
        let mut guard = state.mobile_token.write().map_err(|e| e.to_string())?;
        if guard.is_empty() {
            *guard = generate_token();
            let mut cfg = load_config();
            cfg.token = guard.clone();
            save_config(&cfg)?;
            tracing::info!("[Mobile] 首次启用，已生成访问 token");
        }
        guard.clone()
    };

    let (ws_tx, _) = tokio::sync::broadcast::channel(WS_BROADCAST_CAPACITY);

    // 绑 0.0.0.0（局域网可达）；端口占用 → 退化 OS 分配（对齐 handoff 优雅降级模式）
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[Mobile] 端口 {port} 绑定失败（{e}），退化为 OS 分配端口");
            tokio::net::TcpListener::bind(("0.0.0.0", 0))
                .await
                .map_err(|e2| format!("mobile server 启动失败: {e2}"))?
        }
    };
    let actual_port = listener
        .local_addr()
        .map_err(|e| format!("读取监听地址失败: {e}"))?
        .port();

    // ctx 需要 actual_port（/relay-hint 下发 lan_url 用）——先 bind 再构造
    let ctx = MobileCtx {
        app: app.clone(),
        token: state.mobile_token.clone(),
        ws_tx: ws_tx.clone(),
        port: actual_port,
        pair_throttle: Arc::new(std::sync::Mutex::new(PairThrottle::default())),
        workflow_engine: state.workflow_engine.clone(),
    };
    let router = create_router(ctx);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // 状态先落位，再 spawn serve——保证 serve 启动期间 CompoundEmitter 已可见 broadcaster
    *state.mobile_ws_tx.lock().map_err(|e| e.to_string())? = Some(ws_tx);
    *state
        .mobile_server_shutdown
        .lock()
        .map_err(|e| e.to_string())? = Some(shutdown_tx);

    tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::warn!("[Mobile] server 异常退出: {e}");
        }
        tracing::info!("[Mobile] server 已停止");
    });

    let lan_url = primary_lan_ip().map(|ip| format!("http://{ip}:{actual_port}"));
    tracing::info!(
        "[Mobile] server 监听 0.0.0.0:{actual_port}{}",
        lan_url
            .as_deref()
            .map(|u| format!("（局域网入口 {u}）"))
            .unwrap_or_default()
    );

    Ok(MobileServerStatus {
        running: true,
        port: actual_port,
        token,
        lan_url,
        password_set: !load_config().password_hash.is_empty(),
    })
}

/// 停止 server（幂等）。CompoundEmitter 侧随 mobile_ws_tx=None 退化为纯 Tauri。
pub fn stop_server(state: &AppState) -> Result<(), String> {
    let shutdown = state
        .mobile_server_shutdown
        .lock()
        .map_err(|e| e.to_string())?
        .take();
    if let Some(tx) = shutdown {
        let _ = tx.send(());
    }
    *state.mobile_ws_tx.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}

/// 当前状态（running = shutdown handle 存在）
pub fn current_status(state: &AppState) -> MobileServerStatus {
    let running = state
        .mobile_server_shutdown
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    let token = state
        .mobile_token
        .read()
        .map(|g| g.clone())
        .unwrap_or_default();
    let cfg = load_config();
    let port = cfg.port;
    let lan_url = if running {
        primary_lan_ip().map(|ip| format!("http://{ip}:{port}"))
    } else {
        None
    };
    MobileServerStatus {
        running,
        port,
        token,
        lan_url,
        password_set: !cfg.password_hash.is_empty(),
    }
}

/// 主网卡局域网 IP（UDP connect 技巧，不产生实际流量）
fn primary_lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_ipv4() && !ip.is_loopback() {
        Some(ip.to_string())
    } else {
        None
    }
}

// ============================================================================
// Tauri commands（P3 设置页调用）
// ============================================================================

/// 启动移动端 server（可选指定端口；缺省用持久化配置）。持久化 enabled=true。
#[tauri::command]
pub async fn mobile_server_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    port: Option<u16>,
) -> Result<MobileServerStatus, String> {
    let cfg = load_config();
    let port = port.unwrap_or(cfg.port);
    let status = start_server(&app, state.inner(), port).await?;
    let mut cfg = load_config();
    cfg.enabled = true;
    cfg.port = status.port;
    cfg.token = status.token.clone();
    save_config(&cfg)?;
    Ok(status)
}

/// 停止移动端 server。持久化 enabled=false（重启后不再自动启动）。
#[tauri::command]
pub async fn mobile_server_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    stop_server(state.inner())?;
    let mut cfg = load_config();
    cfg.enabled = false;
    save_config(&cfg)?;
    Ok(())
}

/// 确保 server 运行（插件宿主等内部消费者专用）：幂等启动但**不持久化 enabled**——
/// 用户的移动端开关设置不被改变，重启后仍按原设置决定是否自启。
/// 与 mobile_server_start 的唯一差别就是不写配置（大王 2026-08-16 决策：
/// 打开插件不应偷偷打开移动端）。
#[tauri::command]
pub async fn mobile_server_ensure(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<MobileServerStatus, String> {
    let cfg = load_config();
    start_server(&app, state.inner(), cfg.port).await
}

/// 查询 server 状态（含 token 与局域网 URL，供 P3 设置页展示/生成二维码）
#[tauri::command]
pub fn mobile_server_status(
    state: tauri::State<'_, AppState>,
) -> Result<MobileServerStatus, String> {
    Ok(current_status(state.inner()))
}

/// 重新生成 token 并持久化；运行中的 server 即刻用新 token 鉴权（旧 token 失效）。
#[tauri::command]
pub fn mobile_token_regenerate(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let new_token = generate_token();
    *state.mobile_token.write().map_err(|e| e.to_string())? = new_token.clone();
    let mut cfg = load_config();
    cfg.token = new_token.clone();
    save_config(&cfg)?;
    tracing::info!("[Mobile] token 已重新生成");
    Ok(new_token)
}

/// 设置/重置配对密码：校验强度 → 存 salt:hash → 新设置/改密时重签 token
/// （旧 token 立即失效，已配对手机下次鉴权 401，被迫重新输密码）。
/// 空字符串同样走 validate 分支报"至少 6 位"，未提供"清除密码"入口——
/// 配对密码一旦设置持续生效（与桌面"必须设置密码"一致）。
///
/// 幂等保护（2026-08-25 实测事故）：同一密码重复保存**不重签 token**——
/// 设置页「保存」可能被随手点击，无条件轮换会立即踢掉所有已配对手机
/// （401 循环，用户侧表现为反复「配对失效」）。salt 哈希不可直接比较，
/// 用 verify_password 对照存量哈希判定同一密码。
#[tauri::command]
pub fn mobile_password_set(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<(), String> {
    validate_password(&password)?;
    let new_hash = hash_password(&password);
    let mut cfg = load_config();
    if !cfg.password_hash.is_empty()
        && !cfg.token.is_empty()
        && verify_password(&password, &cfg.password_hash)
    {
        tracing::info!("[Mobile] 配对密码未变化，token 保持不变");
        return Ok(());
    }
    let new_token = generate_token();
    // 先更新运行态 token，再落盘——与 mobile_token_regenerate 顺序一致
    *state.mobile_token.write().map_err(|e| e.to_string())? = new_token.clone();
    cfg.password_hash = new_hash;
    cfg.token = new_token.clone();
    save_config(&cfg)?;
    tracing::info!("[Mobile] 配对密码已设置，token 已重签");
    Ok(())
}

// ============================================================================
// 集成测试（模式对齐 handoff_server.rs：真实 axum server + reqwest 断言）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_html_assets_rewrites_relative_refs_with_device_prefix() {
        let html = r#"<link rel="icon" href="./icons/icon-192.png"><script type="module" src="./assets/mobile.html-D7XNzb5s.js"></script><script src="./load-guard.js"></script>"#;
        let marked = mark_html_assets(html, "desktop-9f2c");
        assert!(marked.contains(r#"href="/d/desktop-9f2c/icons/icon-192.png""#));
        assert!(marked.contains(r#"src="/d/desktop-9f2c/assets/mobile.html-D7XNzb5s.js""#));
        assert!(marked.contains(r#"src="/d/desktop-9f2c/load-guard.js""#));
        // 无残留相对引用
        assert!(!marked.contains(r#""./"#));
    }

    #[test]
    fn mark_html_assets_noop_without_device_id() {
        let html = r#"<script src="./assets/a.js"></script>"#;
        assert_eq!(mark_html_assets(html, ""), html);
    }

    /// 最小 WS 客户端：握手 + 读一个文本帧（server→client 帧不带 mask，解析从简）。
    /// 不引 tokio-tungstenite——测试只需要「连得上、收得到 JSON」的最小能力。
    mod ws_client {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        /// 带帧缓冲的连接：101 响应与首个 WS 帧可能同一 TCP 段到达，握手后剩余
        /// 已读字节必须留存——直接丢弃会让 ws_connected 就绪帧被静默吃掉（竞态）
        pub struct Conn {
            stream: TcpStream,
            buf: std::collections::VecDeque<u8>,
        }

        pub async fn connect(port: u16, path: &str) -> Result<(Conn, String), String> {
            let mut stream = TcpStream::connect(("127.0.0.1", port))
                .await
                .map_err(|e| format!("connect: {e}"))?;
            let key = base64_encode(b"nuphus-test-key!");
            let req = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            );
            stream
                .write_all(req.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("connection closed during handshake".into());
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&buf[..pos]).to_string();
                    let leftover = buf[pos + 4..].to_vec();
                    return Ok((
                        Conn {
                            stream,
                            buf: leftover.into(),
                        },
                        headers,
                    ));
                }
                if buf.len() > 8192 {
                    return Err("handshake response too large".into());
                }
            }
        }

        impl Conn {
            /// 读一个完整文本帧（server 帧无 mask；支持 7/16/64 位长度；跳过非文本帧）
            pub async fn read_text(&mut self) -> Result<String, String> {
                loop {
                    let hdr = self.read_bytes(2).await?;
                    let opcode = hdr[0] & 0x0f;
                    let masked = hdr[1] & 0x80 != 0;
                    let mut len = (hdr[1] & 0x7f) as u64;
                    if len == 126 {
                        let ext = self.read_bytes(2).await?;
                        len = u16::from_be_bytes([ext[0], ext[1]]) as u64;
                    } else if len == 127 {
                        let ext = self.read_bytes(8).await?;
                        len = u64::from_be_bytes(ext.try_into().unwrap());
                    }
                    let mask_key = if masked {
                        Some(self.read_bytes(4).await?)
                    } else {
                        None
                    };
                    let mut payload = self.read_bytes(len as usize).await?;
                    if let Some(k) = mask_key {
                        for (i, b) in payload.iter_mut().enumerate() {
                            *b ^= k[i % 4];
                        }
                    }
                    match opcode {
                        0x1 => return String::from_utf8(payload).map_err(|e| e.to_string()),
                        0x8 => return Err("received close frame".into()),
                        _ => continue, // ping/pong/binary 跳过
                    }
                }
            }

            /// 精确读 n 字节：优先消费握手遗留缓冲，不足再从 socket 补读
            async fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>, String> {
                let mut out = Vec::with_capacity(n);
                while out.len() < n {
                    if self.buf.is_empty() {
                        let mut chunk = [0u8; 4096];
                        let m = self
                            .stream
                            .read(&mut chunk)
                            .await
                            .map_err(|e| e.to_string())?;
                        if m == 0 {
                            return Err("connection closed".into());
                        }
                        self.buf.extend(&chunk[..m]);
                    }
                    let take = (n - out.len()).min(self.buf.len());
                    out.extend(self.buf.drain(..take));
                }
                Ok(out)
            }
        }

        fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
            hay.windows(needle.len()).position(|w| w == needle)
        }

        fn base64_encode(data: &[u8]) -> String {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for chunk in data.chunks(3) {
                let b = [
                    chunk[0],
                    *chunk.get(1).unwrap_or(&0),
                    *chunk.get(2).unwrap_or(&0),
                ];
                let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
                out.push(T[(n >> 18) as usize & 63] as char);
                out.push(T[(n >> 12) as usize & 63] as char);
                out.push(if chunk.len() > 1 {
                    T[(n >> 6) as usize & 63] as char
                } else {
                    '='
                });
                out.push(if chunk.len() > 2 {
                    T[n as usize & 63] as char
                } else {
                    '='
                });
            }
            out
        }
    }

    /// 构造测试用 AppState + mock AppHandle，并在 ephemeral 端口启动真实 server。
    /// 返回 (base_url, token, app)——app 持有以保 AppState 生命周期。
    async fn spawn_test_server() -> (String, String, tauri::AppHandle<tauri::test::MockRuntime>) {
        let app = tauri::test::mock_app();
        // 让 tracing（含 serve 错误路径）在测试中可见；多次 try_init 安全
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_test_writer()
            .try_init();
        let handle = app.handle().clone();
        handle.manage(AppState::default());
        let state = handle.state::<AppState>();

        // 预设 token（不依赖磁盘配置）
        *state.mobile_token.write().unwrap() = "test-token-0123456789abcdef".to_string();

        // 绑 ephemeral 端口：start_server 传 0（生产默认 DEFAULT_PORT，由命令层传入）
        let status = start_server(&handle, state.inner(), 0)
            .await
            .expect("start_server 应成功");
        assert!(status.running);
        let base = format!("http://127.0.0.1:{}", status.port);
        // 就绪探针：确认 server 真实进入 accept 状态
        let mut up = false;
        for _ in 0..50 {
            if let Ok(r) = np_get(&format!("{base}/health")).await {
                if r.status() == 200 {
                    up = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(up, "server 启动后 1s 内未进入可用状态");
        (base, status.token, handle)
    }

    /// 无代理 client：本机系统代理（如 Clash@2081）会随机拦截 loopback 请求导致
    /// 10053/10054 假失败——测试目标始终是本进程内 server，必须绕过系统代理。
    fn np_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("reqwest client")
    }

    async fn np_get(url: &str) -> Result<reqwest::Response, reqwest::Error> {
        np_client().get(url).send().await
    }

    /// 配置类测试串行化：并行测试同时改写同一 mobile_server.json 会互相覆盖，
    /// 必须保证「写配置」的测试互斥执行（其余测试只读，不参与竞争）。
    static CONFIG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 临时改写 mobile_server 配置（仅 /pair 测试用）；Drop 时恢复原文件
    /// （或删除本次新建的文件），保证不污染真实用户配置。
    struct TestConfigGuard {
        path: std::path::PathBuf,
        original: Option<String>,
    }

    impl TestConfigGuard {
        /// password=None 表示置空（未设置密码）；Some(hash) 写入给定哈希。
        fn set_password_hash(hash: Option<&str>) -> Self {
            let path = config_path();
            let original = std::fs::read_to_string(&path).ok();
            let mut cfg = load_config();
            cfg.password_hash = hash.unwrap_or("").to_string();
            let _ = save_config_to(&path, &cfg);
            Self { path, original }
        }
    }

    impl Drop for TestConfigGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(content) => {
                    let _ = std::fs::write(&self.path, content);
                }
                None => {
                    let _ = std::fs::remove_file(&self.path);
                }
            }
        }
    }

    #[test]
    // CONFIG_LOCK 串行化写配置测试：锁须跨 await 持有整个测试期（见 2264 注释），
    // 有意设计，非意外持有——allow await_holding_lock。
    #[allow(clippy::await_holding_lock)]
    fn test_pair_no_password_configured() {
        // 桌面端未设置配对密码 → /pair 一律 503
        tokio_test::block_on(async {
            let _lock = CONFIG_LOCK.lock().unwrap();
            let _guard = TestConfigGuard::set_password_hash(None);
            let (base, _token, _app) = spawn_test_server().await;
            let resp = np_client()
                .post(format!("{base}/pair"))
                .json(&serde_json::json!({ "password": "abc123" }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 503);
            let body: serde_json::Value = resp.json().await.unwrap();
            assert!(body["error"].as_str().unwrap().contains("未设置配对密码"));
        });
    }

    #[test]
    // CONFIG_LOCK 串行化写配置测试：锁须跨 await 持有整个测试期（见 2264 注释），
    // 有意设计，非意外持有——allow await_holding_lock。
    #[allow(clippy::await_holding_lock)]
    fn test_pair_endpoint() {
        // 完整配对流：200 签发 token → 401 密码错误 → 连续 5 次失败锁 60s → 429
        tokio_test::block_on(async {
            let _lock = CONFIG_LOCK.lock().unwrap();
            let _guard = TestConfigGuard::set_password_hash(Some(&hash_password("abc123")));
            let (base, token, _app) = spawn_test_server().await;
            let client = np_client();

            // ① 正确密码 → 200 + 当前 token
            let resp = client
                .post(format!("{base}/pair"))
                .json(&serde_json::json!({ "password": "abc123" }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200);
            let body: serde_json::Value = resp.json().await.unwrap();
            assert_eq!(body["token"], token, "配对成功应返回当前 token");

            // ② 错误密码 → 401（failures=1）
            let resp = client
                .post(format!("{base}/pair"))
                .json(&serde_json::json!({ "password": "wrong1" }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 401);
            let body: serde_json::Value = resp.json().await.unwrap();
            assert!(body["error"].as_str().unwrap().contains("密码错误"));

            // ③ 再错 4 次（累计 5 次）→ 触发锁定，第 5 次本身仍是 401
            for i in 0..4 {
                let resp = client
                    .post(format!("{base}/pair"))
                    .json(&serde_json::json!({ "password": "wrong2" }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 401, "第 {} 次失败仍应 401", i + 2);
            }

            // ④ 锁定后即使密码正确也 429，且提示剩余秒数
            let resp = client
                .post(format!("{base}/pair"))
                .json(&serde_json::json!({ "password": "abc123" }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 429);
            let body: serde_json::Value = resp.json().await.unwrap();
            let msg = body["error"].as_str().unwrap();
            assert!(
                msg.contains("秒后重试"),
                "锁定提示应含剩余秒数，实际: {msg}"
            );

            // ⑤ 其他鉴权端点不受影响：/pair 不消耗 token，history 带 token 仍 200
            let resp = np_get(&format!("{base}/history?token={token}"))
                .await
                .unwrap();
            assert_eq!(resp.status(), 200, "现有 token 鉴权路径不应被 /pair 影响");
        });
    }

    #[test]
    fn test_health_no_auth() {
        tokio_test::block_on(async {
            let (base, _token, _app) = spawn_test_server().await;
            let resp = np_get(&format!("{base}/health")).await.unwrap();
            assert_eq!(resp.status(), 200);
            let body: serde_json::Value = resp.json().await.unwrap();
            assert_eq!(body["service"], "nuphus-mobile");
        });
    }

    #[test]
    fn test_auth_required() {
        tokio_test::block_on(async {
            let (base, token, _app) = spawn_test_server().await;
            let client = np_client();
            let payload = serde_json::json!({ "message": "hi" });

            // 无 token → 401
            let r = client
                .post(format!("{base}/message"))
                .json(&payload)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401);
            // 错误 token → 401
            let r = client
                .post(format!("{base}/message"))
                .header("X-Mobile-Token", "wrong")
                .json(&payload)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401);
            // WS 无 token → 握手被拒（非 101）
            let (_s, headers) = ws_client::connect(
                base.trim_start_matches("http://127.0.0.1:")
                    .parse()
                    .unwrap(),
                "/ws",
            )
            .await
            .unwrap();
            assert!(
                headers.contains("401"),
                "WS 无 token 应 401，实际: {headers}"
            );
            // WS 错误 token → 401
            let (_s, headers) = ws_client::connect(
                base.trim_start_matches("http://127.0.0.1:")
                    .parse()
                    .unwrap(),
                "/ws?token=wrong",
            )
            .await
            .unwrap();
            assert!(
                headers.contains("401"),
                "WS 错误 token 应 401，实际: {headers}"
            );
            // query token 渠道与 header 等效（正确 token 的 WS 升级在 ws_receives_events 验证）
            let _ = token;
        });
    }

    #[test]
    fn test_message_reaches_shared_entry_busy_lock() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            // 预占 busy 锁——submit_user_message 内置并发控制应立即拒绝。
            // 命中「Task is already running」即证明 /message 真实进入共享入口的 busy 层。
            let state = app.state::<AppState>();
            state.busy.store(true, std::sync::atomic::Ordering::SeqCst);

            let client = np_client();
            let r = client
                .post(format!("{base}/message"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "message": "来自手机的消息" }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "busy 时应转追加指令而非 409");
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "append", "应标记为追加指令，实际: {body}");
            assert!(nuphus::mobile_append::has_pending(), "消息应进入追加队列");
            // 清理追加队列，避免滞留影响并行测试
            let _ = nuphus::mobile_append::drain_for_injection();

            // 空消息 → 400（共享入口的空消息校验层）
            state.busy.store(false, std::sync::atomic::Ordering::SeqCst);
            let r = client
                .post(format!("{base}/message"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "message": "   " }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);
        });
    }

    #[test]
    fn test_control_endpoints_auth() {
        tokio_test::block_on(async {
            let (base, _token, _app) = spawn_test_server().await;
            let client = np_client();

            // 无 token → 全部 401（body 合法与否都应在鉴权层被拒）
            for path in ["/pause", "/resume", "/terminate", "/stop"] {
                let r = client
                    .post(format!("{base}{path}"))
                    .json(&serde_json::json!({ "action_id": "abc" }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), 401, "{path} 无 token 应 401");
            }
            // 错误 token → 401
            let r = client
                .post(format!("{base}/pause"))
                .header("X-Mobile-Token", "wrong")
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401);
        });
    }

    #[test]
    fn test_pause_resume_terminate_flow() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            let client = np_client();

            // /pause 无 body → 200 {status:"paused", action_id}，pause_flag 置位
            let r = client
                .post(format!("{base}/pause"))
                .header("X-Mobile-Token", &token)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "paused");
            let action_id = body["action_id"].as_str().unwrap().to_string();
            assert!(!action_id.is_empty(), "pause 应返回非空 action_id");

            let state = app.state::<AppState>();
            assert!(
                state.pause_flag.load(std::sync::atomic::Ordering::SeqCst),
                "/pause 后 pause_flag 应为 true"
            );
            assert_eq!(
                nuphus::agent::pause::get_pause_action_id(&state.signals).as_deref(),
                Some(action_id.as_str())
            );

            // /resume 带 action_id → 200，决策预置 Continue（复用桌面命令）
            let r = client
                .post(format!("{base}/resume"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "action_id": action_id }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "resumed");
            assert_eq!(
                nuphus::agent::pause::peek_pause_decision(&state.signals, &action_id),
                Some(nuphus::agent::pause::PauseDecision::Continue)
            );

            // /terminate 带 action_id → 200，决策预置 Terminate（复用桌面命令）
            let r = client
                .post(format!("{base}/terminate"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "action_id": action_id }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "terminated");
            assert_eq!(
                nuphus::agent::pause::peek_pause_decision(&state.signals, &action_id),
                Some(nuphus::agent::pause::PauseDecision::Terminate)
            );

            // 空 action_id → 400
            for path in ["/resume", "/terminate"] {
                let r = client
                    .post(format!("{base}{path}"))
                    .header("X-Mobile-Token", &token)
                    .json(&serde_json::json!({ "action_id": "   " }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), 400, "{path} 空 action_id 应 400");
            }
        });
    }

    #[test]
    fn test_stop_graceful() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            let client = np_client();

            // /stop 无 body → 200 {status:"stopping", action_id}，pause_flag + Terminate 预置
            let r = client
                .post(format!("{base}/stop"))
                .header("X-Mobile-Token", &token)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "stopping");
            let action_id = body["action_id"].as_str().unwrap().to_string();
            assert!(!action_id.is_empty(), "stop 应返回非空 action_id");

            let state = app.state::<AppState>();
            assert!(
                state.pause_flag.load(std::sync::atomic::Ordering::SeqCst),
                "/stop 后 pause_flag 应为 true"
            );
            assert_eq!(
                nuphus::agent::pause::peek_pause_decision(&state.signals, &action_id),
                Some(nuphus::agent::pause::PauseDecision::Terminate)
            );
        });
    }

    #[test]
    fn test_ws_receives_compound_emitter_events() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            let port: u16 = base
                .trim_start_matches("http://127.0.0.1:")
                .parse()
                .unwrap();

            // WS 客户端带正确 token 连接 → 握手应 101
            let (mut ws, headers) = ws_client::connect(port, &format!("/ws?token={token}"))
                .await
                .unwrap();
            assert!(
                headers.contains("101"),
                "正确 token 应完成升级，实际: {headers}"
            );

            // 订阅就绪帧（消除 101 与 subscribe() 之间的竞态：收到它才保证后续事件不漏）
            let hello = tokio::time::timeout(std::time::Duration::from_secs(5), ws.read_text())
                .await
                .expect("5s 内应收到就绪帧")
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&hello).unwrap()["type"],
                "ws_connected"
            );

            // 经 CompoundEmitter（从 AppState 构造，server 运行中 → 双推）发射事件
            let state = app.state::<AppState>();
            let emitter = crate::emitter::CompoundEmitter::new(app.clone(), state.inner());
            nuphus::agent::events::EventEmitter::emit(
                &emitter,
                nuphus::agent::events::NuphusEvent::UserMessageReceived {
                    content: "手机端测试消息".to_string(),
                    source: "mobile".to_string(),
                    images: vec![],
                },
            );

            // WS 客户端应收到与桌面相同的 NuphusEvent JSON（跳过连接即发的 snapshot 帧）
            let text = loop {
                let text = tokio::time::timeout(std::time::Duration::from_secs(5), ws.read_text())
                    .await
                    .expect("5s 内应收到事件")
                    .unwrap();
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                if v["type"] != "session_snapshot" {
                    break text;
                }
            };
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(json["type"], "user_message_received");
            assert_eq!(json["content"], "手机端测试消息");
            assert_eq!(json["source"], "mobile");
        });
    }

    #[test]
    fn test_ws_heartbeat_per_connection() {
        tokio_test::block_on(async {
            // 心跳间隔直接调小（生产 15s 由包装函数固定，此处非配置项、仅测试可达）
            let (bcast_tx, _) = tokio::sync::broadcast::channel::<String>(WS_BROADCAST_CAPACITY);
            let tx = bcast_tx.clone();
            let engine = Arc::new(tokio::sync::RwLock::new(
                nuphus::workflow::WorkflowEngine::new(),
            ));
            let app = Router::new().route(
                "/ws",
                get(move |ws: WebSocketUpgrade| {
                    let tx = tx.clone();
                    let engine = engine.clone();
                    async move {
                        ws.on_upgrade(move |socket| {
                            handle_ws_with_heartbeat(
                                socket,
                                tx,
                                engine,
                                None, // snapshot：测试不需要
                                std::time::Duration::from_millis(100),
                            )
                        })
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            // 两个客户端同连：心跳 per-connection 直发，互不多收
            let (mut ws1, h1) = ws_client::connect(port, "/ws").await.unwrap();
            let (mut ws2, h2) = ws_client::connect(port, "/ws").await.unwrap();
            assert!(h1.contains("101") && h2.contains("101"));
            for ws in [&mut ws1, &mut ws2] {
                let hello = tokio::time::timeout(std::time::Duration::from_secs(2), ws.read_text())
                    .await
                    .expect("2s 内应收到就绪帧")
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&hello).unwrap()["type"],
                    "ws_connected"
                );
            }

            // 无任何 broadcast 事件，两端 2s 内都应收到心跳帧
            for ws in [&mut ws1, &mut ws2] {
                let frame = tokio::time::timeout(std::time::Duration::from_secs(2), ws.read_text())
                    .await
                    .expect("2s 内应收到心跳帧")
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&frame).unwrap()["type"],
                    "heartbeat"
                );
            }

            // 窗口计数防 broadcast 误用：100ms 间隔 → 单端 1.05s ≈10 份；
            // 若误走 broadcast，2 连接各自注入 → 单端会收到 ≈20 份
            let mut count = 0u32;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1050);
            while tokio::time::timeout_at(deadline, ws1.read_text())
                .await
                .is_ok()
            {
                count += 1;
            }
            assert!(
                (3..=15).contains(&count),
                "单连接心跳应 ≈10 份/s（实际 {count}；远超 15 说明心跳误走了 broadcast）"
            );
        });
    }

    #[test]
    fn test_ws_workflow_events_forwarded_and_control_endpoints() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            let port: u16 = base
                .trim_start_matches("http://127.0.0.1:")
                .parse()
                .unwrap();

            // WS 连接（正确 token）
            let (mut ws, headers) = ws_client::connect(port, &format!("/ws?token={token}"))
                .await
                .unwrap();
            assert!(headers.contains("101"), "正确 token 应完成升级: {headers}");

            // 就绪帧（消除 101 与 subscribe 竞态）
            let hello = tokio::time::timeout(std::time::Duration::from_secs(5), ws.read_text())
                .await
                .expect("5s 内应收到就绪帧")
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&hello).unwrap()["type"],
                "ws_connected"
            );

            // 引擎 emit RunStarted → WS 应收到 {"type":"workflow_event","event":"run_started",...}
            let state = app.state::<AppState>();
            {
                let engine = state.workflow_engine.read().await;
                engine
                    .event_bus()
                    .emit(nuphus::workflow::events::WorkflowEvent::RunStarted {
                        run_id: "run-1".to_string(),
                        workflow_id: "wf-1".to_string(),
                    });
            }

            // 跳过连接即发的 snapshot 帧，读到 workflow 事件
            let frame = loop {
                let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.read_text())
                    .await
                    .expect("5s 内应收到 workflow 事件")
                    .unwrap();
                let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
                if v["type"] != "session_snapshot" {
                    break frame;
                }
            };
            let json: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(json["type"], "workflow_event", "应携带 type 标记: {json}");
            assert_eq!(json["event"], "run_started", "event 应原样透传: {json}");
            assert_eq!(json["run_id"], "run-1", "run_id 应原样透传: {json}");
            assert_eq!(
                json["workflow_id"], "wf-1",
                "workflow_id 应原样透传: {json}"
            );

            // 控制端点鉴权：无 token → 401
            let r = np_client()
                .post(format!("{base}/workflow-pause"))
                .json(&serde_json::json!({ "workflow_id": "wf-1" }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401, "无 token 应 401");

            // 控制端点：正确 token → 200（引擎控制命令幂等，无运行任务时 no-op）
            for path in ["workflow-pause", "workflow-resume", "workflow-stop"] {
                let r = np_client()
                    .post(format!("{base}/{path}"))
                    .header("X-Mobile-Token", &token)
                    .json(&serde_json::json!({ "workflow_id": "wf-1" }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(
                    r.status(),
                    200,
                    "POST /{path} 应 200（实际 {}）",
                    r.status()
                );
            }

            // 空 workflow_id → 400
            let r = np_client()
                .post(format!("{base}/workflow-stop"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "workflow_id": "" }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "空 workflow_id 应 400");
        });
    }

    #[test]
    fn test_compound_emitter_none_is_tauri_only() {
        tokio_test::block_on(async {
            // server 未启动（mobile_ws_tx=None）→ CompoundEmitter 退化为纯 Tauri，
            // 不 panic、不产生任何 WS 输出——与 P0 后桌面行为等价
            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            handle.manage(AppState::default());
            let state = handle.state::<AppState>();
            assert!(state.mobile_ws_tx.lock().unwrap().is_none());

            let emitter = crate::emitter::CompoundEmitter::new(handle.clone(), state.inner());
            assert!(emitter.mobile.is_none());
            nuphus::agent::events::EventEmitter::emit(
                &emitter,
                nuphus::agent::events::NuphusEvent::DirectResponse {
                    message: "desktop only".to_string(),
                },
            ); // 不 panic 即通过（Tauri 端为 mock no-op）
        });
    }

    #[test]
    fn test_token_regenerate_invalidates_old() {
        tokio_test::block_on(async {
            let (base, old_token, app) = spawn_test_server().await;
            let state = app.state::<AppState>();
            state.busy.store(true, std::sync::atomic::Ordering::SeqCst); // 让鉴权通过后有确定性出口

            let post = |token: &str| {
                let base = base.clone();
                let token = token.to_string();
                async move {
                    np_client()
                        .post(format!("{base}/message"))
                        .header("X-Mobile-Token", token)
                        .json(&serde_json::json!({ "message": "x" }))
                        .send()
                        .await
                        .unwrap()
                }
            };

            // 旧 token 可用（200 append = 通过鉴权、命中 busy 转追加）
            let r = post(&old_token).await;
            assert_eq!(
                r.status(),
                200,
                "POST1 应 200 append（实际 {}）",
                r.status()
            );
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "append", "POST1 应标记为追加，实际: {body}");

            // regenerate：直接写共享 Arc（command 层同款逻辑，避开磁盘配置依赖）
            *state.mobile_token.write().unwrap() = generate_token();

            // 旧 token 即刻失效，新 token 生效
            let r = post(&old_token).await;
            assert_eq!(
                r.status(),
                401,
                "POST2 旧 token 应 401（实际 {}）",
                r.status()
            );
            let new_token = state.mobile_token.read().unwrap().clone();
            let r = post(&new_token).await;
            assert_eq!(
                r.status(),
                200,
                "POST3 新 token 应 200 append（实际 {}）",
                r.status()
            );
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body["status"], "append", "POST3 应标记为追加，实际: {body}");
            // 清理追加队列，避免滞留影响并行测试
            let _ = nuphus::mobile_append::drain_for_injection();
        });
    }

    /// 无 LLM 调用的 ApiClient 桩：仅用于构造带 session 的 Runtime（/history 测试）
    struct StubClient;

    #[async_trait::async_trait]
    impl nuphus::api::ApiClient for StubClient {
        async fn stream(
            &self,
            _request: nuphus::api::MessageRequest,
        ) -> nuphus::Result<Vec<nuphus::api::AssistantEvent>> {
            Ok(vec![])
        }

        fn model_name(&self) -> &str {
            "stub-model"
        }

        fn provider_kind(&self) -> nuphus::api::ProviderKind {
            nuphus::api::ProviderKind::MiniMax
        }
    }

    #[test]
    fn test_history_requires_auth() {
        tokio_test::block_on(async {
            let (base, token, _app) = spawn_test_server().await;
            let client = np_client();
            // 无 token → 401
            let r = client.get(format!("{base}/history")).send().await.unwrap();
            assert_eq!(r.status(), 401);
            // 错误 token → 401
            let r = client
                .get(format!("{base}/history"))
                .header("X-Mobile-Token", "wrong")
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401);
            // query token 等效可用（空 session → 空列表）
            let r = client
                .get(format!("{base}/history?token={token}"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body: serde_json::Value = r.json().await.unwrap();
            assert_eq!(body, serde_json::json!([]), "无 agent 时应返回空列表");
        });
    }

    #[test]
    fn test_history_returns_session_messages() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            // 构造带对话历史的 leader_agent（StubClient 无 LLM 调用，仅承载 session）
            let state = app.state::<AppState>();
            let mut runtime = nuphus::runtime::RuntimeBuilder::new()
                .llm(std::sync::Arc::new(StubClient))
                .tools(nuphus::ToolRegistry::leader())
                .build()
                .expect("Runtime 应可构造");
            runtime.set_history(&[
                ("user".to_string(), "帮我看下今天的日程".to_string()),
                ("assistant".to_string(), "今天有三项安排……".to_string()),
                ("user".to_string(), "第一项几点开始？".to_string()),
                ("assistant".to_string(), "上午 10 点。".to_string()),
            ]);
            state.runtime.lock().unwrap().leader_agent = Some(runtime);

            let r = np_client()
                .get(format!("{base}/history"))
                .header("X-Mobile-Token", &token)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body: Vec<serde_json::Value> = r.json().await.unwrap();
            assert_eq!(body.len(), 4, "两轮对话应返回 4 条消息");
            assert_eq!(body[0]["role"], "user");
            assert_eq!(body[0]["content"], "帮我看下今天的日程");
            assert_eq!(body[1]["role"], "assistant");
            assert_eq!(body[3]["content"], "上午 10 点。");
        });
    }

    #[test]
    fn test_spa_serving() {
        tokio_test::block_on(async {
            let (base, _token, _app) = spawn_test_server().await;
            // 不存在的资源 → 404（含构建提示）
            let r = np_get(&format!(
                "{base}/definitely-not-exist-{}.xyz",
                uuid::Uuid::new_v4()
            ))
            .await
            .unwrap();
            assert_eq!(r.status(), 404);
            // 路径穿越 → 404（不命中任何文件）
            let r = np_get(&format!("{base}/..%2F..%2FCargo.toml"))
                .await
                .unwrap();
            assert!(r.status() == 404 || r.status() == 400, "路径穿越应被拒绝");
            // dist 已构建时 / 应返回移动端入口 HTML（本机 dev 环境已构建）
            let dist_index = crate::commands::process::workspace_root()
                .join("frontend")
                .join("dist")
                .join("mobile.html");
            if dist_index.exists() {
                let r = np_get(&format!("{base}/")).await.unwrap();
                assert_eq!(r.status(), 200);
                let cache = r
                    .headers()
                    .get(axum::http::header::CACHE_CONTROL)
                    .map(|v| v.to_str().unwrap_or("").to_string());
                let body = r.text().await.unwrap();
                assert!(body.contains("root"), "mobile.html 应包含 React 挂载点");
                assert_eq!(cache.as_deref(), Some("no-cache"), "HTML 入口应 no-cache");
            }
        });
    }

    #[test]
    fn test_confirm_channel() {
        tokio_test::block_on(async {
            let (base, token, app) = spawn_test_server().await;
            let client = np_client();
            let body = serde_json::json!({
                "action_id": "test-action-001", "approved": true,
                "session": true, "tool": "system_shell"
            });

            // 无 token → 401
            let r = client
                .post(format!("{base}/confirm"))
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401);
            // 空 action_id → 400
            let r = client
                .post(format!("{base}/confirm"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "action_id": "  ", "approved": true }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 400);

            // 合法回执 → 200，且写入 SharedSignals 信号队列（agent 轮询消费点）
            let state = app.state::<AppState>();
            let r = client
                .post(format!("{base}/confirm"))
                .header("X-Mobile-Token", &token)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            // check_security_result 一次性消费：应取到 approved=true
            let result = nuphus::security::check_security_result(&state.signals, "test-action-001");
            assert_eq!(result, Some(true), "确认结果应已进入信号队列");
            // 对话级授权已登记
            assert!(
                nuphus::security::is_session_approved(&state.signals, "system_shell"),
                "session=true 时应登记对话级授权"
            );

            // 拒绝路径：approved=false
            let r = client
                .post(format!("{base}/confirm"))
                .header("X-Mobile-Token", &token)
                .json(&serde_json::json!({ "action_id": "test-action-002", "approved": false }))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let result = nuphus::security::check_security_result(&state.signals, "test-action-002");
            assert_eq!(result, Some(false), "拒绝结果应已进入信号队列");
        });
    }

    #[test]
    fn test_full_lifecycle_matches_settings_panel_flow() {
        // 与设置面板动线逐条对齐（P3 基线 2）：
        // 关（端口不监听）→ 开（端口监听 + health 200）→ QR 内容 = lan_url + ?token= →
        // 重置 token（旧 token 401 / 新 token 可用）→ 关（端口释放）
        tokio_test::block_on(async {
            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            handle.manage(AppState::default());
            let state = handle.state::<AppState>();

            // ① 关：未启动时不监听
            assert!(!current_status(state.inner()).running);

            // ② 开：start_server（面板 mobile_server_start 的同一函数）→ 端口监听
            *state.mobile_token.write().unwrap() = "lifecycle-token-0123456789abcd".to_string();
            let status = start_server(&handle, state.inner(), 0).await.unwrap();
            assert!(status.running);
            let base = format!("http://127.0.0.1:{}", status.port);
            let r = np_get(&format!("{base}/health")).await.unwrap();
            assert_eq!(r.status(), 200, "开启后端口应开始监听");

            // ③ QR 内容：面板 pairUrl = lan_url + "/?token=" + token（同一字符串拼接）
            let pair_url = format!(
                "{}/?token={}",
                status.lan_url.as_ref().unwrap(),
                status.token
            );
            assert!(pair_url.starts_with("http://"));
            assert!(
                pair_url.contains(":/"),
                "lan_url 应为可直连的本机局域网地址"
            );
            assert!(pair_url.ends_with(&format!("?token={}", status.token)));
            assert!(status
                .lan_url
                .as_ref()
                .unwrap()
                .contains(&status.port.to_string()));
            // token 有效：/history 200
            let r = np_get(&format!("{base}/history?token={}", status.token))
                .await
                .unwrap();
            assert_eq!(r.status(), 200);

            // ④ 重置 token（面板 mobile_token_regenerate 的同一逻辑）：QR 刷新 + 旧 token 401
            let old_token = status.token.clone();
            let new_token = generate_token();
            *state.mobile_token.write().unwrap() = new_token.clone();
            let r = np_get(&format!("{base}/history?token={old_token}"))
                .await
                .unwrap();
            assert_eq!(r.status(), 401, "重置后旧 token 应立即失效");
            let r = np_get(&format!("{base}/history?token={new_token}"))
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "重置后新 token 应可用");
            // 新 QR 内容应携带新 token（面板 refresh 后重新拼接）
            let new_pair_url = format!("{}/?token={}", status.lan_url.unwrap(), new_token);
            assert!(new_pair_url.ends_with(&format!("?token={new_token}")));
            assert_ne!(pair_url, new_pair_url);

            // ⑤ 关：stop_server（面板 mobile_server_stop 的同一函数）→ 端口释放
            stop_server(state.inner()).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert!(
                np_get(&format!("{base}/health")).await.is_err(),
                "停止后端口应释放（连接被拒）"
            );
            assert!(!current_status(state.inner()).running);
        });
    }

    #[test]
    fn test_config_persistence_roundtrip() {
        // 配置读写往返（temp 文件，不触真实 config_dir）——重启后 token 不变的持久化基础
        let dir = std::env::temp_dir().join(format!("nuphus-mobile-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("mobile_server.json");
        let cfg = MobileServerConfig {
            enabled: true,
            port: 18772,
            token: generate_token(),
            password_hash: hash_password("abc123"),
        };
        save_config_to(&path, &cfg).unwrap();
        let loaded = load_config_from(&path);
        assert!(loaded.enabled);
        assert_eq!(loaded.port, 18772);
        assert_eq!(loaded.token, cfg.token);
        assert_eq!(
            loaded.password_hash, cfg.password_hash,
            "password_hash 应往返一致"
        );
        assert!(loaded.token.len() >= 32, "token 应 ≥32 字符");

        // 旧配置兼容：无 password_hash 字段的 JSON 必须能反序列化（serde default）
        let legacy_json = r#"{"enabled":true,"port":18772,"token":"legacy-token"}"#;
        std::fs::write(&path, legacy_json).unwrap();
        let legacy = load_config_from(&path);
        assert!(legacy.enabled);
        assert_eq!(legacy.port, 18772);
        assert_eq!(legacy.token, "legacy-token");
        assert!(
            legacy.password_hash.is_empty(),
            "旧配置缺 password_hash 字段时应反序列化为空串"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_password_hash_roundtrip() {
        // hash 格式 "{salt}:{hex}"；salt=32 hex；hex=64 hex
        let h = hash_password("abc123");
        let (salt, digest) = h.split_once(':').expect("hash 应含 ':' 分隔");
        assert_eq!(salt.len(), 32, "salt 应为 32 hex（UUIDv4 simple）");
        assert_eq!(digest.len(), 64, "sha256 hex 应为 64 字符");

        // 正确密码通过；错误密码不通过；同密码两次 salt 不同
        assert!(verify_password("abc123", &h));
        assert!(!verify_password("abc124", &h));
        assert_ne!(hash_password("abc123"), h, "每次哈希 salt 应随机");

        // 非法存储格式一律拒绝
        assert!(!verify_password("abc123", "no-separator"));
        assert!(!verify_password("abc123", ":"));
        assert!(!verify_password("abc123", ""));
    }

    #[test]
    fn test_validate_password() {
        // 合法：≥6 位且含字母+数字
        assert!(validate_password("abc123").is_ok());
        assert!(validate_password("passw0rd").is_ok());
        assert!(validate_password("AbC123").is_ok());
        // 过短
        assert!(validate_password("ab1").is_err());
        // 纯字母 / 纯数字 / 含非 ASCII 字母
        assert!(validate_password("abcdef").is_err());
        assert!(validate_password("123456").is_err());
        assert!(
            validate_password("你好123").is_err(),
            "非 ASCII 字母不满足要求"
        );
        assert!(validate_password("").is_err());
    }
    // ── CORS 白名单（审计 P2-3）──
    // 注：中继分支依赖真实 relay_client.json 配置，单测环境通常无此文件
    // （load_config 返回默认 disabled → 中继分支不命中），以下只测与配置无关的分支。

    #[test]
    fn cors_allows_localhost_and_rejects_strangers() {
        // localhost / 127.0.0.1 开发源放行
        assert_eq!(
            cors_allowed_origin(Some("http://localhost:5174")),
            Some("http://localhost:5174")
        );
        assert_eq!(
            cors_allowed_origin(Some("http://127.0.0.1:18771")),
            Some("http://127.0.0.1:18771")
        );
        // 陌生跨源 → 不下发 ACAO
        assert_eq!(cors_allowed_origin(Some("https://evil.example.com")), None);
        assert_eq!(cors_allowed_origin(Some("http://192.168.1.99:8080")), None);
        // 无 Origin（同源/非浏览器）→ 无需 CORS 头
        assert_eq!(cors_allowed_origin(None), None);
    }

    #[test]
    fn origin_host_of_parses_scheme_port_and_ipv6() {
        // 裸域名
        assert_eq!(
            origin_host_of("https://r.example.com").as_deref(),
            Some("r.example.com")
        );
        // 带端口（隧道明文口 / HTTP API 口同 host 放行的关键）
        assert_eq!(
            origin_host_of("http://r.example.com:18081").as_deref(),
            Some("r.example.com")
        );
        assert_eq!(
            origin_host_of("http://r.example.com:18080").as_deref(),
            Some("r.example.com")
        );
        // 尾部斜杠
        assert_eq!(
            origin_host_of("https://r.example.com/").as_deref(),
            Some("r.example.com")
        );
        // IPv6 字面量保留 [] 段
        assert_eq!(
            origin_host_of("http://[::1]:18772").as_deref(),
            Some("[::1]")
        );
        // 无 scheme 裸 host
        assert_eq!(
            origin_host_of("r.example.com").as_deref(),
            Some("r.example.com")
        );
        // 非法输入
        assert_eq!(origin_host_of(""), None);
        assert_eq!(origin_host_of("://"), None);
    }
}
// touch-force-recompile