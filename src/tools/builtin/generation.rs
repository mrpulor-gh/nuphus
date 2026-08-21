//! generation — MiniMax 多模态生成工具（image_generate / video_generate）
//!
//! 定位：MiniMax 生成 API 的通用调用器，只做「认证 + 传输 + 落盘」。
//! 模型与生成参数全部开放用户填写（`params` JSON 原样透传合并进请求体），
//! 工具不枚举不拦截任何生成参数——新模型新参数无需改代码。
//!
//! API 事实（2026-08-12 从 MiniMax 官方 cli 仓库源码核实：MiniMax-AI/cli）：
//! - 图像：`POST {base}/v1/image_generation` 同步返回 `data.image_urls[]`
//!   （或 `data.image_base64[]`）；宽高约束 512–2048 且 8 的倍数（官方校验）
//! - 视频 V2（H3 系）：`POST {base}/v2/video_generation` → task_id → 轮询
//!   `GET {base}/v2/query/video_generation/{task_id}`（status ∈
//!   succeeded/failed/cancelled/expired）
//! - 视频 V1（Hailuo-2.3 等）：`POST {base}/v1/video_generation` → task_id → 轮询
//!   `GET {base}/v1/query/video_generation?task_id=`（status ∈ Success/Failed）
//! - 产出文件：`GET {base}/v1/files/retrieve?file_id=` → `file.download_url` → 下载
//! - V1/V2 路由对齐官方 cli 的 isVideoV2Model：模型名含 "H3" 走 V2，其余走 V1
//!
//! 凭证：`config::load_registry()` 找 provider_type=minimax 且 api_key 非空的
//! provider。api_key 只进 Authorization 头，永不写入输出/日志/工作流文件。
//! 产出目录：`~/.nuphus/generated/`（自动创建），文件名带时间戳。

use crate::config::{load_registry, ModelRegistry, ProviderKind};
use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// MiniMax 默认 base（registry provider 未配 base_url 时的兜底，与
/// config::providers::minimax 保持一致）
const DEFAULT_BASE_URL: &str = "https://api.minimaxi.com/v1";
/// image_generate 默认模型（仅便捷兜底，用户可填任意模型名）
const DEFAULT_IMAGE_MODEL: &str = "image-01";
/// video_generate 默认模型（仅便捷兜底，用户可填任意模型名）
const DEFAULT_VIDEO_MODEL: &str = "MiniMax-H3";
/// 视频轮询默认间隔 / 默认总超时
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
const DEFAULT_VIDEO_TIMEOUT_SECS: u64 = 300;
/// 视频轮询超时上限（外层 registry 给本工具 900s 桶，见 registry.rs 超时分级）
const MAX_VIDEO_TIMEOUT_SECS: u64 = 600;
/// 单个文件下载上限（防异常响应打爆磁盘；H3 官方限制视频源文件 ≤50MB）
const DOWNLOAD_CAP_BYTES: u64 = 256 * 1024 * 1024;
/// 错误响应体透出截断长度
const ERROR_BODY_CAP: usize = 500;

static GEN_AGENT_CACHE: Mutex<Option<(reqwest::blocking::Client, i64)>> = Mutex::new(None);

/// 生成工具专用 client（复用 web.rs 的 AGENT_TTL 缓存模式）
fn get_agent() -> reqwest::blocking::Client {
    let now = super::web::unix_now();
    if let Ok(cache) = GEN_AGENT_CACHE.lock() {
        if let Some((ref agent, ts)) = *cache {
            if now - ts < super::web::AGENT_TTL {
                return agent.clone();
            }
        }
    }
    let agent = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to build reqwest blocking Client");
    if let Ok(mut cache) = GEN_AGENT_CACHE.lock() {
        *cache = Some((agent.clone(), now));
    }
    agent
}

// ── 凭证解析 ──

struct GenCredential {
    api_key: String,
    base_url: String,
}

