//! 中继客户端 — 桌面 Nuphus 出站连接中继服务器，接收外部任务并执行。
//!
//! 架构（对齐 mobile_server.json 配置模式）：
//! - 配置：`config_dir/nuphus/relay_client.json`（enabled / url / device_id / token）
//! - 启动：Tauri setup 阶段 spawn，enabled=true 时自动连接
//! - 连接：出站 WebSocket（tokio-tungstenite），断线指数退避重连（1s..60s）
//! - 任务：收到 `{"type":"task","task_id","content"}` → 调注入的 handler 执行
//!   → 回传 `{"type":"result"|"error","task_id","content"|"message"}`
//! - 生产 handler 复用共享入口 `commands::process::submit_user_message`
//!   （与桌面/手机同一条 Agent 执行链路，source="relay"）
//! - MVP 原则：只做「外部触发 → 桌面执行 → 结果回传」，不落盘、不排队

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Manager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{self, Message as WsMessage};

/// 中继客户端配置（config_dir/nuphus/relay_client.json）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RelayClientConfig {
    pub enabled: bool,
    /// 中继服务器 base URL，如 ws://relay.example.com:18080
    pub url: String,
    /// 本机设备标识（中继按此路由）
    pub device_id: String,
    /// 设备鉴权 token（与中继服务端 RELAY_DEVICE_TOKEN 一致）
    pub token: String,
    /// 调用方 token（与中继服务端 RELAY_CALLER_TOKEN 一致）；
    /// 客户端自身不用，经 /relay-hint 下发给手机端外网发送用
    pub caller_token: String,
    /// 隧道公网入口（手机外网访问地址），如 https://r.example.com；
    /// 为空时从 url 派生（ws://host:18080 → http://host:18081）
    #[serde(default)]
    pub public_url: String,
}

fn config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("nuphus")
        .join("relay_client.json")
}

// ── 官方中继默认配置（开箱即用，零配置） ──────────────────────────────
// 首次启动自动写入 url/public_url；自建中继用户可覆盖 relay_client.json 后不受影响。
// ⚠️ 官方中继凭据（device/caller token）绝不硬编码在源码——仓库公开（github/gitee），
// 硬编码 = 任何人克隆即拿到服务器凭据（2026-08-26 审计发现，commit ab71791 曾硬编码）。
// 改为构建/运行时经环境变量注入：NUPHUS_RELAY_DEVICE_TOKEN / NUPHUS_RELAY_CALLER_TOKEN；
// 未注入时留空（用户自建中继在 relay_client.json 自行配置，或官方发布流程注入）。
const DEFAULT_RELAY_URL: &str = "wss://relay.nuphus.com";
const DEFAULT_RELAY_PUBLIC_URL: &str = "https://r.nuphus.com";

/// 历史版本曾默认下发的共享 device_id——公共中继上多用户互抢同一路由槽的根源，
/// ensure_default_config 检测到即自动迁移为唯一 id
const LEGACY_SHARED_DEVICE_IDS: [&str; 1] = ["desktop-main"];

/// 确保中继配置就绪：文件缺失或关键字段为空时写入官方默认值（幂等，可重复调用）。
/// 调用时机：Tauri setup（spawn_relay_loops 之前）——新用户首次使用免配置，
/// 开启手机访问即自动启用官方中继；自建中继用户已有配置则原样保留。
pub fn ensure_default_config() {
    let mut cfg = load_config();
    let mut changed = false;
    if cfg.url.trim().is_empty() {
        cfg.url = DEFAULT_RELAY_URL.to_string();
        changed = true;
    }
    if cfg.token.trim().is_empty() {
        // 官方凭据仅经构建期注入（env!：发布 CI 设环境变量 → 二进制内嵌）或运行时
        // 环境变量注入；均未注入则留空，用户自建中继在 relay_client.json 自行配置。
        let injected = env!("NUPHUS_RELAY_DEVICE_TOKEN").trim().to_string();
        let rt = std::env::var("NUPHUS_RELAY_DEVICE_TOKEN").unwrap_or_default();
        let t = if !injected.is_empty() { injected } else { rt };
        if !t.trim().is_empty() {
            cfg.token = t;
            changed = true;
        }
    }
    if cfg.caller_token.trim().is_empty() {
        let injected = env!("NUPHUS_RELAY_CALLER_TOKEN").trim().to_string();
        let rt = std::env::var("NUPHUS_RELAY_CALLER_TOKEN").unwrap_or_default();
        let t = if !injected.is_empty() { injected } else { rt };
        if !t.trim().is_empty() {
            cfg.caller_token = t;
            changed = true;
        }
    }
    if cfg.public_url.trim().is_empty() {
        cfg.public_url = DEFAULT_RELAY_PUBLIC_URL.to_string();
        changed = true;
    }
    // device_id 必须每机唯一（中继按 device_id 路由）：缺失时随机生成。
    // 遗留共享默认值迁移（2026-08-25 实测事故）：旧版本默认 desktop-main，公共中继
    // 上所有此类用户互相抢占同一路由槽——连接风暴、手机被路由到陌生人的电脑
    // （配对失效/指令丢失）。检测到即换发唯一 id 并持久化。
    if cfg.device_id.trim().is_empty() || LEGACY_SHARED_DEVICE_IDS.contains(&cfg.device_id.trim()) {
        let legacy = cfg.device_id.trim().to_string();
        cfg.device_id = format!("desktop-{}", uuid::Uuid::new_v4().simple());
        changed = true;
        if !legacy.is_empty() {
            tracing::warn!(
                "[Relay] 检测到遗留共享 device_id「{legacy}」（公共中继多用户撞车源），已迁移为 {}",
                cfg.device_id
            );
        }
    }
    if changed {
        if let Err(e) = save_config(&cfg) {
            tracing::warn!("[Relay] 初始化默认配置失败: {e}");
        } else {
            tracing::info!(
                "[Relay] 已初始化官方中继默认配置（device_id={}）",
                cfg.device_id
            );
        }
    }
}

pub fn load_config() -> RelayClientConfig {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            // BOM 免疫：外部编辑器保存常带 BOM，serde_json 解析会失败——
            // 静默回退默认值曾把 enabled/token 清掉（二维码消失事故根因）
            match serde_json::from_str::<RelayClientConfig>(raw.trim_start_matches('\u{feff}')) {
                Ok(cfg) => cfg,
                Err(e) => {
                    tracing::error!(
                        "[Relay] relay_client.json 解析失败（回退默认值，原文件保留）: {e}"
                    );
                    RelayClientConfig::default()
                }
            }
        }
        Err(_) => RelayClientConfig::default(),
    }
}

/// 配置写入（中继开关等设置页入口）
pub fn save_config(cfg: &RelayClientConfig) -> Result<(), String> {
    if let Some(parent) = config_path().parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let data = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    std::fs::write(config_path(), data).map_err(|e| format!("写入配置失败: {e}"))
}

