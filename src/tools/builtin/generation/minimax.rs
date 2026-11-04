//! minimax — MiniMax 媒体生成协议（`MiniMaxMediaProtocol`）
//!
//! 端点、请求键名、响应解析、V1/V2 路由只活在本文件；语义层与产品契约层
//! （工具 description / workflow schema / UI / skill 文档）看不到任何这里的细节。
//! 新增第二家提供商时仿照本文件写一个协议实现，再在 `super::GENERATION_PROTOCOLS`
//! 注册一行即可。
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

use super::artifacts::{self, Payload};
use super::{
    Artifact, GenError, GenTarget, GenerationProtocol, GenerationRequest,
    DEFAULT_POLL_INTERVAL_SECS, DEFAULT_VIDEO_TIMEOUT_SECS,
};
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

/// MiniMax 默认 base（仅当绑定的 provider 是 MiniMax 段且未配 base_url 时兜底，
/// 与 config::providers::minimax 保持一致）；别的协议没有内置端点。
const DEFAULT_BASE_URL: &str = "https://api.minimaxi.com/v1";
/// 错误响应体透出截断长度
const ERROR_BODY_CAP: usize = 500;

/// MiniMax 媒体生成协议：图像（同步）+ 视频（异步任务轮询）。
pub(crate) struct MiniMaxMediaProtocol;

impl GenerationProtocol for MiniMaxMediaProtocol {
    fn kind(&self) -> &'static str {
        "minimax_media"
    }

    fn default_base_url(&self) -> Option<&'static str> {
        Some(DEFAULT_BASE_URL)
    }

    fn image(
        &self,
        target: &GenTarget,
        req: &GenerationRequest,
    ) -> Result<Vec<Artifact>, GenError> {
        let body = image_body(req, &target.model);
        let agent = artifacts::get_agent();
        let url = format!("{}/image_generation", target.base_url);
        let resp = post_json(&agent, target, &url, &Value::Object(body))?;
        let data = resp
            .get("data")
            .ok_or_else(|| GenError::protocol("图像响应缺少 data 字段"))?;
        let items = collect_image_payloads(data);
        if items.is_empty() {
            return Err(GenError::empty_result(
                "未返回任何图像（data.image_urls 与 data.image_base64 均为空）",
            ));
        }
        artifacts::save_image_payloads(&agent, &items)
    }

    fn video(
        &self,
        target: &GenTarget,
        req: &GenerationRequest,
    ) -> Result<Vec<Artifact>, GenError> {
        let model = target.model.as_str();
        let body = video_body(req, model);
        let agent = artifacts::get_agent();
        let root = api_root(&target.base_url).to_string();
        let v2 = is_video_v2_model(model);

        // 1. 提交生成任务
        let submit_url = if v2 {
            format!("{root}/v2/video_generation")
        } else {
            format!("{}/video_generation", target.base_url)
        };
        let submit_resp = post_json(&agent, target, &submit_url, &Value::Object(body))?;
        let task_id = submit_resp
            .get("task_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| GenError::protocol("视频响应缺少 task_id"))?
            .to_string();

        // 2. 轮询任务状态（V2: 路径参数；V1: query 参数）
        let query_url = if v2 {
            format!("{root}/v2/query/video_generation/{task_id}")
        } else {
            format!(
                "{}/query/video_generation?task_id={task_id}",
                target.base_url
            )
        };
        let start = Instant::now();
        let timeout = Duration::from_secs(req.timeout_secs.unwrap_or(DEFAULT_VIDEO_TIMEOUT_SECS));
        let interval =
            Duration::from_secs(req.poll_interval_secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS));
        let mut last_status = "unknown".to_string();
        let file_id = loop {
            if start.elapsed() >= timeout {
                return Err(GenError::timeout(format!(
                    "视频生成轮询超时（task_id={task_id}，已等待 {}s，最后状态: {last_status}）",
                    timeout.as_secs()
                )));
            }
            let resp = get_json(&agent, target, &query_url)?;
            let status = video_task_status(&resp)
                .ok_or_else(|| GenError::protocol("视频轮询响应缺少状态字段"))?;
            last_status = status.clone();
            match status.to_ascii_lowercase().as_str() {
                "succeeded" | "success" => {
                    break video_file_id(&resp).ok_or_else(|| {
                        GenError::protocol(format!(
                            "视频生成成功但响应缺少 file_id（task_id={task_id}）"
                        ))
                    })?;
                }
                "failed" | "fail" | "cancelled" | "canceled" | "expired" => {
                    return Err(GenError::protocol(format!(
                        "视频生成失败（task_id={task_id}，status={status}）"
                    )));
                }
                // processing/queueing/preparing 等中间态：继续等
                _ => std::thread::sleep(interval),
            }
        };

        // 3. 取下载地址
        let retrieve_url = format!("{}/files/retrieve?file_id={file_id}", target.base_url);
        let retrieve_resp = get_json(&agent, target, &retrieve_url)?;
        let download_url = retrieve_resp
            .pointer("/file/download_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                GenError::protocol(format!(
                    "files/retrieve 响应缺少 file.download_url（file_id={file_id}）"
                ))
            })?;

        // 4. 下载落盘
        Ok(vec![artifacts::save_video_download(
            &agent,
            download_url,
            &task_id,
        )?])
    }
}