/// 从 registry 解析 MiniMax 凭证。
/// `provider` 为 Some 时按 provider 名过滤（多 MiniMax provider 场景指定）；
/// 否则取第一个 provider_type=minimax 且 api_key 非空的 provider。
fn find_minimax_credential(
    registry: &ModelRegistry,
    provider: Option<&str>,
) -> Result<GenCredential, String> {
    let cfg = registry
        .providers
        .iter()
        .find(|p| {
            p.provider_type == ProviderKind::MiniMax
                && !p.api_key.trim().is_empty()
                && provider.is_none_or(|name| p.name == name)
        })
        .ok_or_else(|| match provider {
            Some(name) => format!(
                "未找到名为 '{name}' 且配置了 api_key 的 MiniMax provider（检查 registry 配置）"
            ),
            None => {
                "未配置 MiniMax provider：registry 中无 provider_type=minimax 且 api_key 非空的配置"
                    .to_string()
            }
        })?;
    let base_url = if cfg.base_url.trim().is_empty() {
        DEFAULT_BASE_URL.to_string()
    } else {
        cfg.base_url.trim().trim_end_matches('/').to_string()
    };
    Ok(GenCredential {
        api_key: cfg.api_key.clone(),
        base_url,
    })
}

// ── 请求体合并与路由判定 ──

/// 合并请求体：params 透传为底，顶层参数（model/prompt）覆盖同名键。
fn merge_request_body(
    params: Option<&Value>,
    model: &str,
    prompt: &str,
) -> Result<Map<String, Value>, String> {
    let mut body = match params {
        None => Map::new(),
        Some(v) => v
            .as_object()
            .cloned()
            .ok_or_else(|| "params must be a JSON object".to_string())?,
    };
    // 顶层参数优先：后插入覆盖 params 同名键
    body.insert("model".to_string(), json!(model));
    body.insert("prompt".to_string(), json!(prompt));
    Ok(body)
}

/// 视频 V1/V2 路由：对齐 MiniMax 官方 cli 的 isVideoV2Model——
/// 模型名含 "H3" 走 V2 端点，其余走 V1。
fn is_video_v2_model(model: &str) -> bool {
    model.contains("H3")
}

/// registry base_url 通常以 /v1 结尾；V2 端点在同级 /v2 下，取 API root。
fn api_root(base_url: &str) -> &str {
    let b = base_url.trim_end_matches('/');
    b.strip_suffix("/v1").unwrap_or(b)
}

// ── MiniMax 响应解析 ──

/// 从响应体提取 base_resp 业务错误（status_code != 0 视为失败）。
fn extract_api_error(body: &Value) -> Option<String> {
    let base_resp = body.get("base_resp")?;
    let code = base_resp.get("status_code")?.as_i64()?;
    if code == 0 {
        return None;
    }
    let msg = base_resp
        .get("status_msg")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Some(format!("MiniMax API error (status_code={code}): {msg}"))
}

/// 图像响应条目：URL 或 base64 载荷。
enum ImageItem {
    Url(String),
    Base64(String),
}

/// 从 image_generation 响应的 data 字段提取图像条目
/// （`image_urls[]` 与 `image_base64[]` 两种形态都接）。
fn collect_image_items(data: &Value) -> Vec<ImageItem> {
    let mut items = Vec::new();
    if let Some(urls) = data.get("image_urls").and_then(|v| v.as_array()) {
        for u in urls.iter().filter_map(|v| v.as_str()) {
            if !u.is_empty() {
                items.push(ImageItem::Url(u.to_string()));
            }
        }
    }
    if let Some(b64s) = data.get("image_base64").and_then(|v| v.as_array()) {
        for b in b64s.iter().filter_map(|v| v.as_str()) {
            if !b.is_empty() {
                items.push(ImageItem::Base64(b.to_string()));
            }
        }
    }
    items
}