/// 中继开关（MobilePage「中继转发」开关）：持久化 enabled + 运行时即时启停（免重启）。
/// 开：配置完整才启动双回路，不完整返回错误提示；关：优雅退出（等任务收口）。
#[tauri::command]
pub async fn relay_client_set_enabled(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<String, String> {
    let mut cfg = load_config();
    cfg.enabled = enabled;
    if enabled && !relay_loops_allowed(&cfg) {
        return Err(
            "中继配置不完整（url/device_id/token），请先完成 relay_client.json 配置".into(),
        );
    }
    save_config(&cfg)?;
    if enabled {
        spawn_relay_loops(app);
        Ok("中继已启用".into())
    } else {
        stop_relay_loops().await;
        Ok("中继已停用".into())
    }
}

/// caller_token 轮换（MobilePage「重新生成调用凭据」按钮）。
/// 调中继管理端点（device_token 鉴权）→ 服务端写 relay_caller.token 热生效；
/// 成功后更新本地 relay_client.json 并重启中继回路。旧凭据即刻失效——
/// 已配对手机的外网访问需重新扫二维码。
/// 依赖服务端带 /admin/rotate-caller-token 的版本；旧服务端 404 时明确提示升级。
#[tauri::command]
pub async fn relay_caller_token_rotate(app: tauri::AppHandle) -> Result<String, String> {
    let mut cfg = load_config();
    if cfg.url.trim().is_empty() || cfg.token.is_empty() {
        return Err("中继配置不完整（url/token），无法轮换".into());
    }
    let base = cfg.url.trim().trim_end_matches('/');
    let http_base = base
        .replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1);
    let resp = reqwest::Client::new()
        .post(format!("{http_base}/admin/rotate-caller-token"))
        .header("X-Relay-Token", &cfg.token)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("轮换请求失败: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("中继服务端版本过旧（无轮换端点），请先升级 VPS 上的 relay-server".into());
    }
    if !resp.status().is_success() {
        return Err(format!("轮换被拒: HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;
    let new_token = body
        .get("caller_token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "响应缺少 caller_token".to_string())?
        .to_string();
    cfg.caller_token = new_token;
    save_config(&cfg)?;
    // 回路重启：设备通道用 device_token 不受轮换影响，但重启保证 hint 下发读到新值、
    // 连接状态干净（stop 等任务收口，enabled 时立即重连）
    stop_relay_loops().await;
    if cfg.enabled {
        spawn_relay_loops(app);
    }
    Ok("调用凭据已重新生成，旧凭据即刻失效".into())
}

// ── 消息协议（与中继服务端 relay-server 对齐） ─────────────────────────

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Task { task_id: String, content: String },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeviceMsg {
    Result { task_id: String, content: String },
    Error { task_id: String, message: String },
}

fn ws_url(cfg: &RelayClientConfig) -> String {
    let base = cfg.url.trim_end_matches('/');
    format!("{}/ws/device?device_id={}", base, cfg.device_id)
}

/// 构造 WS 连接请求：token 走 Authorization: Bearer，**不进 URL**（防日志/代理/截图泄露）。
/// 中继服务端三通道鉴权（Header / 子协议 / query），Header 优先级最高。
fn ws_request(url: &str, token: &str) -> Result<http::Request<()>, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = url
        .to_string()
        .into_client_request()
        .map_err(|e| format!("WS 请求构造失败: {e}"))?;
    req.headers_mut().insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&format!("Bearer {}", token))
            .map_err(|e| format!("token 构造失败: {e}"))?,
    );
    Ok(req)
}

/// 建立带 TCP keepalive 的 WS 连接（防 NAT 空闲掐断「半死连接」）。
///
/// 背景：桌面 relay 双回路（/ws/device + /ws/tunnel）空闲时无任何帧，运营商 NAT 会在
/// 空闲超时（实测 ~6.5 分钟）后静默掐断 TCP——两端 TCP 层仍 ESTABLISHED、read 永不
/// 返回，形成「半死连接」。手机经中继访问时 Open 帧发不到桌面 → 白屏。
///
/// keepalive（idle 30s / interval 10s）：
/// - 每 30s 发 TCP 探测包刷新 NAT 表项 → 预防半死；
/// - 若 NAT 已掐断，探测无 ACK，TCP 层重试后报错 → read 返回 Err → 触发重连。
///
/// 这是 TCP 层标准保活，非业务层「心跳」，符合「无心跳无服务器检测」架构意图。
async fn connect_ws_with_keepalive(
    url: &str,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ConnectError,
> {
    let req = ws_request(url, token).map_err(ConnectError::Request)?;
    let uri = req.uri().clone();
    let host = uri
        .host()
        .ok_or_else(|| ConnectError::Request("URL 缺少 host".into()))?;
    // wss:// 未显式写端口时 uri.port_u16() 返回 None——必须按 scheme 取默认端口
    // （wss/https → 443），否则会连到 80 端口被反代 308 重定向（HTTP 308 故障根因）。
    let port = uri.port_u16().unwrap_or_else(|| match uri.scheme_str() {
        Some("wss") | Some("https") => 443,
        _ => 80,
    });

    let tcp = tokio::net::TcpStream::connect((host, port))
        .await
        .map_err(|e| ConnectError::Handshake(Box::new(tungstenite::Error::Io(e))))?;

    // 设 TCP keepalive：std TcpStream 的 set_keepalive 只能开/关，无法设短间隔，
    // 需经 socket2::SockRef 设 idle/interval。转 std → 设置 → 转回 tokio。
    let std_tcp = tcp
        .into_std()
        .map_err(|e| ConnectError::Request(format!("socket 转换失败: {e}")))?;
    {
        let sock = socket2::SockRef::from(&std_tcp);
        sock.set_keepalive(true)
            .map_err(|e| ConnectError::Request(format!("set_keepalive 失败: {e}")))?;
        let ka = socket2::TcpKeepalive::new()
            .with_time(Duration::from_secs(30))
            .with_interval(Duration::from_secs(10));
        sock.set_tcp_keepalive(&ka)
            .map_err(|e| ConnectError::Request(format!("set_tcp_keepalive 失败: {e}")))?;
    }
    let tcp = tokio::net::TcpStream::from_std(std_tcp)
        .map_err(|e| ConnectError::Request(format!("socket 转回失败: {e}")))?;

    // 握手按 scheme 决定是否包 TLS：wss→native-tls TLS 层，ws→裸 TCP。
    // 之前用 client_async（裸 WS）——迁移 wss:// 后缺 TLS 层，明文 HTTP 升级请求打到
    // 443 TLS 端口，被服务端回「HTTP version must be 1.1 or higher」。
    //
    // 整段拨号（TCP+TLS+WS 升级）加超时：对端半开/代理黑洞时 client_async_tls
    // 会永久悬挂——外层循环随之卡死、不再有任何重试（实测事故：隧道静默失联
    // 35 分钟，僵尸 ESTABLISHED socket 挂着，无任何日志）。超时按网络类失败走退避。
    let (ws, _resp) = tokio::time::timeout(
        Duration::from_secs(WS_DIAL_TIMEOUT_SECS),
        tokio_tungstenite::client_async_tls(req, tcp),
    )
    .await
    .map_err(|_| {
        ConnectError::Handshake(Box::new(tungstenite::Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "dial timeout: {}s 内未完成 TCP/TLS/WS 升级",
                WS_DIAL_TIMEOUT_SECS
            ),
        ))))
    })?
    .map_err(|e| ConnectError::Handshake(Box::new(e)))?;
    Ok(ws)
}

// ── 重连统一状态机（任务通道 / 隧道两回路各持一个 RelayBackoff，同构）─────────
// 设计约束（大王定调）：
// - 任何状态下探测永不停止——网络错误指数退避，配置错误切 60s 慢档，只降速不停止；
// - 日志只在状态迁移时记录：首次失败 warn → 持续失败降 debug → 进故障态 warn 一次
//   → 恢复连接 info 一次。中继挂一整天，每回路 WARN ≤ 2 条；
// - 连接状态外化（RELAY_STATE），/relay-hint 与桌面设置页共用同一数据源。

/// 失败分级：配置/鉴权类 vs 网络类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// 网络类：Io / 5xx / 连接建立后的运行中断——指数退避 3s×2→60s
    Network,
    /// 配置类：HTTP 4xx / 本地请求构造失败——直接 60s 慢档（3s 白试无意义）
    Config,
}

/// 连接握手错误分级（纯函数）：HTTP 4xx → Config（鉴权/路由错误）；
/// 5xx / Io / 其余 → Network。
fn classify_connect_error(err: &tungstenite::Error) -> FailKind {
    match err {
        tungstenite::Error::Http(resp) => match resp.status().as_u16() {
            400..=499 => FailKind::Config,
            _ => FailKind::Network,
        },
        _ => FailKind::Network,
    }
}

/// 字符串兜底分级：连接建立后的运行中断、panic 等无法结构化的错误 → 一律网络类
fn classify_fallback(_msg: &str) -> FailKind {
    FailKind::Network
}