// ── 语义字段 → MiniMax 请求体键名（映射只活在本文件）──

/// 公共请求体：`extra` 透传为底，顶层参数（model/prompt）覆盖同名键。
fn base_body(req: &GenerationRequest, model: &str) -> Map<String, Value> {
    let mut body = req.extra.clone();
    // 顶层参数优先：后插入覆盖 extra 同名键
    body.insert("model".to_string(), json!(model));
    body.insert("prompt".to_string(), json!(req.prompt));
    body
}

/// 图像请求体：`ratio→aspect_ratio`、`count→n`、`width→width`、`height→height`、
/// `output_format→response_format`（缺省 url）。
///
/// 模态归属与 schema description 一致：`duration` / `first_frame` 属 video，
/// 不注入图像请求体（协议自行忽略不适用的语义字段）。
fn image_body(req: &GenerationRequest, model: &str) -> Map<String, Value> {
    let mut body = base_body(req, model);
    // 语义字段：只在 extra 没有显式同名键时注入（逃生舱优先）
    if let Some(ratio) = &req.ratio {
        body.entry("aspect_ratio".to_string())
            .or_insert_with(|| json!(ratio));
    }
    if let Some(count) = req.count {
        body.entry("n".to_string()).or_insert_with(|| json!(count));
    }
    if let Some(width) = req.width {
        body.entry("width".to_string())
            .or_insert_with(|| json!(width));
    }
    if let Some(height) = req.height {
        body.entry("height".to_string())
            .or_insert_with(|| json!(height));
    }
    if let Some(output_format) = &req.output_format {
        body.entry("response_format".to_string())
            .or_insert_with(|| json!(output_format));
    }
    // response_format 缺省 url（用户可在 extra 中显式覆盖为 base64）
    body.entry("response_format".to_string())
        .or_insert_with(|| json!("url"));
    body
}

/// 视频请求体：`ratio→aspect_ratio`、`duration→duration`、
/// `first_frame→first_frame_image`（空串 = 文生视频，不注入键）。
///
/// `count` / `width` / `height` / `output_format` 属 image，不注入视频请求体。
fn video_body(req: &GenerationRequest, model: &str) -> Map<String, Value> {
    let mut body = base_body(req, model);
    if let Some(ratio) = &req.ratio {
        body.entry("aspect_ratio".to_string())
            .or_insert_with(|| json!(ratio));
    }
    if let Some(duration) = req.duration {
        body.entry("duration".to_string())
            .or_insert_with(|| json!(duration));
    }
    // 空串 = 文生视频：不注入键，让生成端按自己的缺省走
    if let Some(first_frame) = req
        .first_frame
        .as_ref()
        .filter(|path| !path.trim().is_empty())
    {
        body.entry("first_frame_image".to_string())
            .or_insert_with(|| json!(first_frame));
    }
    body
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
    Some(format!("API error (status_code={code}): {msg}"))
}

