//! Nuphus 中继服务端——薄转发，不落盘消息内容。
//!
//! 设计原则（吸取 Gateway 教训）：
//! - 无状态：不存消息、不存会话历史，只做「身份校验 → 路由 → 回传」
//! - 在线设备表与任务等待表都在内存，进程重启即清空（客���端断线重连即可）
//! - 元数据可后续扩展（设备 ID、次数、字节数 → Token 聚合地基）
//!
//! 协议：
//! - GET /health                     → {"ok":true}
//! - GET /ws/device?device_id=&token= → 桌面出站 WS 长连接（设备鉴权）
//! - POST /task (X-Relay-Token)       → 外部调用方提交任务，转发设备并等待结果
//!
//! 消息协议（WS JSON，tag=type）：
//!   server → device: {"type":"task","task_id":"...","content":"..."}
//!   device → server: {"type":"result","task_id":"...","content":"..."}
//!                   | {"type":"error","task_id":"...","message":"..."}
//!
//! 鉴权（token 解析优先级：文件 > 环境变量，改文件即热轮换，无需重启）：
//!   RELAY_DEVICE_TOKEN / {data_dir}/relay_device.token     设备 WS 连接 token
//!   RELAY_CALLER_TOKEN / {data_dir}/relay_caller.token     外部调用方 token（POST /task）
//!   MVP 可两者设同一值；缺省空串则拒绝一切请求（防止裸奔）。
//!
//! WS 鉴权三通道（任选其一，防止 token 落 URL/日志）：
//!   1. Authorization: Bearer <token>
//!   2. Sec-WebSocket-Protocol: auth.<token>
//!   3. query ?token= <token>（兼容旧客户端）
//!
//! 安全加固（防公网暴露下的 DoS / 扫描 / 暴力探测）：
//! - HTTP 中间件：IP 速率限制 + 结构化访问日志
//! - 隧道 18081：每 IP 新建连接限速 / 全局并发上限 / 单 IP 并发上限 / 空闲超时
//!
//! 隧道门禁（2026-08 起：Pro 体系已移除，远程访问对所有配对设备免费开放）：
//! - 公网隧道入口（18081）不再按套餐拦截，只按隧道链路在线状态分流：
//!     链路在线                     → 照常转发
//!     链路离线但设备通道在线       → 回重试页（RETRY_HTML，语义「连接中」）
//!     链路离线且设备通道也不在     → 回设备离线页（OFFLINE_HTML）
//!   两页均注入局域网直连入口（设备自报 lan_url），同 WiFi 手机自动切直连。
//! - 滥用防护由基础设施承担：IP 速率限制 + 隧道并发上限 + idle 超时。
//! - 静态页为完整 HTTP/1.1 响应直写 TCP 流；中继是薄转发，不解析手机发来的请求内容。

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Query, Request, State,
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};

const TASK_TIMEOUT_SECS: u64 = 120;
const MAX_CONTENT_BYTES: usize = 512 * 1024; // 单条任务内容上限 512KB

// ── 安全加固参数（防公网暴露 DoS / 扫描） ────────────────────────────────
/// 隧道：每 IP 每分钟最多新建连接数（移动端 SPA 首载 + WS 重连 + 正常操作，20 过严会误杀）
const TUNNEL_RATE_PER_IP_MIN: usize = 60;
/// 隧道：全局并发连接上限（超过直接拒绝，防资源耗尽）
const TUNNEL_MAX_ACTIVE_TOTAL: usize = 256;
/// 隧道：单 IP 并发连接上限（移动端首载静态资源+API+WS长连接+keep-alive 峰值约 15-20，8 过严误杀）
const TUNNEL_MAX_ACTIVE_PER_IP: usize = 32;
/// 隧道：空闲（无任何数据帧）超时秒数，超时强制关闭。
/// 历史教训：曾降到 12s 防 HTTP 资源挂起，但桌面 WS 心跳间隔 15s（mobile_server
/// WS_HEARTBEAT_INTERVAL）> 12s → WS 隧道在心跳前被杀 → 手机「连接中」永不连上（实测 P0）。
/// 30s 权衡：WS 心跳 15s 安全（30s 内必有 pong 数据）；HTTP 资源挂起 30s 内 load-guard
/// 前端已提示（8s），30s 后隧道关闭触发浏览器报错，不会永久白屏。
const TUNNEL_IDLE_TIMEOUT_SECS: u64 = 30;
/// 隧道 Ready 等待超时：桌面「已投递本地」但 Ready/Data 回不到中继 = 写方向半死。
/// 原 10s 在链路抖动（国内↔境外 VPS TLS/WS 丢包重传）下会误杀正常隧道——Ready 单帧
/// 丢失后服务器空等 10s 判定半死 → 强制 reset → 桌面重连循环（实测 2026-08-19）。
/// 20s 给丢包重传留足余量；就绪判定已放宽为「Ready 或 Data」兜底（见 handle_tunnel_conn）。
const TUNNEL_READY_TIMEOUT_SECS: u64 = 20;
/// 隧道写超时：ws_tx.send 在写方向半死时永久阻塞（对端不读/TCP 窗口满），
/// 5s 超时判定半死主动断开隧道 WS，触发桌面重连自愈（对齐桌面 relay_client 5s 写看门狗）。
const TUNNEL_WRITE_TIMEOUT_SECS: u64 = 5;
/// HTTP：每 IP 每分钟请求上限（health 豁免——健康检查应始终可答）
const HTTP_RATE_PER_IP_MIN: usize = 120;
/// HTTP：限流窗口（秒）
const RATE_WINDOW_SECS: u64 = 60;