/// 视频任务状态提取：V2 嵌套在 task.status，V1 在顶层 status，两种都接。
fn video_task_status(resp: &Value) -> Option<String> {
    resp.pointer("/task/status")
        .and_then(|v| v.as_str())
        .or_else(|| resp.get("status").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

/// 视频产出 file_id 提取：task.file_id / 顶层 file_id 两种形态都接。
fn video_file_id(resp: &Value) -> Option<String> {
    resp.pointer("/task/file_id")
        .and_then(|v| v.as_str())
        .or_else(|| resp.get("file_id").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ── HTTP 收发 ──

fn classify_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("MiniMax API request failed: timeout ({e})")
    } else if e.is_connect() {
        format!("MiniMax API request failed: connection error ({e})")
    } else {
        format!("MiniMax API request failed: {e}")
    }
}

/// 响应文本读取并做业务错误解析（base_resp 透出，不吞错误体）。
fn parse_response(resp: reqwest::blocking::Response, endpoint: &str) -> Result<Value, String> {
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| format!("读取 MiniMax 响应体失败 ({endpoint}): {e}"))?;
    if !status.is_success() {
        let truncated: String = text.chars().take(ERROR_BODY_CAP).collect();
        return Err(format!(
            "MiniMax API HTTP {status} ({endpoint}): {truncated}"
        ));
    }
    let body: Value = serde_json::from_str(&text).map_err(|e| {
        let truncated: String = text.chars().take(ERROR_BODY_CAP).collect();
        format!("MiniMax 响应非 JSON ({endpoint}): {e} — body: {truncated}")
    })?;
    if let Some(err) = extract_api_error(&body) {
        return Err(format!("{err} ({endpoint})"));
    }
    Ok(body)
}

fn post_json(
    agent: &reqwest::blocking::Client,
    cred: &GenCredential,
    url: &str,
    body: &Value,
) -> Result<Value, String> {
    let resp = agent
        .post(url)
        .bearer_auth(&cred.api_key)
        .json(body)
        .send()
        .map_err(|e| classify_reqwest_error(&e))?;
    parse_response(resp, url)
}

fn get_json(
    agent: &reqwest::blocking::Client,
    cred: &GenCredential,
    url: &str,
) -> Result<Value, String> {
    let resp = agent
        .get(url)
        .bearer_auth(&cred.api_key)
        .send()
        .map_err(|e| classify_reqwest_error(&e))?;
    parse_response(resp, url)
}

// ── 落盘 ──

/// 产出目录：~/.nuphus/generated/（自动创建）
fn generated_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法定位用户主目录".to_string())?;
    let dir = home.join(".nuphus").join("generated");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建产出目录失败 {}: {e}", dir.display()))?;
    Ok(dir)
}

fn file_timestamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S_%3f").to_string()
}

/// 从 URL 推断图像扩展名（白名单内取原扩展，否则兜底 jpg）。
fn image_ext_from_url(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "png",
        "webp" => "webp",
        "jpeg" => "jpeg",
        _ => "jpg",
    }
}

/// 下载 URL 到本地文件（带大小上限）。
fn download_to_file(
    agent: &reqwest::blocking::Client,
    url: &str,
    path: &std::path::Path,
) -> Result<u64, String> {
    let mut resp = agent
        .get(url)
        .send()
        .map_err(|e| format!("下载生成文件失败: {}", classify_reqwest_error(&e)))?;
    if !resp.status().is_success() {
        return Err(format!("下载生成文件失败: HTTP {}", resp.status()));
    }
    let mut file =
        std::fs::File::create(path).map_err(|e| format!("创建文件失败 {}: {e}", path.display()))?;
    let written = std::io::copy(&mut resp, &mut file)
        .map_err(|e| format!("写入文件失败 {}: {e}", path.display()))?;
    if written == 0 {
        let _ = std::fs::remove_file(path);
        return Err(format!("下载内容为空: {url}"));
    }
    if written > DOWNLOAD_CAP_BYTES {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "下载文件超过大小上限 ({DOWNLOAD_CAP_BYTES} bytes): {url}"
        ));
    }
    Ok(written)
}

/// base64 载荷落盘。
fn save_base64_image(b64: &str, path: &std::path::Path) -> Result<u64, String> {
    use base64::Engine;
    // 兼容 data URI 前缀形态
    let payload = b64.split_once(',').map(|(_, p)| p).unwrap_or(b64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("image_base64 解码失败: {e}"))?;
    if bytes.is_empty() {
        return Err("image_base64 解码结果为空".to_string());
    }
    std::fs::write(path, &bytes).map_err(|e| format!("写入文件失败 {}: {e}", path.display()))?;
    Ok(bytes.len() as u64)
}

// ── 生成流程 ──