/// 连接错误：区分「握手阶段」（可分级）与「连接建立后的运行中断」（一律网络类）
#[derive(Debug)]
pub enum ConnectError {
    /// WS 握手阶段失败（tungstenite 错误，HTTP 响应可提取状态码）
    Handshake(Box<tungstenite::Error>),
    /// 本地请求构造失败（非法 URL / token 头）——配置类
    Request(String),
    /// 连接建立后的运行中断（掉线 / 看门狗 / 读错误）——一律网络类
    Runtime(String),
}

impl ConnectError {
    fn kind(&self) -> FailKind {
        match self {
            ConnectError::Handshake(e) => classify_connect_error(e),
            ConnectError::Request(_) => FailKind::Config,
            ConnectError::Runtime(msg) => classify_fallback(msg),
        }
    }

    fn summary(&self) -> String {
        match self {
            ConnectError::Handshake(e) => match &**e {
                tungstenite::Error::Http(resp) => format!("HTTP {}", resp.status().as_u16()),
                other => other.to_string(),
            },
            ConnectError::Request(m) | ConnectError::Runtime(m) => m.clone(),
        }
    }

    /// 连接是否曾成功建立（决定外层能否重置退避）
    fn was_connected(&self) -> bool {
        matches!(self, ConnectError::Runtime(_))
    }
}

const BACKOFF_BASE_SECS: u64 = 1;
const BACKOFF_CAP_SECS: u64 = 60;
/// 连接建立（TCP+TLS+WS 升级）全程超时：任一阶段悬挂即按网络类失败重试，
/// 杜绝「僵尸拨号」卡死重连循环（实测：隧道静默失联 35 分钟无任何尝试）
const WS_DIAL_TIMEOUT_SECS: u64 = 15;
/// 隧道读空闲看门狗：服务端每 10s 下发协议 Ping，60s 无任何帧 = 链路/对端任务
/// 已死但 TCP 未断（半开），强制断开走重连自愈。正常路径永不触发。
const TUNNEL_READ_IDLE_SECS: u64 = 60;
/// 连续网络失败达到此次数进故障态（warn 一次，探测继续）
const FAULT_THRESHOLD: u32 = 5;

/// 退避状态机：Network 1s→×2→cap 60s（±20% 抖动）；Config 固定 60s 慢档；
/// 成功连接 on_success() 重置回 1s 快档。写方向半死期间手机白屏窗口 = 重连延迟，
/// 基值从 3s 降到 1s 让恢复更快（无心跳架构下隧道重连是唯一自愈路径）。
pub struct RelayBackoff {
    current: Duration,
    /// 连续失败次数（成功连接后清零）
    pub fail_count: u32,
    /// Config 慢档标记
    slow: bool,
    rng: u64,
}

impl RelayBackoff {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() ^ u64::from(d.subsec_nanos()))
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            | 1;
        Self {
            current: Duration::from_secs(BACKOFF_BASE_SECS),
            fail_count: 0,
            slow: false,
            rng: seed,
        }
    }

    /// 当前档位基准值（不含抖动）
    fn base_delay(&self) -> Duration {
        if self.slow {
            Duration::from_secs(BACKOFF_CAP_SECS)
        } else {
            self.current
        }
    }

    /// 本次重试等待时长：基准 ±20% 抖动（防多客户端同频锤中继），抖动后再钳 cap——
    /// 保证任何情况下 ≤60s（规格：中继恢复后 60s 内必重连成功）
    pub fn next_delay(&mut self) -> Duration {
        jitter(self.base_delay(), next_rand01(&mut self.rng))
            .min(Duration::from_secs(BACKOFF_CAP_SECS))
    }

    pub fn on_failure(&mut self, kind: FailKind) {
        self.fail_count = self.fail_count.saturating_add(1);
        match kind {
            FailKind::Config => self.slow = true,
            FailKind::Network => {
                if !self.slow {
                    self.current = (self.current * 2).min(Duration::from_secs(BACKOFF_CAP_SECS));
                }
            }
        }
    }

    /// 连接成功建立：重置回 1s 快档、清零失败计数、退出慢档
    pub fn on_success(&mut self) {
        self.current = Duration::from_secs(BACKOFF_BASE_SECS);
        self.fail_count = 0;
        self.slow = false;
    }
}