/// 获取 std::Mutex 守卫，锁中毒时恢复（into_inner）而非 panic 整进程。
/// 本服务所有 Mutex 保护的都是 map/计数器状态（无跨字段不变量），持锁 panic 后
/// 数据仍自洽，恢复继续服务是幂等安全的；公网中继任一锁中毒即崩溃不可接受。
/// （对齐桌面端 src/mobile_append.rs 的 poison 恢复模式）
fn lock_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── 状态 ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    /// 在线设备：device_id → (连接实例 ID, 向设备 WS 发送消息的 channel)。
    /// 带连接实例 ID：连接退出时只删自己的条目，避免旧连接清理误删新连接注册
    /// （审计 R1.2：隧道路径已修，设备路径同 bug 类——桌面快速重连时旧连接清理
    ///  会删掉新连接刚注册的 dev_tx → POST /task 误报 device offline）。
    devices: Arc<Mutex<HashMap<String, (u64, mpsc::Sender<ServerMsg>)>>>,
    /// 任务等待表：task_id → 结果回传通道
    tasks: Arc<Mutex<HashMap<String, oneshot::Sender<Result<TaskResult, String>>>>>,
    /// 隧道 WS 连接：device_id → (连接实例 ID, 向设备隧道 WS 发送帧的 channel)（外网手机访问桌面 mobile_server）
    /// 无界：Data 帧 send 非阻塞，避免有界背压阻塞共享读循环饿死并发隧道。
    /// 带连接实例 ID：连接退出时只删自己的条目，避免旧连接清理误删新连接注册
    /// （实测 P0：桌面重连后旧连接退出无条件 remove，把新连接的链路注册删掉 →
    ///  连接实际在线但门禁查不到链路 → 手机持续收到「设备离线」页）。
    tunnel_links: Arc<Mutex<HashMap<String, (u64, mpsc::UnboundedSender<TunnelFrame>)>>>,
    /// 隧道强制重置信号：device_id → (连接实例 ID, 触发该隧道 WS 主动断开重连的 sender)。
    /// 半死自愈：中继 ready 超时（设备未就绪）说明隧道写方向半死，发信号断开隧道 WS，
    /// 桌面 read 返回 Err 后经统一状态机重连，新连接恢复正常（否则半死连接永不重连）。
    /// 带连接实例 ID：连接退出时只删自己的条目，避免旧连接清理误删新连接注册（多连接切换竞态）。
    tunnel_reset: Arc<Mutex<HashMap<String, (u64, mpsc::UnboundedSender<()>)>>>,
    /// 隧道发起连接归属：tunnel_id → 发起该隧道时的隧道 WS 连接 reset sender。
    /// ready 超时只 reset 发起连接（旧连接已断则 send 失败无害），
    /// 避免误伤切换后的新连接（实测 P0：旧连接半死超时后新连接刚 online 就被残留 reset 误杀，
    /// 引发重连风暴 → 手机持续看到「设备离线」页）。
    tunnel_owner: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<()>>>>,
    /// 活跃隧道：tunnel_id → 公网连接侧接收设备帧的 channel
    /// 无界：设备 WS 读循环 Data 帧 send 非阻塞，多隧道并发互不饿死。
    tunnels: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TunnelFrame>>>>,
    // ── 安全加固状态 ──
    /// HTTP 速率限制：IP → 窗口内请求时间戳（滑动窗口）
    http_hits: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    /// 隧道新建连接限流：IP → 窗口内连接时间戳
    tunnel_rate: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    /// 当前全局活跃隧道连接数
    tunnel_active: Arc<Mutex<usize>>,
    /// 每 IP 当前活跃隧道连接数
    tunnel_active_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
    /// 设备自报局域网直连地址：device_id → "http://<IP>:<port>"（隧道握手时写入；
    /// 断开不删，保留 last-known 供离线页注入「同一 WiFi 直连」入口；缺省 None = 旧客户端）
    device_lan_urls: Arc<Mutex<HashMap<String, String>>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            tunnel_links: Arc::new(Mutex::new(HashMap::new())),
            tunnel_reset: Arc::new(Mutex::new(HashMap::new())),
            tunnel_owner: Arc::new(Mutex::new(HashMap::new())),
            tunnels: Arc::new(Mutex::new(HashMap::new())),
            http_hits: Arc::new(Mutex::new(HashMap::new())),
            tunnel_rate: Arc::new(Mutex::new(HashMap::new())),
            tunnel_active: Arc::new(Mutex::new(0)),
            tunnel_active_ip: Arc::new(Mutex::new(HashMap::new())),
            device_lan_urls: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

// ── 安全：token 解析（文件 > 环境变量，改文件即热轮换） ───────────────────
/// 数据目录（token 文件位置）：优先 RELAY_DATA_DIR，否则当前目录
fn data_dir() -> std::path::PathBuf {
    std::env::var("RELAY_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// 读取 token：优先 {data_dir}/{file}，否则环境变量 {env_key}；两者皆空返回空串
fn load_token(file: &str, env_key: &str) -> String {
    let from_file = std::fs::read_to_string(data_dir().join(file))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(t) = from_file {
        return t;
    }
    std::env::var(env_key).unwrap_or_default()
}

fn device_token() -> String {
    load_token("relay_device.token", "RELAY_DEVICE_TOKEN")
}

fn caller_token() -> String {
    load_token("relay_caller.token", "RELAY_CALLER_TOKEN")
}

fn token_ok(provided: &str, expected: &str) -> bool {
    !expected.is_empty() && provided == expected
}

/// WS 鉴权 token 提取（三通道，防 token 落 URL/日志）：
/// 1. Authorization: Bearer <token>   —— 桌面 Rust 客户端优先
/// 2. Sec-WebSocket-Protocol: auth.<token> —— 浏览器可传 protocols
/// 3. query ?token= <token>           —— 兼容旧客户端
fn extract_ws_token(headers: &HeaderMap, query_token: &str) -> String {
    if let Some(v) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(t) = s.strip_prefix("Bearer ") {
                let t = t.trim();
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
    }
    if let Some(v) = headers.get("sec-websocket-protocol") {
        if let Ok(s) = v.to_str() {
            for proto in s.split(',') {
                if let Some(t) = proto.trim().strip_prefix("auth.") {
                    let t = t.trim();
                    if !t.is_empty() {
                        return t.to_string();
                    }
                }
            }
        }
    }
    query_token.to_string()
}

/// 提取客户端请求的 WS 子协议（auth.<token>），用于握手回显——浏览器 WebSocket 要求
/// 服务器必须从请求的 protocols 中回显一个，否则握手失败。
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

// ── 安全：速率限制与访问日志 ──────────────────────────────────────────────

/// 滑动窗口速率检查：record 返回 true 表示在限额内（并记录本次）
fn rate_check(map: &Mutex<HashMap<IpAddr, Vec<Instant>>>, ip: IpAddr, limit: usize) -> bool {
    let mut guard = lock_recover(&map);
    let now = Instant::now();
    let cutoff = now - Duration::from_secs(RATE_WINDOW_SECS);
    let hits = guard.entry(ip).or_default();
    // 清理窗口外的时间戳
    hits.retain(|t| *t > cutoff);
    if hits.len() >= limit {
        return false;
    }
    hits.push(now);
    true
}

/// 从请求取对端 IP（ConnectInfo 扩展，由 into_make_service_with_connect_info 注入）
fn peer_ip(req: &Request) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
}

/// HTTP 安全中间件：访问日志 + IP 速率限制（/health 豁免限流，保证健康检查可达）
async fn security_mw(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let ip = peer_ip(&req).unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let is_health = path == "/health";

    if !is_health && !rate_check(&state.http_hits, ip, HTTP_RATE_PER_IP_MIN) {
        tracing::warn!("[relay] rate limited: {} {} {}", ip, method, path);
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    // 仅记录非 health 请求（health 会被监控高频探测，记录即噪声）
    if !is_health {
        tracing::info!("[relay] http {} {} -> {} (ip={})", method, path, status, ip);
    }
    resp
}

// ── 协议 ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Task { task_id: String, content: String },
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeviceMsg {
    Result { task_id: String, content: String },
    Error { task_id: String, message: String },
}

/// 隧道帧协议（/ws/tunnel）：外网手机 → 中继 → 桌面 mobile_server 的字节隧道。
/// 帧类型：open（服务端→设备，新连接）/ ready / error / data（base64）/ close。
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TunnelFrame {
    Open { tunnel_id: String },
    Ready { tunnel_id: String },
    Error { tunnel_id: String, message: String },
    Data { tunnel_id: String, data: String },
    Close { tunnel_id: String },
}

#[derive(Deserialize)]
struct TaskPayload {
    device_id: String,
    content: String,
}

#[derive(Serialize)]
struct TaskOk {
    task_id: String,
    result: String,
}

#[derive(Serialize)]
struct TaskErr {
    task_id: String,
    error: String,
}

#[derive(Deserialize)]
struct DeviceQuery {
    device_id: String,
    /// query token 可选：主鉴权通道是 Authorization: Bearer（桌面客户端 / 浏览器子协议），
    /// query ?token= 仅为兼容旧客户端。若设为必需 String，仅用 Header 鉴权的客户端会因
    /// Query 提取失败被 axum 直接 400 拒绝（历史 bug：桌面 relay_client 连不上中继）。
    #[serde(default)]
    token: String,
    /// 设备自报局域网直连地址（http://<桌面IP>:<port>），隧道握手时缓存供离线页
    /// 注入「同一 WiFi 直连」入口；缺省空串=旧客户端（离线页不显示直连入口）
    #[serde(default)]
    lan_url: String,
}

// ── 鉴权 ───────────────────────────────────────────────��────────────────

// 鉴权实现已上移至「安全：token 解析」区（文件优先，热轮换）

// ── CORS ────────────────────────────────────────────────────────────────

/// CORS 中间件：手机 PWA（局域网 origin，如 http://192.168.1.x:18772）跨域
/// POST /task 到中继服务器必须通过浏览器预检。无 CORS 头时浏览器拦截响应：
/// iOS Safari fetch 抛 "Load failed"，Chrome 抛 "Failed to fetch"。
async fn cors_mw(req: Request, next: Next) -> Response {
    // 预检（OPTIONS）：直接回 204 + 允许头（中继只暴露 GET / POST / OPTIONS）
    if req.method() == Method::OPTIONS {
        return Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .header(
                "Access-Control-Allow-Headers",
                "Content-Type, X-Relay-Token, Authorization",
            )
            .header("Access-Control-Max-Age", "86400")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response());
    }
    // 正常请求：附加允许头（WS 升级为 Rust 客户端直连，不受浏览器 CORS 影响）
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
    resp
}

// ── Handlers ────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

/// POST /task — 外部调用方提交任务，转发给在线设备，等待结果。
async fn post_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TaskPayload>,
) -> Response {
    // 1. 鉴权
    let provided = headers
        .get("X-Relay-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !token_ok(provided, &caller_token()) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"forbidden"})),
        )
            .into_response();
    }

    // 2. 输入校验
    if payload.content.is_empty() || payload.content.len() > MAX_CONTENT_BYTES {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"bad content"})),
        )
            .into_response();
    }

    // 3. 查找在线设备
    let tx = {
        let devices = lock_recover(&state.devices);
        devices.get(&payload.device_id).map(|(_, tx)| tx.clone())
    };
    let Some(tx) = tx else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"device offline"})),
        )
            .into_response();
    };

    // 4. 创建任务等待通道
    let task_id = new_task_id(&payload.device_id);
    let (res_tx, res_rx) = oneshot::channel();
    lock_recover(&state.tasks).insert(task_id.clone(), res_tx);

    // 5. 转发给设备
    if tx
        .send(ServerMsg::Task {
            task_id: task_id.clone(),
            content: payload.content.clone(),
        })
        .await
        .is_err()
    {
        lock_recover(&state.tasks).remove(&task_id);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"device disconnected"})),
        )
            .into_response();
    }

    // 6. 等待结果（超时）
    match tokio::time::timeout(std::time::Duration::from_secs(TASK_TIMEOUT_SECS), res_rx).await {
        Ok(Ok(Ok(result))) => {
            lock_recover(&state.tasks).remove(&task_id);
            (
                StatusCode::OK,
                Json(TaskOk {
                    task_id,
                    result: result.content,
                }),
            )
                .into_response()
        }
        Ok(Ok(Err(msg))) => {
            lock_recover(&state.tasks).remove(&task_id);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(TaskErr {
                    task_id,
                    error: msg,
                }),
            )
                .into_response()
        }
        _ => {
            lock_recover(&state.tasks).remove(&task_id);
            (
                StatusCode::GATEWAY_TIMEOUT,
                Json(TaskErr {
                    task_id,
                    error: "device timeout".into(),
                }),
            )
                .into_response()
        }
    }
}