fn run_image_generate(
    prompt: &str,
    provider: Option<&str>,
    model: &str,
    params: Option<&Value>,
) -> Result<String, String> {
    let registry = load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
    let cred = find_minimax_credential(&registry, provider)?;
    let mut body = merge_request_body(params, model, prompt)?;
    // response_format 缺省 url（用户可在 params 中显式覆盖为 base64）
    body.entry("response_format".to_string())
        .or_insert_with(|| json!("url"));

    let agent = get_agent();
    let url = format!("{}/image_generation", cred.base_url);
    let resp = post_json(&agent, &cred, &url, &Value::Object(body))?;
    let data = resp
        .get("data")
        .ok_or_else(|| "MiniMax 图像响应缺少 data 字段".to_string())?;
    let items = collect_image_items(data);
    if items.is_empty() {
        return Err(
            "MiniMax 未返回任何图像（data.image_urls 与 data.image_base64 均为空）".to_string(),
        );
    }

    let dir = generated_dir()?;
    let ts = file_timestamp();
    let mut lines = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match item {
            ImageItem::Url(u) => {
                let path = dir.join(format!("image_{ts}_{i}.{}", image_ext_from_url(u)));
                let size = download_to_file(&agent, u, &path)?;
                lines.push(format!("{} ({} bytes)", path.display(), size));
            }
            ImageItem::Base64(b) => {
                let path = dir.join(format!("image_{ts}_{i}.jpg"));
                let size = save_base64_image(b, &path)?;
                lines.push(format!("{} ({} bytes)", path.display(), size));
            }
        }
    }
    Ok(format!(
        "已生成 {} 张图像：\n{}",
        lines.len(),
        lines.join("\n")
    ))
}

fn run_video_generate(
    prompt: &str,
    provider: Option<&str>,
    model: &str,
    params: Option<&Value>,
    poll_interval_secs: u64,
    timeout_secs: u64,
) -> Result<String, String> {
    let registry = load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
    let cred = find_minimax_credential(&registry, provider)?;
    let body = merge_request_body(params, model, prompt)?;
    let agent = get_agent();
    let root = api_root(&cred.base_url).to_string();
    let v2 = is_video_v2_model(model);

    // 1. 提交生成任务
    let (submit_url, body_value) = if v2 {
        (format!("{root}/v2/video_generation"), Value::Object(body))
    } else {
        (
            format!("{}/video_generation", cred.base_url),
            Value::Object(body),
        )
    };
    let submit_resp = post_json(&agent, &cred, &submit_url, &body_value)?;
    let task_id = submit_resp
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "MiniMax 视频响应缺少 task_id".to_string())?
        .to_string();

    // 2. 轮询任务状态（V2: 路径参数；V1: query 参数）
    let query_url = if v2 {
        format!("{root}/v2/query/video_generation/{task_id}")
    } else {
        format!("{}/query/video_generation?task_id={task_id}", cred.base_url)
    };
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let interval = Duration::from_secs(poll_interval_secs);
    let mut last_status = "unknown".to_string();
    let file_id = loop {
        if start.elapsed() >= timeout {
            return Err(format!(
                "视频生成轮询超时（task_id={task_id}，已等待 {}s，最后状态: {last_status}）",
                timeout_secs
            ));
        }
        let resp = get_json(&agent, &cred, &query_url)?;
        let status = video_task_status(&resp)
            .ok_or_else(|| "MiniMax 视频轮询响应缺少状态字段".to_string())?;
        last_status = status.clone();
        match status.to_ascii_lowercase().as_str() {
            "succeeded" | "success" => {
                break video_file_id(&resp).ok_or_else(|| {
                    format!("视频生成成功但响应缺少 file_id（task_id={task_id}）")
                })?;
            }
            "failed" | "fail" | "cancelled" | "canceled" | "expired" => {
                return Err(format!(
                    "视频生成失败（task_id={task_id}，status={status}）"
                ));
            }
            // processing/queueing/preparing 等中间态：继续等
            _ => std::thread::sleep(interval),
        }
    };

    // 3. 取下载地址
    let retrieve_url = format!("{}/files/retrieve?file_id={file_id}", cred.base_url);
    let retrieve_resp = get_json(&agent, &cred, &retrieve_url)?;
    let download_url = retrieve_resp
        .pointer("/file/download_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("files/retrieve 响应缺少 file.download_url（file_id={file_id}）"))?;

    // 4. 下载落盘
    let dir = generated_dir()?;
    let path = dir.join(format!("video_{}.mp4", file_timestamp()));
    let size = download_to_file(&agent, download_url, &path)?;
    Ok(format!(
        "视频生成完成（task_id={task_id}）：\n{} ({} bytes)",
        path.display(),
        size
    ))
}

// ── 工具注册 ──