/// xorshift64 伪随机 → [0, 1)
fn next_rand01(seed: &mut u64) -> f64 {
    let mut x = *seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// 基准时长 ±20% 抖动（纯函数）
fn jitter(base: Duration, rand01: f64) -> Duration {
    let ms = base.as_millis() as f64;
    let factor = 1.0 + (rand01 - 0.5) * 0.4;
    Duration::from_millis((ms * factor).round().max(1.0) as u64)
}

// ── 连接状态外化（进程级共享，/relay-hint 与桌面设置页同一数据源）────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chan {
    Relay,
    Tunnel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChannelState {
    Connected,
    Retrying { since: i64, attempts: u32 },
    Fault { reason: String },
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayConnState {
    pub relay: ChannelState,
    pub tunnel: ChannelState,
}

static RELAY_STATE: std::sync::RwLock<RelayConnState> = std::sync::RwLock::new(RelayConnState {
    relay: ChannelState::Disabled,
    tunnel: ChannelState::Disabled,
});

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn set_channel_state(chan: Chan, state: ChannelState) {
    let mut guard = RELAY_STATE.write().unwrap_or_else(|e| e.into_inner());
    match chan {
        Chan::Relay => guard.relay = state,
        Chan::Tunnel => guard.tunnel = state,
    }
}

/// 失败时的状态迁移：Config → 立即 Fault；Network 连续 ≥5 次 → Fault；否则 Retrying
/// （since 取本轮连续失败的首次时间，attempts 随次数更新）
fn update_state_on_failure(chan: Chan, kind: FailKind, fail_count: u32, summary: &str) {
    match kind {
        FailKind::Config => set_channel_state(
            chan,
            ChannelState::Fault {
                reason: format!("配置/鉴权类错误（{summary}），请检查 relay_client.json"),
            },
        ),
        FailKind::Network if fail_count >= FAULT_THRESHOLD => set_channel_state(
            chan,
            ChannelState::Fault {
                reason: format!("连续 {fail_count} 次连接失败：{summary}"),
            },
        ),
        FailKind::Network => {
            let mut guard = RELAY_STATE.write().unwrap_or_else(|e| e.into_inner());
            let cur = match chan {
                Chan::Relay => &mut guard.relay,
                Chan::Tunnel => &mut guard.tunnel,
            };
            let since = match cur {
                ChannelState::Retrying { since, .. } => *since,
                _ => now_unix(),
            };
            *cur = ChannelState::Retrying {
                since,
                attempts: fail_count,
            };
        }
    }
}

/// 读取进程级中继连接状态
pub fn relay_conn_state() -> RelayConnState {
    RELAY_STATE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// 失败登记：退避推进 + 日志纪律 + 状态外化（两回路共用），返回本次重试等待时长。
/// 日志纪律：第 1 次失败 warn（含错误摘要）→ 第 2 次 info 一次「进入静默重试」后
/// 详情降 debug → 连续第 5 次进故障态 warn 一次 → 之后全程 debug（探测永不停止）。
/// Config 错误首次出现即 warn 指引并切 60s 慢档。
fn register_failure(
    chan: Chan,
    tag: &str,
    backoff: &mut RelayBackoff,
    fault_logged: &mut bool,
    kind: FailKind,
    summary: &str,
) -> Duration {
    let first_config = kind == FailKind::Config && !backoff.slow;
    backoff.on_failure(kind);
    let n = backoff.fail_count;
    let delay = backoff.next_delay();

    update_state_on_failure(chan, kind, n, summary);

    match n {
        1 => tracing::warn!(
            "{} 连接失败: {}（{}s 后重试）",
            tag,
            summary,
            delay.as_secs()
        ),
        2 => {
            tracing::info!(
                "{} 持续失败，进入静默重试（详情降 debug，实时状态见移动端页面）",
                tag
            );
            tracing::debug!(
                "{} 第 {} 次连接失败: {}（{}s 后重试）",
                tag,
                n,
                summary,
                delay.as_secs()
            );
        }
        _ => tracing::debug!(
            "{} 第 {} 次连接失败: {}（{}s 后重试）",
            tag,
            n,
            summary,
            delay.as_secs()
        ),
    }
    if first_config {
        tracing::warn!(
            "{} 配置/鉴权类错误（{}），已切 60s 慢档探测，请检查 relay_client.json",
            tag,
            summary
        );
        *fault_logged = true;
    } else if n >= FAULT_THRESHOLD && !*fault_logged {
        tracing::warn!(
            "{} 连续 {} 次连接失败，进入故障态（探测不停止，按退避档位继续重试）",
            tag,
            n
        );
        *fault_logged = true;
    }
    delay
}

/// 隧道公网入口端口——与 relay-server 的 RELAY_TUNNEL_PORT 默认值约定一致
/// （relay-server/src/main.rs 启动公网隧道监听，env 未设置时为 18081）。
/// 桌面端按此约定从中继 WS 地址派生公网入口，不引入新配置项。
const TUNNEL_PUBLIC_PORT: u16 = 18081;

/// 隧道公网入口基址 URL（手机外网访问地址），**裸 origin 形态、不带任何参数**。
/// 消费方（手机端 relay-hint 的 tunnel_url）把它当字符串前缀拼接 REST/WS 路径
/// （frontend/mobile api.ts resolveApi / ws.ts host 提取）——带 query 会拼出坏 URL，
/// 因此本函数保持改造前语义：显式 public_url 原样返回；派生规则
/// ws://host:18080 → http://host:18081、wss:// → https://，host 保留，端口替换。
/// 未启用或 url 为空 → None。多设备路由的 device 标记由导航入口变体携带
/// （public_tunnel_entry_url），运行时连接靠中继 IP 粘性路由桥接。
/// 手写解析（src-tauri 无 url crate 依赖，不为本需求单加依赖），
/// 仅处理 `scheme://authority[/path]` 形态——配置即此形态。
pub(crate) fn public_tunnel_url(cfg: &RelayClientConfig) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    // 显式配置的隧道公网入口优先（支持裸域名 + HTTPS，如 https://r.example.com）
    if !cfg.public_url.trim().is_empty() {
        return Some(cfg.public_url.trim().trim_end_matches('/').to_string());
    }
    if cfg.url.trim().is_empty() {
        return None;
    }
    let raw = cfg.url.trim().trim_end_matches('/');
    let (scheme, rest) = raw.split_once("://")?;
    let http_scheme = match scheme {
        "ws" => "http",
        "wss" => "https",
        _ => return None,
    };
    // authority 为 host[:port]，丢弃可能存在的路径段；IPv6 字面量取 [...] 段
    let authority = rest.split('/').next()?;
    let host = if let Some(end) = authority.strip_prefix('[').and_then(|a| a.find(']')) {
        &authority[..=end + 1]
    } else {
        authority.split(':').next()?
    };
    if host.is_empty() {
        return None;
    }
    Some(format!("{}://{}:{}", http_scheme, host, TUNNEL_PUBLIC_PORT))
}

/// 隧道公网**导航入口** URL = public_tunnel_url 基址 + `?device=<device_id>`。
/// 用途：桌面设置页二维码/配对链接等「浏览器直接导航」场景——中继按该参数把
/// 首条隧道连接路由到本机，并播种 IP 粘性表（后续无标记连接沿用，见 relay-server）。
/// 仅基址不含 query/fragment 时追加（拼接语义不可靠时宁可不拼）；device_id 缺失或
/// 未启用 → 与基址行为一致（None / 裸基址走服务端默认兜底）。
pub(crate) fn public_tunnel_entry_url(cfg: &RelayClientConfig) -> Option<String> {
    let base = public_tunnel_url(cfg)?;
    let device = cfg.device_id.trim();
    if device.is_empty() || base.contains('?') || base.contains('#') {
        return Some(base);
    }
    Some(format!("{base}/?device={device}"))
}

/// 桌面设置页查询中继连接状态（与 /relay-hint 同一数据源，只读）
#[derive(Serialize)]
pub struct RelayClientStatus {
    pub enabled: bool,
    pub state: RelayConnState,
    /// 隧道公网入口（http(s)://host[:port]，多设备改造后携带 ?device=<device_id>），
    /// 未启用中继时为 None；
    /// 设置页「远程访问」引导用它拼接配对链接
    pub public_url: Option<String>,
}

#[tauri::command]
pub fn relay_client_status() -> RelayClientStatus {
    let mut cfg = load_config();
    // 二维码必须带 device_id（公共中继归属依据）。缺失/被外部编辑器清空时幂等补齐——
    // 裸基址二维码在公共中继多用户在线时会被 Ambiguous 拒成引导页
    // （「扫码连不上 / 显示多台电脑在线」根因之一，2026-08-25 实测事故）。
    if cfg.device_id.trim().is_empty() {
        ensure_default_config();
        cfg = load_config();
    }
    RelayClientStatus {
        enabled: cfg.enabled,
        state: relay_conn_state(),
        // 二维码/配对链接是导航入口：携带 ?device= 供中继按设备路由（多用户改造）
        public_url: public_tunnel_entry_url(&cfg),
    }
}

// ── 回路生命周期管理 ────────────────────────────────────────────────────
// 2026-08 起 Pro 体系移除：enabled + 配置完整即启动，无套餐门禁。

/// 运行中的双回路句柄：shutdown 发送端 + 任务 JoinHandle
struct RelayHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
}

static RELAY_HANDLE: std::sync::RwLock<Option<RelayHandle>> = std::sync::RwLock::new(None);

/// 启动条件（纯函数）：配置 enabled 且完整。
pub fn relay_loops_allowed(cfg: &RelayClientConfig) -> bool {
    cfg.enabled && !cfg.url.is_empty() && !cfg.device_id.is_empty() && !cfg.token.is_empty()
}

/// 启动中继双回路（幂等：已运行则不重复 spawn）。
/// 调用时机：Tauri setup（main.rs）一次。
pub fn spawn_relay_loops(app: tauri::AppHandle) {
    let mut guard = RELAY_HANDLE.write().unwrap_or_else(|e| e.into_inner());
    if guard.is_some() {
        return;
    }
    let cfg = load_config();
    if !relay_loops_allowed(&cfg) {
        // 「enabled 但配置不完整」才告警；未 enabled 一律静默。
        if cfg.enabled {
            tracing::warn!("[Relay] enabled=true 但配置不完整（url/device_id/token），跳过");
        }
        return;
    }
    tracing::info!("[Relay] 配置 enabled=true，启动中继双回路 {}", cfg.url);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let app2 = app.clone();
    let cfg2 = cfg.clone();
    let rx2 = shutdown_rx.clone();
    let relay_task = tauri::async_runtime::spawn(async move {
        run_relay_loop(
            cfg2,
            move |content| {
                let app = app2.clone();
                async move { handle_task(app, content).await }
            },
            rx2,
        )
        .await;
    });

    let tunnel_task = tauri::async_runtime::spawn(async move {
        run_tunnel_loop(cfg, shutdown_rx).await;
    });

    *guard = Some(RelayHandle {
        shutdown_tx,
        tasks: vec![relay_task, tunnel_task],
    });
}

/// 优雅退出双回路：发 shutdown，等任务退出后状态复位 Disabled。
/// 调用方：relay_client_set_enabled（关闭中继）。RelayHandle 持有的 shutdown_tx 同时是
/// 回路 watch 通道的保活锚点（sender drop 会让所有 changed() 立即返回，回路会被误判
/// 为收到关停信号而退出）。
pub async fn stop_relay_loops() {
    let handle = {
        let mut guard = RELAY_HANDLE.write().unwrap_or_else(|e| e.into_inner());
        guard.take()
    };
    let Some(handle) = handle else { return };
    let _ = handle.shutdown_tx.send(true);
    for task in handle.tasks {
        let _ = task.await;
    }
    set_channel_state(Chan::Relay, ChannelState::Disabled);
    set_channel_state(Chan::Tunnel, ChannelState::Disabled);
    tracing::info!("[Relay] 中继双回路已优雅退出");
}

/// 退避等待或收到关停信号。返回 true = 收到关停（调用方应优雅返回）。
async fn sleep_or_shutdown(
    delay: Duration,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown.changed() => true,
    }
}