/// POST /admin/rotate-caller-token — caller_token 轮换（设备凭据鉴权）。
///
/// 安全模型：X-Relay-Token 用 **device_token** 校验（设备通道凭据，永不下发手机端），
/// 拥有 caller_token 的手机端无法调用此端点（权限隔离：调用方凭据不能轮换自己）。
/// 轮换写入 relay_caller.token 文件——load_token 每次请求重读文件，热生效免重启；
/// 旧 caller_token 即刻失效（所有已下发手机端的外网访问凭据作废）。
/// 响应仅此一次经 TLS 返回新 token，桌面端落 relay_client.json。
async fn post_rotate_caller_token(headers: HeaderMap) -> Response {
    let provided = headers
        .get("X-Relay-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !token_ok(provided, &device_token()) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        )
            .into_response();
    }

    // 生成 48 hex（对齐现有 caller_token 格式）：24 字节密码学随机
    use rand::RngCore;
    let mut buf = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut buf);
    let new_token: String = buf.iter().map(|b| format!("{b:02x}")).collect();

    let path = data_dir().join("relay_caller.token");
    if let Err(e) = std::fs::write(&path, &new_token) {
        tracing::error!("[relay] rotate caller token 写入失败: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "persist failed"})),
        )
            .into_response();
    }
    tracing::info!("[relay] caller_token 已轮换（设备发起）");
    Json(serde_json::json!({ "caller_token": new_token })).into_response()
}

/// GET /ws/device — 桌面出站 WS 长连接。
async fn ws_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DeviceQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let token = extract_ws_token(&headers, &q.token);
    if !token_ok(&token, &device_token()) {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    // 浏览器子协议鉴权：服务器必须回显所选协议，否则握手失败
    let ws = if let Some(p) = extract_ws_subprotocol(&headers) {
        ws.protocols([p])
    } else {
        ws
    };
    ws.on_upgrade(move |socket| handle_device_socket(state, socket, q.device_id))
}

async fn handle_device_socket(state: AppState, socket: WebSocket, device_id: String) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // 设备消息通道：POST /task 通过它给设备发任务
    let (dev_tx, mut dev_rx) = mpsc::channel::<ServerMsg>(64);
    static DEV_SEQ: AtomicU64 = AtomicU64::new(1);
    let conn_id = DEV_SEQ.fetch_add(1, Ordering::Relaxed);
    lock_recover(&state.devices).insert(device_id.clone(), (conn_id, dev_tx));
    tracing::info!("[relay] device online: {}", device_id);

    // 写端：消费 dev_rx（服务端→设备）。无心跳——桌面主动连，连接在即在线，
    // 断开由 TCP 语义自然感知，桌面断线自动重连。
    let writer = tokio::spawn(async move {
        while let Some(msg) = dev_rx.recv().await {
            let text = serde_json::to_string(&msg).unwrap_or_default();
            if ws_tx.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Text(text) => {
                let parsed: Option<DeviceMsg> = serde_json::from_str(&text).ok();
                match parsed {
                    Some(DeviceMsg::Result { task_id, content }) => {
                        if let Some(tx) = lock_recover(&state.tasks).remove(&task_id) {
                            let _ = tx.send(Ok(TaskResult { content }));
                        }
                    }
                    Some(DeviceMsg::Error { task_id, message }) => {
                        if let Some(tx) = lock_recover(&state.tasks).remove(&task_id) {
                            let _ = tx.send(Err(message));
                        }
                    }
                    None => {
                        tracing::warn!(
                            "[relay] device sent unparsable msg: {}",
                            &text[..text.len().min(200)]
                        );
                    }
                }
            }
            Message::Close(_) | Message::Ping(_) | Message::Pong(_) => {}
            _ => {}
        }
    }

    // 连接断开：只删自己的设备条目（旧连接清理不得误删新连接注册，审计 R1.2）
    {
        let mut guard = lock_recover(&state.devices);
        if let Some((id, _)) = guard.get(&device_id) {
            if *id == conn_id {
                guard.remove(&device_id);
            }
        }
    }
    writer.abort();
    tracing::info!("[relay] device offline: {}", device_id);
}