impl ToolRegistry {
    pub(crate) fn register_image_generate(&mut self) {
        self.register(ToolDef {
            name: "image_generate".to_string(),
            description: "生成图像（MiniMax image_generation API，同步返回）。产出为本地文件路径（~/.nuphus/generated/），可被后续步骤 capture 引用。模型可填任意 MiniMax 图像模型（默认 image-01，仅便捷兜底）；params 为可选 JSON 对象，原样透传合并进 API 请求体（如 aspect_ratio/width/height/n/response_format 及一切新参数，工具不枚举不拦截；与顶层参数同名时顶层优先）。宽高官方约束 512-2048 且 8 的倍数。凭证自动取 registry 中 MiniMax provider 的 api_key，无需也不应传入密钥。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "图像生成提示词"
                    },
                    "provider": {
                        "type": "string",
                        "description": "可选。registry 中的 provider 名；默认自动发现 provider_type=minimax 且已配置 api_key 的 provider"
                    },
                    "model": {
                        "type": "string",
                        "default": DEFAULT_IMAGE_MODEL,
                        "description": "生成模型名，默认 image-01；可填任意 MiniMax 图像模型"
                    },
                    "params": {
                        "type": "object",
                        "description": "可选。透传合并进 API 请求体的生成参数（aspect_ratio/width/height/n/response_format 等）。原样发送，与顶层参数同名时顶层优先"
                    }
                },
                "required": ["prompt"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let prompt = params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    return Ok(ToolResult::failure("prompt cannot be empty"));
                }
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string());
                let model = params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(DEFAULT_IMAGE_MODEL)
                    .trim()
                    .to_string();
                let extra = params.get("params").cloned();
                super::run_blocking(move || {
                    match run_image_generate(&prompt, provider.as_deref(), &model, extra.as_ref())
                    {
                        Ok(text) => Ok(ToolResult::success(text)),
                        Err(e) => Ok(ToolResult::failure(e)),
                    }
                })
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_video_generate(&mut self) {
        self.register(ToolDef {
            name: "video_generate".to_string(),
            description: "生成视频（MiniMax video_generation API，异步任务轮询，分钟级耗时）。产出为本地 mp4 文件路径（~/.nuphus/generated/），可被后续步骤 capture 引用。模型可填任意 MiniMax 视频模型（默认 MiniMax-H3，仅便捷兜底）；V1/V2 端点按模型名自动路由（含 \"H3\" 走 V2，其余如 Hailuo-2.3/MiniMax-Hailuo-02/S2V-01 走 V1）。params 为可选 JSON 对象，原样透传合并进 API 请求体（duration/resolution/ratio/first_frame_image 及一切新参数，工具不枚举不拦截；与顶层参数同名时顶层优先）。图像输入按 MiniMax API 要求以 URL 或 data URI 形式写进 params。轮询有超时兜底（timeout_secs），超时/失败错误含 task_id 便于排查。凭证自动取 registry 中 MiniMax provider 的 api_key，无需也不应传入密钥。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "视频生成提示词"
                    },
                    "provider": {
                        "type": "string",
                        "description": "可选。registry 中的 provider 名；默认自动发现 provider_type=minimax 且已配置 api_key 的 provider"
                    },
                    "model": {
                        "type": "string",
                        "default": DEFAULT_VIDEO_MODEL,
                        "description": "生成模型名，默认 MiniMax-H3；可填任意 MiniMax 视频模型（模型名含 H3 走 V2 端点，其余走 V1）"
                    },
                    "params": {
                        "type": "object",
                        "description": "可选。透传合并进 API 请求体的生成参数（duration/resolution/ratio/first_frame_image 等）。原样发送，与顶层参数同名时顶层优先"
                    },
                    "poll_interval_secs": {
                        "type": "integer",
                        "default": DEFAULT_POLL_INTERVAL_SECS,
                        "description": "轮询间隔秒数，默认 5"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "default": DEFAULT_VIDEO_TIMEOUT_SECS,
                        "description": "轮询总超时秒数，默认 300，上限 600"
                    }
                },
                "required": ["prompt"]
            }),
            category: ToolCategory::WebSearch,
            executor: |params, _ctx| {
                let prompt = params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if prompt.is_empty() {
                    return Ok(ToolResult::failure("prompt cannot be empty"));
                }
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string());
                let model = params
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(DEFAULT_VIDEO_MODEL)
                    .trim()
                    .to_string();
                let extra = params.get("params").cloned();
                let poll_interval_secs = params
                    .get("poll_interval_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
                    .clamp(1, 60);
                let timeout_secs = params
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(DEFAULT_VIDEO_TIMEOUT_SECS)
                    .clamp(30, MAX_VIDEO_TIMEOUT_SECS);
                super::run_blocking(move || {
                    match run_video_generate(
                        &prompt,
                        provider.as_deref(),
                        &model,
                        extra.as_ref(),
                        poll_interval_secs,
                        timeout_secs,
                    ) {
                        Ok(text) => Ok(ToolResult::success(text)),
                        Err(e) => Ok(ToolResult::failure(e)),
                    }
                })
            },
            depends_on: vec![],
        });
    }
}