/// 核心循环：连接 → 收任务 → handler 执行 → 回传 → 断线重连。
/// handler 可注入（生产=submit_user_message，测试=echo）。
/// 重连走统一状态机：Network 3s×2→60s ±20% 抖动；Config 60s 慢档；
/// 成功连接重置回快档；探测永不停止。
/// shutdown：套餐降级等外部关停信号——退避 sleep 与长连接期间均监听，收到即优雅返回。
pub async fn run_relay_loop<F, Fut>(
    cfg: RelayClientConfig,
    handler: F,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let url = ws_url(&cfg);
    let token = cfg.token.clone();
    tracing::info!("[Relay] 客户端启动，连接 {}", url);
    set_channel_state(
        Chan::Relay,
        ChannelState::Retrying {
            since: now_unix(),
            attempts: 0,
        },
    );
    let mut backoff = RelayBackoff::new();
    let mut fault_logged = false;

    loop {
        if *shutdown.borrow() {
            tracing::info!("[Relay] 收到关停信号，任务通道回路退出");
            return;
        }
        // 连接期同样监听关停：取消 connect_once future（丢弃即关闭 WS/TCP）后立即返回，
        // 避免降 free 后长连接挂着不断。
        let result = tokio::select! {
            r = connect_once(&url, &token, &handler) => r,
            _ = shutdown.changed() => {
                tracing::info!("[Relay] 收到关停信号，任务通道回路退出");
                return;
            }
        };
        match result {
            Ok(()) => {
                // 连接曾建立并正常断开 → 重置退避（修复：旧版只增不减），断开按 Network 登记
                backoff.on_success();
                fault_logged = false;
                let delay = register_failure(
                    Chan::Relay,
                    "[Relay]",
                    &mut backoff,
                    &mut fault_logged,
                    FailKind::Network,
                    "连接已断开（服务器关闭）",
                );
                if sleep_or_shutdown(delay, &mut shutdown).await {
                    tracing::info!("[Relay] 收到关停信号，任务通道回路退出");
                    return;
                }
            }
            Err(e) => {
                if e.was_connected() {
                    backoff.on_success();
                    fault_logged = false;
                }
                let kind = e.kind();
                let summary = e.summary();
                let delay = register_failure(
                    Chan::Relay,
                    "[Relay]",
                    &mut backoff,
                    &mut fault_logged,
                    kind,
                    &summary,
                );
                if sleep_or_shutdown(delay, &mut shutdown).await {
                    tracing::info!("[Relay] 收到关停信号，任务通道回路退出");
                    return;
                }
            }
        }
    }
}

async fn connect_once<F, Fut>(url: &str, token: &str, handler: &F) -> Result<(), ConnectError>
where
    F: Fn(String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
{
    let ws = connect_ws_with_keepalive(url, token).await?;
    tracing::info!("[Relay] 已连接中继");
    set_channel_state(Chan::Relay, ChannelState::Connected);
    let (mut write, mut read) = ws.split();

    loop {
        let msg = match read.next().await {
            Some(Ok(m)) => m,
            Some(Err(e)) => return Err(ConnectError::Runtime(format!("WS 读错误: {e}"))),
            None => return Err(ConnectError::Runtime("connection closed".into())),
        };
        match msg {
            WsMessage::Text(text) => {
                let parsed: Option<ServerMsg> = serde_json::from_str(&text).ok();
                match parsed {
                    Some(ServerMsg::Task { task_id, content }) => {
                        tracing::info!(
                            "[Relay] 收到任务 {} ({} 字符)",
                            task_id,
                            content.chars().count()
                        );
                        match handler(content).await {
                            Ok(result) => {
                                let reply = DeviceMsg::Result {
                                    task_id,
                                    content: result,
                                };
                                let _ = write
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&reply).unwrap_or_default(),
                                    ))
                                    .await;
                            }
                            Err(e) => {
                                let reply = DeviceMsg::Error {
                                    task_id,
                                    message: e,
                                };
                                let _ = write
                                    .send(WsMessage::Text(
                                        serde_json::to_string(&reply).unwrap_or_default(),
                                    ))
                                    .await;
                            }
                        }
                    }
                    None => {
                        // 未知消息：静默忽略，避免 warn 刷屏
                        tracing::debug!("[Relay] 收到未知消息: {}", &text[..text.len().min(200)]);
                    }
                }
            }
            WsMessage::Close(_) => return Err(ConnectError::Runtime("server closed".into())),
            WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Binary(_) => {}
            _ => {}
        }
    }
}

// ── 生产 handler：复用共享入口，source="relay" ─────────────────────────

pub async fn handle_task<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    content: String,
) -> Result<String, String> {
    let state = app.state::<crate::state::AppState>();
    let resp = crate::commands::process::submit_user_message(
        app.clone(),
        state.inner(),
        content,
        None, // images
        None, // history
        None, // relation（无配置通道时用缓存兜底）
        None, // mode（默认）
        None, // references
        None, // send_id
        Some("relay".to_string()),
    )
    .await?;

    if resp.success {
        // ProcessInputResponse.message 为最终回复文本；空则任务已执行完成
        let result = if resp.message.trim().is_empty() {
            "done".to_string()
        } else {
            resp.message
        };
        Ok(result)
    } else {
        Err("task failed".to_string())
    }
}

// ── 隧道：外网手机访问桌面 mobile_server（TCP-over-WS）─────────────────────
// 桌面连中继 /ws/tunnel，中继把公网隧道端口（默认 18081）的字节流经本连接
// 转发；本地投递到 127.0.0.1:18772（mobile_server）。手机外网访问
// http://<中继IP>:18081/ 即等于访问桌面 mobile_server——页面/API/WS 全通，
// 恢复「手机 = 第二块屏」的完整体验（不再走单条 POST /task 消息通道）。

/// 本地 mobile_server 地址（桌面 Nuphus 的移动端服务）
const TUNNEL_LOCAL_ADDR: &str = "127.0.0.1:18772";

/// 隧道帧协议（与 relay-server 对齐）
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TunnelFrame {
    Open { tunnel_id: String },
    Ready { tunnel_id: String },
    Error { tunnel_id: String, message: String },
    Data { tunnel_id: String, data: String },
    Close { tunnel_id: String },
}

/// 获取桌面局域网 IPv4 地址（UDP 路由探测法，不发送实际流量）。
/// 用于隧道握手上报 lan_url：手机同一 WiFi 下被隧道断线窗口困住时，
/// 中继离线页据此注入「同一 WiFi 下直连电脑」入口，点击即可切到免费局域网直连。
fn local_lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("8.8.8.8", 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_ipv4() && !ip.is_loopback() {
        Some(ip.to_string())
    } else {
        None
    }
}