#[derive(Debug)]
struct TaskResult {
    content: String,
}

fn new_task_id(device_id: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}", device_id, ts)
}

// ── 隧道：外网手机访问桌面 mobile_server（TCP-over-WS）─────────────────────
// 手机端访问 RELAY_TUNNEL_PORT（默认 18081）→ 中继把字节流经设备隧道 WS 转发
// 给桌面 relay_client → 桌面投递到本地 127.0.0.1:18772（mobile_server）→ 回传。
// 中继保持薄转发：只转字节，不理解 HTTP/WS 业务，无状态。

/// GET /ws/tunnel — 设备建立隧道长连接（鉴权同 /ws/device）。
async fn ws_tunnel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DeviceQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let token = extract_ws_token(&headers, &q.token);
    if !token_ok(&token, &device_token()) {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    // 浏览器子协议鉴权：服务器必须回显所选协议，否则握手失败
    let ws = if let Some(p) = extract_ws_subprotocol(&headers) {
        ws.protocols([p])
    } else {
        ws
    };
    ws.on_upgrade(move |socket| handle_tunnel_socket(state, socket, q.device_id, q.lan_url))
}

async fn handle_tunnel_socket(
    state: AppState,
    socket: WebSocket,
    device_id: String,
    lan_url: String,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    // 写失败信号：writer 写超时/失败（写方向半死）时置 true，主读循环据此退出断开连接，
    // 触发桌面重连自愈（对齐桌面 relay_client 的 write_fail 看门狗）。
    let (write_fail_tx, mut write_fail_rx) = tokio::sync::watch::channel(false);

    // 缓存设备自报局域网直连地址（非空才写；断开不删除——离线页据此注入「同一 WiFi 直连」入口）
    if !lan_url.is_empty() {
        lock_recover(&state.device_lan_urls).insert(device_id.clone(), lan_url);
    }

    // 设备隧道发送通道：公网连接任务/本循环把帧发给设备
    static CONN_SEQ: AtomicU64 = AtomicU64::new(1);
    let conn_id = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<TunnelFrame>();
    lock_recover(&state.tunnel_links).insert(device_id.clone(), (conn_id, tx));
    // 注册强制重置信号：handle_tunnel_conn ready 超时（半死）时触发本连接断开重连
    let (reset_tx, mut reset_rx) = mpsc::unbounded_channel::<()>();
    lock_recover(&state.tunnel_reset).insert(device_id.clone(), (conn_id, reset_tx));
    tracing::info!("[relay] tunnel link online: {}", device_id);

    // 写端：消费 tx → 发设备；并每 10s 发协议层 Ping 保持隧道活跃。
    // 背景：桌面→中继隧道 WS 空闲时无业务帧，TCP keepalive 在云 LB/NAT 场景可能
    // 不刷新应用层空闲计时器 → 隧道被静默掐断 → 手机「设备离线」（实测间隔 1 分钟级
    // 断线）。协议层 Ping 产生应用层流量刷新 NAT/LB 表项；桌面 read 循环忽略
    // Ping/Pong 帧（relay_client Ok(_) 分支），无需改桌面。
    // 写超时看门狗：ws_tx.send 在写方向半死时（对端不读/TCP 窗口满）会永久阻塞而非
    // 返回 Err，导致 Open/Data/Ping 帧卡死、只能靠 ready 超时兜底（实测 Open 帧卡 12.5s）。
    // 5s 超时判定半死主动断开，触发桌面重连自愈。
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(10));
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // interval 首次 tick 立即触发：先消费一次，保证首个周期 10s 后才有 Ping
        ping.tick().await;
        loop {
            tokio::select! {
                frame = rx.recv() => {
                    let Some(frame) = frame else { break };
                    let text = serde_json::to_string(&frame).unwrap_or_default();
                    match tokio::time::timeout(
                        Duration::from_secs(TUNNEL_WRITE_TIMEOUT_SECS),
                        ws_tx.send(Message::Text(text.into())),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) | Err(_) => {
                            let _ = write_fail_tx.send(true);
                            break;
                        }
                    }
                }
                _ = ping.tick() => {
                    match tokio::time::timeout(
                        Duration::from_secs(TUNNEL_WRITE_TIMEOUT_SECS),
                        ws_tx.send(Message::Ping(Vec::new())),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) | Err(_) => {
                            let _ = write_fail_tx.send(true);
                            break;
                        }
                    }
                }
            }
        }
    });

    // 读端：设备回帧（ready/data/error/close）→ 路由给对应公网连接。
    // 同时监听 reset 信号：半死自愈——ready 超时后主动断开隧道 WS，桌面重连恢复。
    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                let Some(msg) = msg else { break };
                match msg {
                    Ok(Message::Text(text)) => {
                        let parsed: Option<TunnelFrame> = serde_json::from_str(&text).ok();
                        match parsed {
                            Some(TunnelFrame::Ready { tunnel_id }) => {
                                let t = {
                                    let guard = lock_recover(&state.tunnels);
                                    guard.get(&tunnel_id).cloned()
                                };
                                if let Some(t) = t {
                                    let _ = t.send(TunnelFrame::Ready { tunnel_id });
                                }
                            }
                            Some(TunnelFrame::Error { tunnel_id, message }) => {
                                let t = {
                                    let guard = lock_recover(&state.tunnels);
                                    guard.get(&tunnel_id).cloned()
                                };
                                if let Some(t) = t {
                                    let _ = t.send(TunnelFrame::Error { tunnel_id, message });
                                }
                            }
                            Some(TunnelFrame::Data { tunnel_id, data }) => {
                                let t = {
                                    let guard = lock_recover(&state.tunnels);
                                    guard.get(&tunnel_id).cloned()
                                };
                                if let Some(t) = t {
                                    let _ = t.send(TunnelFrame::Data { tunnel_id, data });
                                }
                            }
                            Some(TunnelFrame::Close { tunnel_id }) => {
                                let t = {
                                    let guard = lock_recover(&state.tunnels);
                                    guard.get(&tunnel_id).cloned()
                                };
                                if let Some(t) = t {
                                    let _ = t.send(TunnelFrame::Close { tunnel_id });
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(Message::Close(_) | Message::Ping(_) | Message::Pong(_)) => {}
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            _ = reset_rx.recv() => {
                tracing::warn!("[relay] tunnel link 收到强制重置信号，断开重连: {}", device_id);
                break;
            }
            _ = write_fail_rx.changed() => {
                tracing::warn!("[relay] tunnel link 写方向半死（write 超时/失败），断开重连: {}", device_id);
                break;
            }
        }
    }

    // 只删自己的链路条目：旧连接清理不得误删新连接注册（新连接已覆盖同 device_id 条目）
    {
        let mut guard = lock_recover(&state.tunnel_links);
        if let Some((id, _)) = guard.get(&device_id) {
            if *id == conn_id {
                guard.remove(&device_id);
            }
        }
    }
    // 只删自己的 reset 条目：旧连接清理不得误删新连接注册（新连接已覆盖同 device_id 条目）
    {
        let mut guard = lock_recover(&state.tunnel_reset);
        if let Some((id, _)) = guard.get(&device_id) {
            if *id == conn_id {
                guard.remove(&device_id);
            }
        }
    }
    // 僵尸隧道清理（借鉴 dsh-pocket「任一端断开清理另一端」）：本链路断开时，该设备
    // 的活跃隧道（tunnels 表，tunnel_id 以 `{device_id}-` 为前缀）无人再喂帧——
    // 公网侧 writer 的 rx.recv() 会永久悬挂。drop sender 触发连锁清理：
    // rx 得 None → writer 退出并 shutdown TCP → 公网读循环收 Ok(0) → 完整收口。
    // 不向设备发 Close（WS 已断，发帧无意义）。
    {
        let prefix = format!("{device_id}-");
        let orphan_ids: Vec<String> = {
            let guard = lock_recover(&state.tunnels);
            guard
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect()
        };
        if !orphan_ids.is_empty() {
            {
                let mut guard = lock_recover(&state.tunnels);
                for id in &orphan_ids {
                    guard.remove(id); // drop sender → 公网侧连锁清理
                }
            }
            let mut owner = lock_recover(&state.tunnel_owner);
            for id in &orphan_ids {
                owner.remove(id);
            }
            tracing::info!(
                "[relay] tunnel link 断开，清理 {} 条活跃隧道: {}",
                orphan_ids.len(),
                device_id
            );
        }
    }
    writer.abort();
    tracing::info!("[relay] tunnel link offline: {}", device_id);
}

/// 启动公网隧道监听（独立端口）。RELAY_TUNNEL_PORT 默认 18081；
/// RELAY_TUNNEL_DEVICE 指定目标设备（默认 desktop-main）。
async fn spawn_tunnel_listener(state: AppState) {
    let port: u16 = std::env::var("RELAY_TUNNEL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(18081);
    let target = std::env::var("RELAY_TUNNEL_DEVICE").unwrap_or_else(|_| "desktop-main".into());

    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("[relay] 隧道端口 :{} 绑定失败: {}", port, e);
            return;
        }
    };
    tracing::info!(
        "[relay] tunnel listening on 0.0.0.0:{} -> device {}",
        port,
        target
    );

    loop {
        let Ok((tcp, peer)) = listener.accept().await else {
            continue;
        };
        let ip = peer.ip();
        let st = state.clone();
        let tgt = target.clone();

        // ── 安全：隧道入口限流（每 IP 新建连接限速 + 全局/单 IP 并发上限） ──
        if !rate_check(&st.tunnel_rate, ip, TUNNEL_RATE_PER_IP_MIN) {
            tracing::warn!("[relay] tunnel rate limited: ip={}", ip);
            continue; // 直接 drop TCP，不转发
        }
        {
            let total = lock_recover(&st.tunnel_active);
            let ip_map = lock_recover(&st.tunnel_active_ip);
            let n = ip_map.get(&ip).copied().unwrap_or(0);
            if *total >= TUNNEL_MAX_ACTIVE_TOTAL || n >= TUNNEL_MAX_ACTIVE_PER_IP {
                tracing::warn!(
                    "[relay] tunnel concurrency limited: ip={} total={} per_ip={}",
                    ip,
                    *total,
                    n
                );
                continue;
            }
        }
        // 预占并发计数（handle 结束经 TunnelCountGuard Drop 释放）
        {
            let mut total = lock_recover(&st.tunnel_active);
            let mut ip_map = lock_recover(&st.tunnel_active_ip);
            *total += 1;
            *ip_map.entry(ip).or_insert(0) += 1;
        }
        tokio::spawn(async move {
            handle_tunnel_conn(st, tcp, tgt, ip).await;
        });
    }
}

/// 隧道并发计数守卫：handle 退出（含所有提前 return / panic）时自动释放
struct TunnelCountGuard {
    state: AppState,
    ip: IpAddr,
}

impl Drop for TunnelCountGuard {
    fn drop(&mut self) {
        let mut total = lock_recover(&self.state.tunnel_active);
        if *total > 0 {
            *total -= 1;
        }
        let mut ip_map = lock_recover(&self.state.tunnel_active_ip);
        if let Some(n) = ip_map.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                ip_map.remove(&self.ip);
            }
        }
    }
}