// ── 单测（不真调 API；真实调用验证走 ignored 测试手动执行）──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{ModelEntry, ProviderConfig};

    fn minimax_provider(name: &str, api_key: &str) -> ProviderConfig {
        ProviderConfig {
            name: name.to_string(),
            provider_type: ProviderKind::MiniMax,
            api_key: api_key.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            auth_header: String::new(),
            auth_prefix: String::new(),
            timeout_secs: 300,
            models: vec![ModelEntry {
                id: "MiniMax-M3".to_string(),
                alias: vec![],
                max_tokens: None,
                context_window: None,
                supports_streaming: true,
                supports_vision: true,
                supports_audio: false,
                supports_image_generation: false,
                reasoning_efforts: vec![],
                default_effort: None,
            }],
            reasoning_effort: None,
        }
    }

    // ── params 透传合并 ──

    #[test]
    fn merge_body_passthrough_and_top_level_wins() {
        let params = json!({
            "aspect_ratio": "16:9",
            "n": 2,
            "prompt": "should be overridden",
            "model": "should-be-overridden"
        });
        let body = merge_request_body(Some(&params), "image-01", "a cat").unwrap();
        // 透传键保留
        assert_eq!(body.get("aspect_ratio").unwrap(), &json!("16:9"));
        assert_eq!(body.get("n").unwrap(), &json!(2));
        // 顶层参数优先
        assert_eq!(body.get("prompt").unwrap(), &json!("a cat"));
        assert_eq!(body.get("model").unwrap(), &json!("image-01"));
    }

    #[test]
    fn merge_body_without_params() {
        let body = merge_request_body(None, "m", "p").unwrap();
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn merge_body_rejects_non_object_params() {
        let params = json!(["not", "an", "object"]);
        assert!(merge_request_body(Some(&params), "m", "p").is_err());
    }

    // ── V1/V2 路由判定 ──

    #[test]
    fn video_v2_routing_by_model_name() {
        assert!(is_video_v2_model("MiniMax-H3"));
        assert!(is_video_v2_model("some-H3-variant"));
        assert!(!is_video_v2_model("Hailuo-2.3"));
        assert!(!is_video_v2_model("MiniMax-Hailuo-02"));
        assert!(!is_video_v2_model("S2V-01"));
        assert!(!is_video_v2_model("h3-lowercase")); // 大小写敏感，对齐官方 cli
    }

    #[test]
    fn api_root_strips_v1_suffix() {
        assert_eq!(
            api_root("https://api.minimaxi.com/v1"),
            "https://api.minimaxi.com"
        );
        assert_eq!(
            api_root("https://api.minimaxi.com/v1/"),
            "https://api.minimaxi.com"
        );
        assert_eq!(
            api_root("https://api.minimax.io/v1"),
            "https://api.minimax.io"
        );
    }

    // ── 错误响应解析 ──

    #[test]
    fn extract_api_error_nonzero_status() {
        let body = json!({"base_resp": {"status_code": 1002, "status_msg": "invalid api key"}});
        let err = extract_api_error(&body).unwrap();
        assert!(err.contains("1002"));
        assert!(err.contains("invalid api key"));
    }

    #[test]
    fn extract_api_error_zero_or_missing() {
        assert!(
            extract_api_error(&json!({"base_resp": {"status_code": 0, "status_msg": "ok"}}))
                .is_none()
        );
        assert!(extract_api_error(&json!({"data": {}})).is_none());
    }

    // ── 图像响应解析 ──

    #[test]
    fn collect_image_items_urls_and_base64() {
        let data = json!({"image_urls": ["https://x/a.png", ""], "image_base64": ["aGVsbG8="]});
        let items = collect_image_items(&data);
        assert_eq!(items.len(), 2); // 空串被过滤
        assert!(matches!(items[0], ImageItem::Url(_)));
        assert!(matches!(items[1], ImageItem::Base64(_)));
    }

    #[test]
    fn collect_image_items_empty() {
        assert!(collect_image_items(&json!({})).is_empty());
    }

    // ── 视频状态 / file_id 解析（V1/V2 两种形态）──

    #[test]
    fn video_status_v2_nested_and_v1_flat() {
        let v2 = json!({"task": {"status": "succeeded", "file_id": "f-1"}});
        assert_eq!(video_task_status(&v2).unwrap(), "succeeded");
        assert_eq!(video_file_id(&v2).unwrap(), "f-1");
        let v1 = json!({"status": "Success", "file_id": "f-2"});
        assert_eq!(video_task_status(&v1).unwrap(), "Success");
        assert_eq!(video_file_id(&v1).unwrap(), "f-2");
    }

    // ── 凭证解析错误路径 ──

    #[test]
    fn credential_missing_minimax_provider() {
        let registry = ModelRegistry::default();
        // 不用 unwrap_err：GenCredential 含 api_key，刻意不实现 Debug 防泄露
        let err = match find_minimax_credential(&registry, None) {
            Err(e) => e,
            Ok(_) => panic!("empty registry should yield no credential"),
        };
        assert!(err.contains("MiniMax"), "error should name MiniMax: {err}");
    }

    #[test]
    fn credential_empty_api_key_rejected() {
        let mut registry = ModelRegistry::default();
        registry.providers.push(minimax_provider("minimax", "  "));
        assert!(find_minimax_credential(&registry, None).is_err());
    }

    #[test]
    fn credential_provider_name_filter() {
        let mut registry = ModelRegistry::default();
        registry.providers.push(minimax_provider("minimax", "k1"));
        // 指定不存在的 provider 名 → 报错且含名字
        let err = match find_minimax_credential(&registry, Some("other")) {
            Err(e) => e,
            Ok(_) => panic!("unknown provider name should yield no credential"),
        };
        assert!(err.contains("other"));
        // 指定存在的名 → 命中，base_url 去尾斜杠
        let cred = find_minimax_credential(&registry, Some("minimax")).unwrap();
        assert_eq!(cred.base_url, "https://api.minimaxi.com/v1");
    }

    #[test]
    fn credential_base_url_fallback() {
        let mut registry = ModelRegistry::default();
        let mut p = minimax_provider("minimax", "k1");
        p.base_url = String::new();
        registry.providers.push(p);
        let cred = find_minimax_credential(&registry, None).unwrap();
        assert_eq!(cred.base_url, DEFAULT_BASE_URL);
    }

    // ── wf_tools 包含性：注册表含两工具、prompt 必填、不在工作流排除名单 ──

    #[test]
    fn workflow_tool_inclusion() {
        use crate::tools::registry::is_workflow_step_tool;
        assert!(is_workflow_step_tool("image_generate"));
        assert!(is_workflow_step_tool("video_generate"));

        let registry = ToolRegistry::builtin();
        for name in ["image_generate", "video_generate"] {
            let def = registry
                .get(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            let required = def
                .parameters
                .get("required")
                .and_then(|v| v.as_array())
                .expect("input_schema should declare required");
            assert!(
                required.iter().any(|v| v.as_str() == Some("prompt")),
                "{name} input_schema should require prompt"
            );
        }
    }

    // ── 真实 API 验证（手动执行，需 registry 已配置 MiniMax key）──
    // cargo test -p nuphus generation::tests::image_generate_real_api -- --ignored --nocapture

    #[test]
    #[ignore = "real MiniMax API call — run manually"]
    fn image_generate_real_api() {
        let registry = ToolRegistry::builtin();
        let def = registry.get("image_generate").unwrap();
        let result = (def.executor)(
            &json!({"prompt": "a cute cat sitting on a windowsill, warm morning light"}),
            &crate::tools::registry::ToolCtx::default(),
        )
        .unwrap();
        assert!(result.success, "image_generate failed: {:?}", result.error);
        println!("{}", result.output.unwrap());
    }
}