/// 隧道循环：连中继隧道 WS，断线走统一状态机重连（Network 3s×2→60s 抖动 /
/// Config 60s 慢档，探测永不停止）。
/// 用 tokio::spawn 包裹每次连接：即使 connect_tunnel_once 内部 panic（如 Mutex
/// poisoning），外层也能捕获 JoinError 并继续重连——否则一次 panic 会让整个
/// 隧道任务死亡，手机端永久失联（实测：并发压力下隧道掉线后无重连）。
/// shutdown：套餐降级等外部关停信号——退避 sleep 与连接期间均监听
/// （连接任务 abort 触发 WS/TCP 关闭），收到即优雅返回。
pub async fn run_tunnel_loop(
    cfg: RelayClientConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let token = cfg.token.clone();
    tracing::info!("[Relay-Tunnel] 隧道客户端启动，连接 {}", cfg.url);
    set_channel_state(
        Chan::Tunnel,
        ChannelState::Retrying {
            since: now_unix(),
            attempts: 0,
        },
    );
    let mut backoff = RelayBackoff::new();
    let mut fault_logged = false;
    loop {
        if *shutdown.borrow() {
            tracing::info!("[Relay-Tunnel] 收到关停信号，隧道回路退出");
            return;
        }
        // lan_url 供离线页注入局域网直连入口（URL query 中 ':' '/' 合法，无需百分号编码）。
        // 每次重连重新探测：桌面 WiFi 重连 IP 变化后，中继侧 device_lan_urls 缓存跟随更新
        // （此前在循环外构建一次，IP 变化后旧地址残留，手机同一 WiFi 也探测失败）。
        let url = {
            let mut url = format!(
                "{}/ws/tunnel?device_id={}",
                cfg.url.trim_end_matches('/'),
                cfg.device_id,
            );
            let port = TUNNEL_LOCAL_ADDR.rsplit(':').next().unwrap_or("18772");
            if let Some(ip) = local_lan_ip() {
                url.push_str(&format!("&lan_url=http://{}:{}", ip, port));
            }
            url
        };
        let token = token.clone();
        let mut join = tokio::task::spawn(async move { connect_tunnel_once(&url, &token).await });
        let result = tokio::select! {
            r = &mut join => r,
            _ = shutdown.changed() => {
                // abort 取消连接任务（丢弃 WS 即关闭 TCP），确认收口后优雅返回
                join.abort();
                let _ = join.await;
                tracing::info!("[Relay-Tunnel] 收到关停信号，隧道回路退出");
                return;
            }
        };
        let (kind, summary) = match &result {
            Ok(Ok(())) => {
                // 连接曾建立后断开 → 重置退避，断开按 Network 登记
                backoff.on_success();
                fault_logged = false;
                (FailKind::Network, "隧道连接已断开".to_string())
            }
            Ok(Err(e)) => {
                if e.was_connected() {
                    backoff.on_success();
                    fault_logged = false;
                }
                (e.kind(), e.summary())
            }
            Err(e) => (
                classify_fallback(&e.to_string()),
                format!("隧道任务异常退出（panic）: {e}"),
            ),
        };
        let delay = register_failure(
            Chan::Tunnel,
            "[Relay-Tunnel]",
            &mut backoff,
            &mut fault_logged,
            kind,
            &summary,
        );
        if sleep_or_shutdown(delay, &mut shutdown).await {
            tracing::info!("[Relay-Tunnel] 收到关停信号，隧道回路退出");
            return;
        }
    }
}

async fn connect_tunnel_once(url: &str, token: &str) -> Result<(), ConnectError> {
    let ws = connect_ws_with_keepalive(url, token).await?;
    tracing::info!("[Relay-Tunnel] 隧道已连接中继");
    set_channel_state(Chan::Tunnel, ChannelState::Connected);

    let (mut write, mut read) = ws.split();
    // 统一发帧通道：各 TCP 读任务 → frame_tx → writer → WS（避免并发写 SplitSink）。
    // 用 unbounded：frame_tx.send 绝不阻塞。否则并发隧道下 Data 帧塞满有界队列后，
    // 新隧道的 Ready 帧也发不出去 → VPS 等待 10s 超时关闭 → 手机大资源请求全失败
    // （实测：并发 9 请求 6 超时；VPS 日志大量"设备未就绪，关闭"）。
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // 写失败信号：writer 的 write.send 失败（隧道 WS 写方向半死）时置 true。
    // 读循环据此退出 → 关闭底层连接 → run_tunnel_loop 触发重连。
    // 否则 writer 死后读循环继续等 Open 帧，ready 发不出去 → VPS 10s「设备未就绪」。
    let (write_fail_tx, mut write_fail_rx) = tokio::sync::watch::channel(false);
    let writer = tokio::spawn(async move {
        while let Some(text) = frame_rx.recv().await {
            // write.send 加超时：写方向半死时（对端不读/TCP 窗口满）send 会永久阻塞而非返回 Err，
            // write_fail 永不触发 → 隧道永不重连（实测 P0：断 WiFi 走蜂窝白屏，桌面「已投递本地」
            // 但 Ready 回不到中继）。5s 超时判定写方向半死，主动断开触发重连。
            match tokio::time::timeout(Duration::from_secs(5), write.send(WsMessage::Text(text)))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    let _ = write_fail_tx.send(true);
                    break;
                }
            }
        }
    });

    // 活跃隧道：tunnel_id → 本地 TCP 写通道（data 帧写入）
    // 用 unbounded：主循环 Data 分发不阻塞，单个慢隧道不会饿死其他并发隧道（手机并发加载页面）
    let tunnels: Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // 本地 TCP 读任务句柄（close 时 abort）
    let readers: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // 断开原因分级（供外层/运维区分网络 vs 对端关闭 vs 写半死）：
    // - None（EOF/Close 帧）＝对端（中继）主动关闭或链路断开
    // - Err(e)　＝ WS 读错误（TCP 层错误，多为网络抖动）
    // - write_fail ＝ 写方向半死（对端不读/TCP 窗口满，Ready/Data 回不到中继）
    let drop_reason: Option<String> = loop {
        // 读空闲看门狗：服务端每 10s 下发协议 Ping，60s 无任何帧 = 链路半开/
        // 对端读任务已死但 TCP 未断——强制断开走外层重连自愈（正常永不触发）。
        let mut idle_watchdog = tokio::time::interval(Duration::from_secs(TUNNEL_READ_IDLE_SECS));
        idle_watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        idle_watchdog.tick().await; // 消耗立即触发的首跳，从现在起每 60s 一跳
        tokio::select! {
            _ = idle_watchdog.tick() => {
                break Some(format!(
                    "隧道读空闲超时（{TUNNEL_READ_IDLE_SECS}s 无任何帧含服务端心跳），强制重连"
                ));
            }
            msg = read.next() => {
                let Some(msg) = msg else {
                    break Some("隧道 WS 被对端关闭（EOF/Close 帧）".to_string());
                };
                match msg {
                    Ok(WsMessage::Text(text)) => {
                        let parsed: Option<TunnelFrame> = serde_json::from_str(&text).ok();
                        match parsed {
                            Some(TunnelFrame::Open { tunnel_id }) => {
                                // 并发派发：Open 处理（connect + ready 回发）不阻塞主读循环。
                                // 否则并发隧道串行 await，N 个请求延迟叠加 N 倍，手机并发加载
                                // 页面资源时部分超时 → 黑屏（实测：并发 5 请求 4 成功 2.7~5.5s + 1 超时）。
                                let ftx = frame_tx.clone();
                                let tunnels = tunnels.clone();
                                let readers = readers.clone();
                                tokio::spawn(async move {
                                    spawn_local_tunnel(&tunnel_id, &ftx, &tunnels, &readers).await;
                                });
                            }
                            Some(TunnelFrame::Data { tunnel_id, data }) => {
                                let tx = { tunnels.lock().unwrap_or_else(|e| e.into_inner()).get(&tunnel_id).cloned() };
                                if let Some(tx) = tx {
                                    use base64::Engine as _;
                                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                                        // unbounded send 非阻塞：绝不因单隧道慢而阻塞整个 WS 读循环
                                        let _ = tx.send(bytes);
                                    }
                                }
                            }
                            Some(TunnelFrame::Close { tunnel_id }) => {
                                close_local_tunnel(&tunnel_id, &tunnels, &readers).await;
                            }
                            _ => {}
                        }
                    }
                    Ok(_) => {
                        // ping / pong / close 等非业务帧：忽略
                    }
                    Err(e) => {
                        break Some(format!("隧道 WS 读错误: {e}"));
                    }
                }
            }
            _ = write_fail_rx.changed() => {
                // writer 写失败（隧道 WS 写方向半死）：退出读循环关闭底层连接，触发重连
                break Some("隧道 WS 写方向半死（write 超时/失败）".to_string());
            }
        }
    };
    tracing::warn!(
        "[Relay-Tunnel] {}，触发重连",
        drop_reason.unwrap_or_else(|| "隧道 WS 读循环退出（未知原因）".to_string())
    );

    // 连接断开：清理所有本地隧道
    let ids: Vec<String> = tunnels
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect();
    for id in ids {
        close_local_tunnel(&id, &tunnels, &readers).await;
    }
    writer.abort();
    Ok(())
}