/// 从 image_generation 响应的 data 字段提取图像载体
/// （`image_urls[]` 与 `image_base64[]` 两种形态都接）。
fn collect_image_payloads(data: &Value) -> Vec<Payload> {
    let mut items = Vec::new();
    if let Some(urls) = data.get("image_urls").and_then(|v| v.as_array()) {
        for u in urls.iter().filter_map(|v| v.as_str()) {
            if !u.is_empty() {
                items.push(Payload::Url(u.to_string()));
            }
        }
    }
    if let Some(b64s) = data.get("image_base64").and_then(|v| v.as_array()) {
        for b in b64s.iter().filter_map(|v| v.as_str()) {
            if !b.is_empty() {
                items.push(Payload::Base64(b.to_string()));
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

/// 响应文本读取并做业务错误解析（base_resp 透出，不吞错误体）。
fn parse_response(resp: reqwest::blocking::Response, endpoint: &str) -> Result<Value, GenError> {
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| GenError::transport(format!("读取响应体失败 ({endpoint}): {e}")))?;
    if !status.is_success() {
        let truncated: String = text.chars().take(ERROR_BODY_CAP).collect();
        return Err(GenError::transport(format!(
            "API HTTP {status} ({endpoint}): {truncated}"
        )));
    }
    let body: Value = serde_json::from_str(&text).map_err(|e| {
        let truncated: String = text.chars().take(ERROR_BODY_CAP).collect();
        GenError::protocol(format!("响应非 JSON ({endpoint}): {e} — body: {truncated}"))
    })?;
    if let Some(err) = extract_api_error(&body) {
        return Err(GenError::protocol(format!("{err} ({endpoint})")));
    }
    Ok(body)
}

fn post_json(
    agent: &reqwest::blocking::Client,
    cred: &GenTarget,
    url: &str,
    body: &Value,
) -> Result<Value, GenError> {
    let resp = agent
        .post(url)
        .bearer_auth(&cred.api_key)
        .json(body)
        .send()
        .map_err(|e| GenError::transport(artifacts::classify_reqwest_error(&e)))?;
    parse_response(resp, url)
}

fn get_json(
    agent: &reqwest::blocking::Client,
    cred: &GenTarget,
    url: &str,
) -> Result<Value, GenError> {
    let resp = agent
        .get(url)
        .bearer_auth(&cred.api_key)
        .send()
        .map_err(|e| GenError::transport(artifacts::classify_reqwest_error(&e)))?;
    parse_response(resp, url)
}

// ── 单测（语义→厂商请求体的映射测试就该在这里）──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GenerationKind;

    fn image_request() -> GenerationRequest {
        GenerationRequest::new(GenerationKind::Image, "a cat")
    }

    fn video_request() -> GenerationRequest {
        GenerationRequest::new(GenerationKind::Video, "a dog running")
    }

    #[test]
    fn image_body_maps_semantic_fields_to_vendor_keys() {
        let mut req = image_request();
        req.ratio = Some("16:9".to_string());
        req.count = Some(2);
        req.width = Some(1024);
        req.height = Some(1024);
        req.output_format = Some("base64".to_string());
        // video 专属语义字段：不得混进图像请求体
        req.duration = Some(6);
        req.first_frame = Some("/tmp/frame.png".to_string());

        let body = image_body(&req, "image-01");
        assert_eq!(body.get("model"), Some(&json!("image-01")));
        assert_eq!(body.get("prompt"), Some(&json!("a cat")));
        assert_eq!(body.get("aspect_ratio"), Some(&json!("16:9")));
        assert_eq!(body.get("n"), Some(&json!(2)));
        assert_eq!(body.get("width"), Some(&json!(1024)));
        assert_eq!(body.get("height"), Some(&json!(1024)));
        assert_eq!(body.get("response_format"), Some(&json!("base64")));
        assert!(
            body.get("duration").is_none(),
            "duration 属 video，不得混进 image 请求体"
        );
        assert!(
            body.get("first_frame_image").is_none(),
            "first_frame 属 video，不得混进 image 请求体"
        );
        // response_format 缺省 url（可在 extra 里显式覆盖）
        assert_eq!(
            image_body(&image_request(), "m").get("response_format"),
            Some(&json!("url"))
        );
    }

    #[test]
    fn video_body_maps_semantic_fields_to_vendor_keys() {
        let mut req = video_request();
        req.ratio = Some("9:16".to_string());
        req.duration = Some(10);
        req.first_frame = Some("{{frame}}".to_string());
        // image 专属语义字段：不得混进视频请求体
        req.count = Some(2);
        req.width = Some(512);
        req.output_format = Some("url".to_string());

        let body = video_body(&req, "video-01");
        assert_eq!(body.get("model"), Some(&json!("video-01")));
        assert_eq!(body.get("prompt"), Some(&json!("a dog running")));
        assert_eq!(body.get("aspect_ratio"), Some(&json!("9:16")));
        assert_eq!(body.get("duration"), Some(&json!(10)));
        assert_eq!(body.get("first_frame_image"), Some(&json!("{{frame}}")));
        assert!(
            body.get("n").is_none(),
            "count 属 image，不得混进 video 请求体"
        );
        assert!(body.get("width").is_none(), "width 属 image");
        assert!(
            body.get("response_format").is_none(),
            "视频请求体不注入 response_format 缺省"
        );
    }

    #[test]
    fn video_body_skips_blank_first_frame() {
        let mut req = video_request();
        req.first_frame = Some("   ".to_string());
        assert!(video_body(&req, "m").get("first_frame_image").is_none());
    }

    #[test]
    fn extra_explicit_keys_win_over_semantic_fields() {
        let mut req = image_request();
        req.ratio = Some("1:1".to_string());
        req.count = Some(1);
        req.extra = GenerationRequest::extra_from_params(Some(&json!({
            "aspect_ratio": "16:9",
            "width": 1024
        })))
        .expect("object params");

        let body = image_body(&req, "m");
        assert_eq!(
            body.get("aspect_ratio"),
            Some(&json!("16:9")),
            "extra 显式键覆盖具名语义字段"
        );
        assert_eq!(body.get("width"), Some(&json!(1024)));
        assert_eq!(
            body.get("n"),
            Some(&json!(1)),
            "extra 未指定的具名字段仍生效"
        );
    }

    #[test]
    fn top_level_model_and_prompt_override_extra() {
        let mut req = image_request();
        req.extra = GenerationRequest::extra_from_params(Some(&json!({
            "prompt": "should be overridden",
            "model": "should-be-overridden"
        })))
        .expect("object params");

        let body = image_body(&req, "image-01");
        assert_eq!(body.get("prompt"), Some(&json!("a cat")));
        assert_eq!(body.get("model"), Some(&json!("image-01")));
    }

    #[test]
    fn collect_image_payloads_urls_and_base64() {
        let data = json!({"image_urls": ["https://x/a.png", ""], "image_base64": ["aGVsbG8="]});
        let items = collect_image_payloads(&data);
        assert_eq!(items.len(), 2); // 空串被过滤
        assert!(matches!(items[0], Payload::Url(_)));
        assert!(matches!(items[1], Payload::Base64(_)));
    }

    #[test]
    fn collect_image_payloads_empty() {
        assert!(collect_image_payloads(&json!({})).is_empty());
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
}