/// 设备离线页：设备隧道链路不在线时返回。自包含，zh-CN，移动端竖屏居中卡片。
/// meta refresh 3s 自动重试：桌面重连（离线窗口 1~6s）期间拿到本页的手机端可自动恢复，
/// 与 RETRY_HTML 的自愈理念一致——任何死窗口都不让用户手动刷新。
const OFFLINE_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Nuphus 设备离线</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  html, body { height: 100%; }
  body {
    display: flex; align-items: center; justify-content: center;
    padding: 24px; background: #0f1115; color: #e6e8ee;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
      "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  .card {
    width: 100%; max-width: 420px; padding: 36px 28px;
    background: #1a1d24; border: 1px solid #2a2e38; border-radius: 16px;
    text-align: center;
  }
  .dot {
    width: 14px; height: 14px; margin: 0 auto 20px;
    background: #e5484d; border-radius: 50%;
  }
  h1 { font-size: 22px; font-weight: 600; margin-bottom: 14px; }
  p { font-size: 15px; line-height: 1.8; color: #a8adb8; }
  .section {
    margin-top: 22px; padding: 16px; text-align: left;
    background: #14161c; border-radius: 12px;
  }
  .section h2 { font-size: 14px; font-weight: 600; color: #e6e8ee; margin-bottom: 6px; }
  .section p { font-size: 13px; }
  .btn {
    display: inline-block; margin-top: 24px; padding: 12px 40px;
    background: #f0b429; color: #14161c; border: none; border-radius: 10px;
    font-size: 16px; font-weight: 600; text-decoration: none;
    -webkit-tap-highlight-color: transparent; cursor: pointer;
    font-family: inherit;
  }
  .btn:active { opacity: 0.85; }
  .btn-lan {
    display: block; margin-top: 12px; padding: 12px 16px;
    background: transparent; color: #e6e8ee;
    border: 1px solid #2a2e38; border-radius: 10px;
    font-size: 15px; font-weight: 500; text-decoration: none;
    -webkit-tap-highlight-color: transparent;
  }
  .btn-lan:active { background: #23262e; }
  .ts { margin-top: 18px; text-align: left; }
  .ts summary { font-size: 13px; color: #a8adb8; cursor: pointer; padding: 6px 0; }
  .ts ul { margin: 8px 0 0; padding-left: 18px; }
  .ts li { font-size: 12.5px; line-height: 1.9; color: #8b919c; }
  .hint { margin-top: 14px; font-size: 12px; color: #6b7280; line-height: 1.6; }
</style>
</head>
<body>
  <div class="card">
    <div class="dot"></div>
    <h1>设备离线</h1>
    <p>桌面端当前不在线，无法建立远程连接。</p>
    <div class="section">
      <h2>请检查</h2>
      <p>确认电脑端 Nuphus 正在运行，且网络连接正常。可点下方按钮立即重试，页面也会自动恢复。</p>
    </div>
    <button type="button" class="btn" onclick="location.reload()">立即重试</button>
    <!--LAN_BLOCK-->
    <!--TS_BLOCK-->
    <div class="hint">已确认电脑在线？点「立即重试」重新连接</div>
  </div>
</body>
</html>"#;

/// 生成「同一 WiFi 直连」注入块：自动探测 JS + 直连按钮。
/// 手机被隧道断线窗口困住时，页面加载即自动 fetch 桌面 /health（无鉴权 + CORS 允许），
/// 可达则自动跳转直连，全程不依赖隧道（JS 来自中继响应本身）。lan_url 为空返回空串。
fn lan_block(lan_url: Option<&str>) -> String {
    // 无条件注入自动重试：页面挂起后整体刷新，交给下一次请求重试。
    // 不再用 <meta refresh> 定时刷新——定时刷新会打断下面的 3s 探测窗口。
    let retry_js = r#"<script>
(function () {
  setTimeout(function () { location.reload(); }, 4000);
})();
</script>"#;
    match lan_url {
        Some(url) if !url.is_empty() => {
            // JS 字符串转义：lan_url 由桌面握手自报，防御性转义防 </script> / 引号破坏脚本
            let js_url = url
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace("</", "<\\/");
            // HTML 属性转义（href）
            let html_url = url.replace('&', "&amp;").replace('"', "&quot;");
            format!(
                r#"<script>
(function () {{
  var LAN_URL = "{js_url}";
  if (!LAN_URL) return;
  var token = null;
  try {{ token = localStorage.getItem("nuphus_mobile_token"); }} catch (e) {{}}
  function goLan() {{
    location.replace(token ? LAN_URL + "/?token=" + encodeURIComponent(token) : LAN_URL);
  }}
  var ctrl = new AbortController();
  var timer = setTimeout(function () {{ ctrl.abort(); }}, 3000);
  fetch(LAN_URL + "/health", {{ signal: ctrl.signal, cache: "no-store" }})
    .then(function (r) {{
      clearTimeout(timer);
      if (!r.ok) return;
      goLan();
    }})
    .catch(function () {{
      // fetch 可能被 Mixed Content 拦截（iOS Safari：HTTPS 页面请求 HTTP 局域网）
      // 或局域网不可达。top-level 导航不受 mixed content 限制——直接尝试切直连；
      // 失败（真不在同一 WiFi）由浏览器显示错误页，用户可返回重试页。
      if (token) {{ goLan(); }}
    }});
}})();
</script>
<a class="btn-lan" href="{html_url}">同一 WiFi 下直连电脑（免费）</a>{retry_js}"#,
                js_url = js_url,
                html_url = html_url,
                retry_js = retry_js,
            )
        }
        _ => retry_js.to_string(),
    }
}

/// 动态生成设备离线页：pro 设备隧道链路离线 + 设备通道也不在线时返回。
/// 注入 lan 直连块（自动探测 + 手动按钮），不可达时 3s 自动重试中继兜底。
fn offline_html(lan_url: Option<&str>) -> String {
    OFFLINE_HTML
        .replace("<!--LAN_BLOCK-->", &lan_block(lan_url))
        .replace("<!--TS_BLOCK-->", TS_BLOCK)
}

/// 动态生成重试页：桌面设备在线（/ws/device 在）但隧道链路离线/半死时返回。
/// 语义「设备在，通道在恢复」——注入 lan 直连块，同 WiFi 手机可自动切直连。
fn retry_html(lan_url: Option<&str>) -> String {
    RETRY_HTML
        .replace("<!--LAN_BLOCK-->", &lan_block(lan_url))
        .replace("<!--TS_BLOCK-->", TS_BLOCK)
}

/// 隧道写方向半死重试页：Ready 超时（链路在线但写方向不通）时返回给手机。
/// 自包含 + meta refresh 2s 自动重试——链路恢复后下一次请求即正常放行，
/// 手机白屏变成可见的自愈提示（设备真离线时走 OFFLINE_HTML，不会到这里）。
const RETRY_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Nuphus 连接重试</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  html, body { height: 100%; }
  body {
    display: flex; align-items: center; justify-content: center;
    padding: 24px; background: #0f1115; color: #e6e8ee;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC",
      "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  .card {
    width: 100%; max-width: 420px; padding: 36px 28px;
    background: #1a1d24; border: 1px solid #2a2e38; border-radius: 16px;
    text-align: center;
  }
  .spinner {
    width: 36px; height: 36px; margin: 0 auto 18px;
    border: 3px solid #2a2e38; border-top-color: #f0b429;
    border-radius: 50%; animation: spin 1s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  h1 { font-size: 20px; font-weight: 600; margin-bottom: 12px; }
  p { font-size: 14px; line-height: 1.8; color: #a8adb8; }
  .btn-lan {
    display: block; margin-top: 20px; padding: 12px 16px;
    background: transparent; color: #e6e8ee;
    border: 1px solid #2a2e38; border-radius: 10px;
    font-size: 15px; font-weight: 500; text-decoration: none;
    -webkit-tap-highlight-color: transparent;
  }
  .btn-lan:active { background: #23262e; }
  .ts { margin-top: 18px; text-align: left; }
  .ts summary { font-size: 13px; color: #a8adb8; cursor: pointer; padding: 6px 0; }
  .ts ul { margin: 8px 0 0; padding-left: 18px; }
  .ts li { font-size: 12.5px; line-height: 1.9; color: #8b919c; }
</style>
</head>
<body>
  <div class="card">
    <div class="spinner"></div>
    <h1>连接不稳定，正在重试</h1>
    <p>网络隧道正在恢复，页面将自动重连。若长时间停留在此页，请检查电脑端 Nuphus 是否在运行。</p>
    <!--LAN_BLOCK-->
    <!--TS_BLOCK-->
  </div>
</body>
</html>"#;

/// 故障自查清单（离线/重试页共用，<!--TS_BLOCK--> 占位注入）。
/// 借鉴 dsh-pocket 的 FAQ 工程化：把最常见原因直接写在用户连不上时看到的页面里，
/// 减少「中继是不是挂了」的误判。details 折叠默认收起，不干扰主信息。
const TS_BLOCK: &str = r#"<details class="ts">
<summary>一直连不上？点这里自查</summary>
<ul>
<li>电脑端 Nuphus 正在运行，且「设置 → 手机访问 → 中继转发」已开启</li>
<li>电脑若开着代理 / VPN（Clash 等 TUN 模式），尝试关闭后重试</li>
<li>手机与电脑在同一 WiFi 时，优先用「同一 WiFi 下直连」，更快更稳</li>
<li>公司 / 校园网可能拦截长连接，切手机 4G / 5G 流量重试</li>
<li>长期无法恢复：桌面端 设置 → 手机访问 →「重新生成调用凭据」后重新扫码</li>
</ul>
</details>"#;

/// 往公网隧道 TCP 流直写完整 HTTP/1.1 静态页响应并关闭连接。
/// 薄转发原则：中继不理解也不读取手机发来的 HTTP 请求内容，直接回固定页面。
async fn write_static_page(tcp: &mut TcpStream, html: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = tcp.write_all(resp.as_bytes()).await;
    let _ = tcp.shutdown().await;
}

async fn handle_tunnel_conn(state: AppState, mut tcp: TcpStream, target: String, ip: IpAddr) {
    // 并发计数已由调用方预占；guard 确保本函数所有退出路径释放
    let _guard = TunnelCountGuard {
        state: state.clone(),
        ip,
    };

    // TCP_NODELAY：禁用 Nagle，避免 HTTP 小请求/控制帧被 40ms ACK 延迟累积拖慢转发
    let _ = tcp.set_nodelay(true);

    // 1. 链路分流（2026-08 起 Pro 体系移除，远程访问对所有配对设备免费）：
    //    链路离线 → 按设备通道（/ws/device）是否在线区分「连接中」与「真离线」，
    //    回重试页/离线页（均注入局域网直连入口，同 WiFi 手机自动切直连）；在线 → 转发。
    let link = lock_recover(&state.tunnel_links)
        .get(&target)
        .map(|(_, tx)| tx.clone());
    let Some(link) = link else {
        let lan = lock_recover(&state.device_lan_urls).get(&target).cloned();
        let device_online = lock_recover(&state.devices).contains_key(&target);
        let mut tcp = tcp;
        if device_online {
            tracing::info!(
                "[relay] 隧道分流：设备 {} 隧道离线但设备在线，回重试页",
                target
            );
            write_static_page(&mut tcp, &retry_html(lan.as_deref())).await;
        } else {
            tracing::info!("[relay] 隧道分流：设备 {} 隧道链路离线，回离线页", target);
            write_static_page(&mut tcp, &offline_html(lan.as_deref())).await;
        }
        return;
    };

    // 2. 注册隧道
    let tunnel_id = new_task_id(&target);
    let (tx, mut rx) = mpsc::unbounded_channel::<TunnelFrame>();
    lock_recover(&state.tunnels).insert(tunnel_id.clone(), tx);
    // 记录隧道发起连接的 reset sender：ready 超时只 reset 此连接。
    // 若不记录而取「当前注册连接」，旧连接半死超时后新连接已注册时会把新连接误杀
    // （实测 P0：新连接刚 online 就被残留 reset 断开 → 重连风暴 → 手机持续「设备离线」）。
    if let Some((_, reset_tx)) = lock_recover(&state.tunnel_reset).get(&target) {
        lock_recover(&state.tunnel_owner).insert(tunnel_id.clone(), reset_tx.clone());
    }
    tracing::info!("[relay] tunnel open: {}", tunnel_id);

    // 3. 通知设备建立本地连接（无界 send 非阻塞；失败仅当设备 WS 已断）
    if link
        .send(TunnelFrame::Open {
            tunnel_id: tunnel_id.clone(),
        })
        .is_err()
    {
        lock_recover(&state.tunnels).remove(&tunnel_id);
        lock_recover(&state.tunnel_owner).remove(&tunnel_id); // 早退同步清理，防泄漏（审计 R1.3）
        return;
    }

    // 4. 等待设备 ready（TUNNEL_READY_TIMEOUT_SECS 超时；设备本地 mobile_server 未启动会回 error）。
    //    就绪判定放宽为「Ready 或 Data」：链路抖动下 Ready 单帧可能丢失，但设备能回 Data
    //    即证明本地连接已就绪、隧道可转发——若只等 Ready，丢帧后服务器空等超时误判写半死，
    //    强制 reset 触发桌面重连循环（实测：Ready 丢失 → 10s 超时 → tunnel link 强制重置）。
    //    等待期间收到的 Data 帧存入预读缓冲，进入转发阶段先写出，保证首包数据不丢失。
    let ready: (bool, Vec<TunnelFrame>) =
        tokio::time::timeout(Duration::from_secs(TUNNEL_READY_TIMEOUT_SECS), async {
            let mut prelude: Vec<TunnelFrame> = Vec::new();
            loop {
                match rx.recv().await {
                    Some(TunnelFrame::Ready { .. }) => return (true, prelude),
                    Some(frame @ TunnelFrame::Data { .. }) => prelude.push(frame),
                    Some(TunnelFrame::Error { .. }) | None => return (false, prelude),
                    Some(_) => continue,
                }
            }
        })
        .await
        .unwrap_or((false, Vec::new()));

    if !ready.0 {
        lock_recover(&state.tunnels).remove(&tunnel_id);
        let _ = link.send(TunnelFrame::Close {
            tunnel_id: tunnel_id.clone(),
        });
        // 手机白屏自愈：链路在线但写方向半死时，先回自动重试页再关闭——白屏变可见提示，
        // meta refresh 2s 自动重试，链路恢复后下一次请求正常放行（设备离线不会走到这里）。
        // 与上方链路离线路径一致注入局域网直连入口（retry_html 而非 RETRY_HTML 常量）——
        // 此前此处直接用常量，<!--LAN_BLOCK--> 占位符未替换，半死路径（手机最常命中的
        // 路径）从不显示「同一 WiFi 直连」入口，同 WiFi 手机被困在重试页（实测反馈）。
        let lan = lock_recover(&state.device_lan_urls).get(&target).cloned();
        write_static_page(&mut tcp, &retry_html(lan.as_deref())).await;
        tracing::warn!("[relay] tunnel {} 设备未就绪，已回重试页并关闭", tunnel_id);
        // 半死自愈：ready 超时 = 隧道写方向半死（桌面「已投递本地」但 Ready 回不到中继）。
        // 触发发起连接的隧道 WS 强制断开，桌面 read 返回 Err 后经统一状态机重连，新连接恢复。
        // 用 owner（发起连接）而非「当前注册连接」：旧连接断后新连接已注册时不会误杀新连接。
        if let Some(reset_tx) = lock_recover(&state.tunnel_owner).remove(&tunnel_id) {
            let _ = reset_tx.send(());
            tracing::warn!(
                "[relay] 已触发隧道强制重置: {} (owner={})",
                tunnel_id,
                target
            );
        } else {
            // owner 缺失（open 时无连接注册）：链路已切走或断开，无需重置，回退静默
            tracing::debug!(
                "[relay] tunnel {} ready 超时但无 owner，跳过重置",
                tunnel_id
            );
        }
        return;
    }

    // 5. 双向转发
    let (mut tcp_r, mut tcp_w) = tcp.into_split();
    let link2 = link.clone();
    // ready 等待期间收到的预读 Data 帧（Ready 丢帧兜底），先写出再进入正常转发
    let prelude = ready.1;
    // 设备帧 → TCP 写
    let writer = tokio::spawn(async move {
        for frame in prelude {
            if let TunnelFrame::Data { data, .. } = frame {
                use base64::Engine as _;
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                    if tcp_w.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
            }
        }
        while let Some(frame) = rx.recv().await {
            match frame {
                TunnelFrame::Data { data, .. } => {
                    use base64::Engine as _;
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                        if tcp_w.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                }
                TunnelFrame::Close { .. } | TunnelFrame::Error { .. } => break,
                _ => {}
            }
        }
        let _ = tcp_w.shutdown().await;
    });

    // TCP 读 → 设备帧（空闲超时：长期无数据强制关闭，防僵尸连接占并发额度）
    let mut buf = [0u8; 16384];
    let idle_timeout = Duration::from_secs(TUNNEL_IDLE_TIMEOUT_SECS);
    loop {
        match tokio::time::timeout(idle_timeout, tcp_r.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                // 无界 send 非阻塞：不因单隧道慢而阻塞共享读循环饿死其他并发隧道
                if link2
                    .send(TunnelFrame::Data {
                        tunnel_id: tunnel_id.clone(),
                        data,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(Err(_)) => break,
            Err(_) => {
                tracing::info!("[relay] tunnel idle timeout: {}", tunnel_id);
                break;
            }
        }
    }

    // 6. 清理：通知设备关闭本地连接
    writer.abort();
    lock_recover(&state.tunnels).remove(&tunnel_id);
    lock_recover(&state.tunnel_owner).remove(&tunnel_id);
    let _ = link2.send(TunnelFrame::Close {
        tunnel_id: tunnel_id.clone(),
    });
    tracing::info!("[relay] tunnel closed: {}", tunnel_id);
}

// ── main ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "relay_server=info,tower_http=warn,axum=warn".into()),
        )
        .init();

    let port: u16 = std::env::var("RELAY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(18080);

    if caller_token().is_empty() || device_token().is_empty() {
        tracing::error!(
            "[relay] RELAY_DEVICE_TOKEN / RELAY_CALLER_TOKEN 未配置，拒绝启动（防裸奔）"
        );
        std::process::exit(1);
    }

    let state = AppState::new();
    let tunnel_state = state.clone();
    let app = Router::new()
        .route("/health", get(health))
        .route("/task", post(post_task))
        .route("/admin/rotate-caller-token", post(post_rotate_caller_token))
        .route("/ws/device", get(ws_device))
        .route("/ws/tunnel", get(ws_tunnel))
        .layer(middleware::from_fn(cors_mw))
        // 安全中间件：IP 速率限制 + 访问日志（需 ConnectInfo 注入对端 IP）
        .layer(middleware::from_fn_with_state(state.clone(), security_mw))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    tracing::info!(
        "[relay] listening on {} (device_token={} caller_token={})",
        addr,
        !device_token().is_empty(),
        !caller_token().is_empty()
    );

    // 隧道监听独立 spawn：失败仅 warn（不影响主服务）
    tokio::spawn(spawn_tunnel_listener(tunnel_state));

    // ── 服务启动：明文 http/ws（生产 TLS 在 nginx 终结；进程内 TLS 已随
    // axum-server tls-rustls 特性一并移除——该特性拉入 aws-lc-sys，交叉编译过重且产线未使用）──
    let service = app.into_make_service_with_connect_info::<SocketAddr>();
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("[relay] bind {} 失败: {}", addr, e);
            std::process::exit(1);
        });
    axum::serve(listener, service).await.unwrap_or_else(|e| {
        tracing::error!("[relay] serve 失败: {}", e);
        std::process::exit(1);
    });
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_html_injects_lan_block() {
        let html = offline_html(Some("http://192.168.1.79:18772"));
        assert!(html.contains("同一 WiFi 下直连电脑（免费）"));
        assert!(html.contains("href=\"http://192.168.1.79:18772\""));
        assert!(html.contains("立即重试"));
        assert!(html.contains("<!--LAN_BLOCK-->") == false, "占位符应被替换");
        assert!(
            html.contains("<!--TS_BLOCK-->") == false,
            "自查占位符应被替换"
        );
        assert!(html.contains("一直连不上"), "故障自查清单应注入");
        // 自动探测 JS：LAN_URL 注入 + /health 探测 + 跳转直连
        assert!(html.contains("<script>"), "必须内联自动探测脚本");
        assert!(
            html.contains("LAN_URL = \"http://192.168.1.79:18772\""),
            "lan_url 注入 JS"
        );
        assert!(html.contains("/health"), "探测端点");
        assert!(html.contains("location.replace"), "自动跳转");
        assert!(html.contains("nuphus_mobile_token"), "跨源配对 token 读取");
    }

    #[test]
    fn offline_html_without_lan_url_omits_block() {
        let html = offline_html(None);
        assert!(!html.contains("同一 WiFi 下直连电脑"));
        assert!(
            !html.contains("<!--LAN_BLOCK-->"),
            "空 lan_url 时占位符应被替换为自动重试脚本"
        );
        // 无条件自动重试脚本仍注入（页面挂起后整体刷新，交给下一次请求重试）
        assert!(html.contains("location.reload"), "无 lan_url 也必须有自动重试");
        assert!(!html.contains("LAN_URL"), "无 lan_url 不注入探测脚本");
        assert!(html.contains("立即重试"), "无 lan_url 也必须有手动重试按钮");
    }

    #[test]
    fn offline_html_ignores_empty_lan_url() {
        let html = offline_html(Some(""));
        assert!(!html.contains("同一 WiFi 下直连电脑"));
        assert!(html.contains("location.reload"), "空 lan_url 也必须有自动重试");
        assert!(!html.contains("LAN_URL"));
        assert!(html.contains("立即重试"));
    }

    #[test]
    fn offline_html_escapes_lan_url_in_js_and_href() {
        // 恶意/异常 lan_url：JS 字符串引号必须转义，HTML href 双引号必须实体化
        let html = offline_html(Some("http://10.0.0.5:18772\" onerror=\"x"));
        assert!(
            html.contains("10.0.0.5:18772\\\""),
            "JS 字符串中引号应被转义"
        );
        assert!(html.contains("&quot;"), "HTML href 引号应实体化");
        assert!(
            !html.contains(
                "<script>\n(function () { var LAN_URL = \"http://10.0.0.5:18772\" onerror"
            ),
            "不得破坏脚本"
        );
    }

    #[test]
    fn retry_html_injects_lan_block() {
        let html = retry_html(Some("http://192.168.1.79:18772"));
        assert!(html.contains("连接不稳定，正在重试"), "重试页语义");
        assert!(html.contains("同一 WiFi 下直连电脑（免费）"));
        assert!(html.contains("<script>"), "自动探测脚本注入");
        assert!(
            html.contains("LAN_URL = \"http://192.168.1.79:18772\""),
            "lan_url 注入"
        );
        assert!(!html.contains("<!--LAN_BLOCK-->"), "占位符应被替换");
        assert!(!html.contains("<!--TS_BLOCK-->"), "自查占位符应被替换");
    }

    #[test]
    fn retry_html_without_lan_url_omits_block() {
        let html = retry_html(None);
        assert!(html.contains("连接不稳定，正在重试"));
        assert!(!html.contains("同一 WiFi 下直连电脑"));
        assert!(html.contains("location.reload"), "无 lan_url 也必须有自动重试");
        assert!(!html.contains("LAN_URL"), "无 lan_url 不注入探测脚本");
        assert!(!html.contains("<!--LAN_BLOCK-->"), "占位符替换为自动重试");
    }

    #[test]
    fn lan_block_empty_for_empty_url() {
        // 空 lan_url：仅注入无条件自动重试脚本，不注入探测/按钮
        let s = lan_block(Some(""));
        assert!(s.contains("location.reload"));
        assert!(!s.contains("LAN_URL"));
        assert!(!s.contains("同一 WiFi"));
        let n = lan_block(None);
        assert!(n.contains("location.reload"));
        assert!(!n.contains("LAN_URL"));
    }

    #[test]
    fn lock_recover_survives_poison() {
        use std::sync::{Arc, Mutex};
        let m = Arc::new(Mutex::new(42i32));
        // 毒化：持锁 panic
        let m2 = m.clone();
        let _ = std::panic::catch_unwind(move || {
            let _g = m2.lock().unwrap();
            panic!("boom");
        });
        assert!(m.is_poisoned(), "锁应处于中毒状态");
        // 恢复后仍可读写，且数据自洽
        {
            let mut g = lock_recover(&m);
            *g += 1;
        }
        assert_eq!(*m.lock().unwrap_or_else(|e| e.into_inner()), 43);
    }

    /// caller_token 热轮换语义：写入 relay_caller.token 后 load_token 立即读到新值
    ///（每次请求重读文件，免重启）。env 为进程级共享，静态锁防并行测试互踩。
    #[test]
    fn caller_token_hot_reload_after_write() {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!("relay-token-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("RELAY_DATA_DIR", &dir);
        std::env::remove_var("RELAY_CALLER_TOKEN");

        // 初始为空
        assert_eq!(caller_token(), "");
        // 写入即生效（模拟轮换端点的持久化）
        std::fs::write(dir.join("relay_caller.token"), "new-token-abc").unwrap();
        assert_eq!(caller_token(), "new-token-abc");
        // token_ok 校验语义：空 expected 永不通过；匹配才通过
        assert!(token_ok("new-token-abc", &caller_token()));
        assert!(!token_ok("wrong", &caller_token()));

        std::env::remove_var("RELAY_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }
}