async fn spawn_local_tunnel(
    tunnel_id: &str,
    frame_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    tunnels: &Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>>,
    readers: &Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
) {
    use base64::Engine as _;
    let tunnel_id = tunnel_id.to_string();
    match tokio::net::TcpStream::connect(TUNNEL_LOCAL_ADDR).await {
        Ok(tcp) => {
            // TCP_NODELAY：本地回环转发禁用 Nagle，避免 HTTP 响应分块 40ms ACK 延迟累积
            let _ = tcp.set_nodelay(true);
            let (mut tcp_r, mut tcp_w) = tcp.into_split();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            tunnels
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(tunnel_id.clone(), tx);

            // 回 ready（链路抖动下单帧可能丢失，周期性重发直到隧道进入数据阶段或关闭：
            // 服务器就绪判定已放宽为「Ready 或 Data」，首个 Data 到达即放行，重发即可停止）
            let (ready_stop_tx, mut ready_stop_rx) = tokio::sync::watch::channel(false);
            let ftx = frame_tx.clone();
            let tid = tunnel_id.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            let ready = serde_json::to_string(&TunnelFrame::Ready {
                                tunnel_id: tid.clone(),
                            })
                            .unwrap_or_default();
                            if ftx.send(ready).is_err() {
                                break;
                            }
                        }
                        _ = ready_stop_rx.changed() => break,
                    }
                }
            });

            // TCP 读 → data 帧
            let ftx = frame_tx.clone();
            let tid = tunnel_id.clone();
            let ready_stop_tx2 = ready_stop_tx.clone();
            let reader = tokio::spawn(async move {
                let mut buf = [0u8; 16384];
                let mut sent_any = false;
                loop {
                    match tcp_r.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                            let frame = serde_json::to_string(&TunnelFrame::Data {
                                tunnel_id: tid.clone(),
                                data,
                            })
                            .unwrap_or_default();
                            if ftx.send(frame).is_err() {
                                break;
                            }
                            if !sent_any {
                                sent_any = true;
                                // 已发出首个 Data：服务器收到设备 Data 即放行（就绪判定已放宽为
                                // 「Ready 或 Data」），Ready 重发可停止——覆盖「服务器零回传」隧道
                                // （TCP RST / 空响应等），避免 Ready 重发任务无限驻留（此前仅在
                                // 收到回传数据时才停止，零回传隧道每 1s 持续发 Ready 直到隧道关闭）。
                                let _ = ready_stop_tx2.send(true);
                            }
                        }
                    }
                }
                // EOF：通知中继关闭
                let close = serde_json::to_string(&TunnelFrame::Close { tunnel_id: tid })
                    .unwrap_or_default();
                let _ = ftx.send(close);
            });
            readers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(tunnel_id.clone(), reader);

            // data 帧 → TCP 写
            tokio::spawn(async move {
                let mut first = true;
                while let Some(bytes) = rx.recv().await {
                    if tcp_w.write_all(&bytes).await.is_err() {
                        break;
                    }
                    if first {
                        first = false;
                        // 首个数据帧已转发 = 服务器已放行隧道（收到 Data 即视为就绪），Ready 重发可停止
                        let _ = ready_stop_tx.send(true);
                    }
                }
                let _ = tcp_w.shutdown().await;
            });

            tracing::info!(
                "[Relay-Tunnel] 隧道 {} 已投递本地 {}",
                tunnel_id,
                TUNNEL_LOCAL_ADDR
            );
        }
        Err(e) => {
            tracing::warn!(
                "[Relay-Tunnel] 隧道 {} 本地连接失败（{} 未启动?）: {}",
                tunnel_id,
                TUNNEL_LOCAL_ADDR,
                e
            );
            let err = serde_json::to_string(&TunnelFrame::Error {
                tunnel_id,
                message: format!("local connect failed: {e}"),
            })
            .unwrap_or_default();
            let _ = frame_tx.send(err);
        }
    }
}

async fn close_local_tunnel(
    tunnel_id: &str,
    tunnels: &Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>>,
    readers: &Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
) {
    let tid = tunnel_id.to_string();
    tunnels
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&tid);
    if let Some(r) = readers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&tid)
    {
        r.abort();
    }
    tracing::debug!("[Relay-Tunnel] 隧道 {} 已关闭", tid);
}

// ── 测试：连真实中继端到端验证（echo handler） ─────────────────────────
// 运行：cargo test -p nuphus --test relay_e2e -- --ignored --nocapture
// 前置：VPS 中继运行中；配置 relay_client.json（或环境变量覆盖）

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn e2e_echo_via_real_relay() {
        let cfg = RelayClientConfig {
            enabled: true,
            url: std::env::var("RELAY_TEST_URL")
                .unwrap_or_else(|_| "ws://relay.example.com:18080".into()),
            device_id: std::env::var("RELAY_TEST_DEVICE")
                .unwrap_or_else(|_| "relay-client-test".into()),
            token: std::env::var("RELAY_TEST_DEVICE_TOKEN").unwrap_or_default(),
            caller_token: std::env::var("RELAY_TEST_CALLER_TOKEN").unwrap_or_default(),
            public_url: String::new(),
        };
        assert!(!cfg.token.is_empty(), "need RELAY_TEST_DEVICE_TOKEN");

        let handler = |content: String| async move { Ok(format!("echo:{}", content)) };
        let cfg2 = cfg.clone();
        let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        let loop_task =
            tokio::spawn(async move { run_relay_loop(cfg2, handler, shutdown_rx).await });

        // 等待连接建立（sleep 模拟）
        tokio::time::sleep(Duration::from_secs(3)).await;

        let base = cfg.url.trim_end_matches('/');
        let http_base = base.replacen("ws://", "http://", 1);
        let call_token = std::env::var("RELAY_TEST_CALLER_TOKEN").unwrap_or_default();
        let resp = reqwest::Client::new()
            .post(format!("{}/task", http_base))
            .header("X-Relay-Token", call_token)
            .json(&serde_json::json!({
                "device_id": cfg.device_id,
                "content": "hello-e2e"
            }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("POST /task 失败");

        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        loop_task.abort();
        assert_eq!(status, 200, "期望 200，实际 {} body={}", status, body);
        assert!(body.contains("echo:hello-e2e"), "结果未回传: {}", body);
        println!("E2E_OK status={} body={}", status, body);
    }
}
// ── 重连状态机单测（纯函数，不起真中继） ─────────────────────────────

#[cfg(test)]
mod backoff_tests {
    use super::*;

    fn http_err(status: u16) -> tungstenite::Error {
        tungstenite::Error::Http(
            http::Response::builder()
                .status(status)
                .body(None::<Vec<u8>>)
                .unwrap(),
        )
    }

    #[test]
    fn classify_http_4xx_is_config() {
        for s in [400u16, 401, 403, 404, 429] {
            assert_eq!(
                classify_connect_error(&http_err(s)),
                FailKind::Config,
                "status {s} 应为 Config"
            );
        }
    }

    #[test]
    fn classify_http_5xx_is_network() {
        for s in [500u16, 502, 503] {
            assert_eq!(
                classify_connect_error(&http_err(s)),
                FailKind::Network,
                "status {s} 应为 Network"
            );
        }
    }

    #[test]
    fn classify_io_is_network() {
        let e = tungstenite::Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert_eq!(classify_connect_error(&e), FailKind::Network);
    }

    #[test]
    fn classify_fallback_string_is_network() {
        assert_eq!(classify_fallback("connection closed"), FailKind::Network);
        assert_eq!(
            classify_fallback("watchdog: no frame for 45s"),
            FailKind::Network
        );
        assert_eq!(classify_fallback("task panicked"), FailKind::Network);
    }

    #[test]
    fn loops_allowed_requires_enabled_complete_config() {
        let cfg = RelayClientConfig {
            enabled: true,
            url: "ws://relay.example:18080".into(),
            device_id: "dev-1".into(),
            token: "tok".into(),
            caller_token: String::new(),
            public_url: String::new(),
        };
        // enabled + 配置完整 → 允许（Pro 体系移除后，远程访问对所有配对设备免费）
        assert!(relay_loops_allowed(&cfg));

        let mut disabled = cfg.clone();
        disabled.enabled = false;
        assert!(!relay_loops_allowed(&disabled));

        let mut no_url = cfg.clone();
        no_url.url = String::new();
        assert!(!relay_loops_allowed(&no_url));
        let mut no_device = cfg.clone();
        no_device.device_id = String::new();
        assert!(!relay_loops_allowed(&no_device));
        let mut no_token = cfg.clone();
        no_token.token = String::new();
        assert!(!relay_loops_allowed(&no_token));
    }

    #[test]
    fn connect_error_variants() {
        let hs = ConnectError::Handshake(Box::new(http_err(401)));
        assert_eq!(hs.kind(), FailKind::Config);
        assert!(!hs.was_connected());
        assert_eq!(hs.summary(), "HTTP 401");

        let rt = ConnectError::Runtime("connection closed".into());
        assert_eq!(rt.kind(), FailKind::Network);
        assert!(rt.was_connected());

        let rq = ConnectError::Request("bad url".into());
        assert_eq!(rq.kind(), FailKind::Config);
        assert!(!rq.was_connected());
    }

    #[test]
    fn backoff_network_sequence_and_cap() {
        let mut b = RelayBackoff::new();
        assert_eq!(b.base_delay(), Duration::from_secs(1));
        for exp in [2u64, 4, 8, 16, 32, 60, 60] {
            b.on_failure(FailKind::Network);
            assert_eq!(b.base_delay(), Duration::from_secs(exp));
        }
        assert_eq!(b.fail_count, 7);
    }

    #[test]
    fn backoff_jitter_within_20_percent() {
        let mut b = RelayBackoff::new();
        for _ in 0..200 {
            let ms = b.next_delay().as_millis();
            assert!(
                (800..=1200).contains(&ms),
                "抖动越界: {ms}ms（基准 1000±20%）"
            );
        }
    }

    #[test]
    fn backoff_delay_never_exceeds_cap_after_jitter() {
        let mut b = RelayBackoff::new();
        for _ in 0..6 {
            b.on_failure(FailKind::Network);
        }
        b.on_failure(FailKind::Config); // 慢档基准 60s
        for _ in 0..200 {
            assert!(
                b.next_delay() <= Duration::from_secs(60),
                "抖动后必须仍 ≤60s（中继恢复后 60s 内重连的规格）"
            );
        }
    }

    #[test]
    fn backoff_config_goes_slow_lane_immediately() {
        let mut b = RelayBackoff::new();
        b.on_failure(FailKind::Config);
        assert_eq!(b.base_delay(), Duration::from_secs(60), "Config 应直接慢档");
        // 慢档期间网络失败不改变档位
        b.on_failure(FailKind::Network);
        assert_eq!(b.base_delay(), Duration::from_secs(60));
        assert_eq!(b.fail_count, 2);
    }

    #[test]
    fn backoff_on_success_resets_to_fast_lane() {
        let mut b = RelayBackoff::new();
        for _ in 0..6 {
            b.on_failure(FailKind::Network);
        }
        b.on_failure(FailKind::Config);
        assert_eq!(b.base_delay(), Duration::from_secs(60));
        b.on_success();
        assert_eq!(b.base_delay(), Duration::from_secs(1));
        assert_eq!(b.fail_count, 0);
        assert!(!b.slow);
    }
}

#[cfg(test)]
mod public_tunnel_url_tests {
    use super::*;

    const DEV: &str = "desktop-9f2c7a1e";

    fn cfg(enabled: bool, url: &str) -> RelayClientConfig {
        RelayClientConfig {
            enabled,
            url: url.to_string(),
            device_id: DEV.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn derives_http_from_ws_and_replaces_port() {
        // 基址保持裸 origin（消费方按字符串前缀拼接 REST/WS 路径，不得带 query）
        assert_eq!(
            public_tunnel_url(&cfg(true, "ws://relay.example.com:18080")),
            Some("http://relay.example.com:18081".to_string())
        );
    }

    #[test]
    fn derives_https_from_wss_and_keeps_host() {
        assert_eq!(
            public_tunnel_url(&cfg(true, "wss://relay.example.com:443")),
            Some("https://relay.example.com:18081".to_string())
        );
        // 无端口 / 尾部斜杠 / 路径段均不影响派生
        assert_eq!(
            public_tunnel_url(&cfg(true, "wss://relay.example.com/")),
            Some("https://relay.example.com:18081".to_string())
        );
        assert_eq!(
            public_tunnel_url(&cfg(true, "ws://relay.example.com:18080/api")),
            Some("http://relay.example.com:18081".to_string())
        );
    }

    #[test]
    fn explicit_public_url_takes_priority() {
        let mut c = cfg(true, "wss://relay.example.com");
        c.public_url = "https://r.example.com".to_string();
        assert_eq!(
            public_tunnel_url(&c),
            Some("https://r.example.com".to_string())
        );
        // 尾部斜杠会被修剪
        c.public_url = "https://r.example.com/".to_string();
        assert_eq!(
            public_tunnel_url(&c),
            Some("https://r.example.com".to_string())
        );
        // public_url 优先于 url 派生，即使 url 是坏 scheme
        c.public_url = "https://r.example.com".to_string();
        c.url = "http://bad-scheme".to_string();
        assert_eq!(
            public_tunnel_url(&c),
            Some("https://r.example.com".to_string())
        );
    }

    #[test]
    fn disabled_or_empty_url_yields_none() {
        assert_eq!(
            public_tunnel_url(&cfg(false, "ws://relay.example.com:18080")),
            None
        );
        assert_eq!(public_tunnel_url(&cfg(true, "")), None);
        assert_eq!(public_tunnel_url(&cfg(true, "   ")), None);
        // 非 ws/wss scheme 不派生（协议不明，宁缺毋滥）
        assert_eq!(
            public_tunnel_url(&cfg(true, "http://relay.example.com:18080")),
            None
        );
    }

    // ── 导航入口变体：携带 ?device=（多设备路由）────────────────────────

    #[test]
    fn entry_url_appends_device_to_derived_base() {
        assert_eq!(
            public_tunnel_entry_url(&cfg(true, "ws://relay.example.com:18080")),
            Some(format!("http://relay.example.com:18081/?device={DEV}"))
        );
        assert_eq!(
            public_tunnel_entry_url(&cfg(true, "wss://relay.example.com")),
            Some(format!("https://relay.example.com:18081/?device={DEV}"))
        );
    }

    #[test]
    fn entry_url_appends_device_to_explicit_public_url() {
        let mut c = cfg(true, "wss://relay.example.com");
        c.public_url = "https://r.example.com".to_string();
        assert_eq!(
            public_tunnel_entry_url(&c),
            Some(format!("https://r.example.com/?device={DEV}"))
        );
        // 尾部斜杠修剪后再追加
        c.public_url = "https://r.example.com/".to_string();
        assert_eq!(
            public_tunnel_entry_url(&c),
            Some(format!("https://r.example.com/?device={DEV}"))
        );
    }

    #[test]
    fn entry_url_verbatim_when_query_or_fragment_present() {
        let mut c = cfg(true, "wss://relay.example.com");
        c.public_url = "https://r.example.com/entry?x=1".to_string();
        assert_eq!(
            public_tunnel_entry_url(&c),
            Some("https://r.example.com/entry?x=1".to_string())
        );
        c.public_url = "https://r.example.com/page#frag".to_string();
        assert_eq!(
            public_tunnel_entry_url(&c),
            Some("https://r.example.com/page#frag".to_string())
        );
    }

    #[test]
    fn entry_url_without_device_id_stays_bare() {
        let mut c = cfg(true, "ws://relay.example.com:18080");
        c.device_id = String::new();
        assert_eq!(
            public_tunnel_entry_url(&c),
            Some("http://relay.example.com:18081".to_string())
        );
    }

    #[test]
    fn entry_url_none_when_disabled() {
        assert_eq!(
            public_tunnel_entry_url(&cfg(false, "ws://relay.example.com:18080")),
            None
        );
    }
}
