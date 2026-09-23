//! Vision OCR via user-configured image understanding model
//!
//! Sends screenshot to user's configured vision model (capabilities.vision).
//!
//! 传输层复用：本模块**不**自行拼 URL / 鉴权头 / JSON body，而是取出模型所在段的
//! `Provider::transport()`（协议族由 Provider 自身声明：OpenAI 兼容自定义/中转段走
//! Chat Completions，`provider_type = "anthropic"` 走 Messages API），把图片作为
//! 一条 user 消息交给传输层。这样自定义/中转 Provider 与官方 Provider 的请求参数、
//! 鉴权、重试、超时、代理回退完全一致——不再存在第二套旁路实现。
//!
//! `stream = true`：部分 OpenAI 兼容中转端点在非流式模式下不返回图片结果（要求
//! 流式）；传输层用 `parse_sse` 解析 SSE，Anthropic 传输层同样按 SSE 解析，两者
//! 返回统一的 `Vec<StreamEvent>`。
//!
//! `max_tokens` 仅在模型段显式配置了 `ModelEntry.max_tokens` 时下发；未配置则不
//! 带该字段，由传输层回退链（`resolve_max_output_tokens` → 服务端默认上限）决定，
//! 与本仓其它 LLM 调用同一口径。
//!
//! Returns `Result<String, String>` — Ok(text) on success, Err(message) on failure.
//! Errors are wrapped into NuphusError::Tool by the caller (client::ocr).

use crate::api::MessageRequest;
use crate::config::{self, resolve_vision_provider, resolve_vision_strategy, VisionStrategy};

fn resolve_vision_model<'a>(
    registry: &'a config::ModelRegistry,
    model_id: &str,
    provider: Option<&str>,
) -> Result<(&'a config::ProviderConfig, &'a config::ModelEntry), String> {
    if let Some(provider) = provider.filter(|p| !p.is_empty()) {
        registry
            .find_model_for_provider(provider, model_id)
            .ok_or_else(|| format!("未找到视觉模型: {provider}/{model_id}"))
    } else {
        registry
            .find_model(model_id)
            .ok_or_else(|| format!("未找到视觉模型: {model_id}"))
    }
}

/// OCR via vision model (image content block + unified transport layer)
///
/// Loads the user's vision model config from capabilities.vision in the
/// model registry, encodes the BMP/PNG image as base64 data URL, and sends it
/// through the model segment's `Provider::transport()`.
///
/// 对外行为不变：接受文件路径，内部读取文件 → data URL → 直调内部函数。
pub async fn vision_ocr(image_path: &str, prompt: Option<&str>) -> Result<String, String> {
    // 4. Read image — convert BMP to PNG (LLM APIs don't support image/bmp)
    let image_bytes = std::fs::read(image_path).map_err(|e| format!("读取图片失败: {e}"))?;
    let (mime_type, final_bytes) = if image_path.to_lowercase().ends_with(".png") {
        ("image/png", image_bytes)
    } else {
        let img =
            image::load_from_memory(&image_bytes).map_err(|e| format!("解析图片失败: {e}"))?;
        let mut png_buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png_buf, image::ImageFormat::Png)
            .map_err(|e| format!("转换PNG失败: {e}"))?;
        ("image/png", png_buf.into_inner())
    };
    let base64_image = base64_encode(&final_bytes);
    let data_url = format!("data:{mime_type};base64,{base64_image}");

    vision_ocr_data_url(&data_url, prompt).await
}

/// 视觉模型直调（data URL 版本）
///
/// 用户消息图片在 session 中已是冻结的 base64 data URL（BMP→PNG 在入 session 时
/// 已完成），无需再落临时文件。此函数复用 vision_ocr 的完整调用链，供
/// desktop_vision 工具与按需图片查看使用。
///
/// 协议解析（Chat Completions / Anthropic Messages）由对应 Provider 的传输层负责。
pub async fn vision_ocr_data_url(data_url: &str, prompt: Option<&str>) -> Result<String, String> {
    let vision_model_id = resolve_vision_model_id()?;

    // 2. 加载 registry 解析模型配置
    let registry = config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;

    // 3. Resolve model alias to find provider config
    let vision_provider = resolve_vision_provider();
    let (provider_config, model_entry) =
        resolve_vision_model(&registry, &vision_model_id, vision_provider.as_deref())?;

    // 4. 该模型段的传输层（协议族由 Provider 自身声明；鉴权头 / 端点 / 重试 /
    //    超时 / 代理回退与该模型的常规 LLM 调用完全同源）
    let provider = config::registry::ProviderRegistry::builtin()
        .get(provider_config.provider_type.as_str())
        .ok_or_else(|| {
            format!(
                "未找到内置 Provider: {}",
                provider_config.provider_type.as_str()
            )
        })?;
    let transport = provider.transport(provider_config, &model_entry.id);

    // 5. 解析 data URL → mime_type + base64 载荷
    let (mime_type, base64_image) = split_data_url(data_url)?;

    // 6. 图片内容块：Anthropic 原生 image source block / OpenAI image_url
    //    （图片对两种协议都是「按 URL 取图」的远程资源，故统一用 data URL 形式传递）
    let prompt_text = prompt
        .filter(|p| !p.is_empty())
        .unwrap_or("请识别并输出这张图片中的所有文字，只输出文字内容，不要添加任何解释。");

    let content = if provider_config.provider_type == crate::api::ProviderKind::Anthropic {
        serde_json::json!([
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": mime_type,
                    "data": base64_image
                }
            },
            { "type": "text", "text": prompt_text }
        ])
    } else {
        serde_json::json!([
            { "type": "text", "text": prompt_text },
            {
                "type": "image_url",
                "image_url": {
                    "url": data_url,
                    "detail": "high"
                }
            }
        ])
    };

    // temperature 不传：部分模型（如 Kimi 推理系）强制 temperature=1，传 0.0 会被 400 拒绝
    let mut request = MessageRequest::new(
        &model_entry.id,
        vec![serde_json::json!({
            "role": "user",
            "content": content,
        })],
    );
    // max_tokens：仅显式配置时下发（未配置则由传输层回退链决定，见模块文档）
    if let Some(max_tokens) = model_entry.max_tokens {
        request = request.with_max_tokens(max_tokens);
    }
    // 流式：中转端点要求（非流式不返回图片结果），传输层按 SSE 解析
    request = request.with_stream(true);

    // 7. 经统一传输层发起请求并解析为统一事件流
    let events = transport
        .stream(request)
        .await
        .map_err(|e| format!("视觉模型请求失败: {e}"))?;

    // 8. 事件流 → 纯文本（TextDelta 拼接；Error 不吞）
    events_to_text(events).map_err(|e| format!("视觉模型调用失败: {e}"))
}

/// 统一事件流 → 文本。
///
/// - 所有 `TextDelta` 按到达顺序拼接（SSE 分片已由传输层合并）；
/// - 任一 `Error(e)` 直接失败——视觉调用的失败必须让 Leader 看到真实原因，
///   不能当作「空结果」静默降级；
/// - 空文本单独报错（模型拒答 / 事件流里没有可见文本），避免把空串当成功交付。
fn events_to_text(events: Vec<crate::transports::StreamEvent>) -> Result<String, String> {
    let mut text = String::new();
    for event in events {
        match event {
            crate::transports::StreamEvent::TextDelta(delta) => text.push_str(&delta),
            crate::transports::StreamEvent::Error(e) => return Err(e),
            _ => {}
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("视觉模型未返回任何文本内容".to_string());
    }
    Ok(text.to_string())
}

/// 使用统一判定获取视觉模型 ID（desktop_vision 工具与描述注入共用）
fn resolve_vision_model_id() -> Result<String, String> {
    match resolve_vision_strategy() {
        VisionStrategy::Main => {
            let registry = config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
            Ok(registry.model.clone())
        }
        VisionStrategy::Capability(m) => Ok(m),
        // 不可用时给**真实的、可执行**的说明：讲清原因（Leader 不支持视觉）+ 两条出路。
        // 这段文本会随工具结果进入 Leader 上下文——是「当前能力确实不可用」的事实告知，
        // 不是静默降级：Leader 看到后应如实告诉用户，而不是假装看到了图像内容。
        VisionStrategy::None => {
            let leader = config::load_registry().map(|r| r.model).unwrap_or_default();
            let leader_part = if leader.is_empty() {
                "当前 Leader 模型不支持视觉输入".to_string()
            } else {
                format!("当前 Leader 模型（{leader}）不支持视觉输入")
            };
            Err(format!(
                "图像理解不可用：{leader_part}，且未配置图像理解模型。\n\
                 请在 Nuphus 设置 → 模型 → 图像音频模型 中指定一个图像理解模型，或把 Leader 切换到支持视觉的模型。\n\
                 在解决之前，涉及截图/图像识别的桌面操作无法完成——请如实告知用户，不要推测图像内容。"
            ))
        }
    }
}

/// 解析 data URL 为 (mime_type, base64 载荷)
fn split_data_url(data_url: &str) -> Result<(String, String), String> {
    // 格式: data:<mime>;base64,<payload>
    let (header, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "Invalid data URL: no comma found".to_string())?;
    let mime_type = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .to_string();
    Ok((mime_type, payload.to_string()))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transports::StreamEvent;

    fn duplicate_model_registry() -> config::ModelRegistry {
        toml::from_str(
            r#"
[[providers]]
name = "first"
provider_type = "custom"
api_key = "first-key"
base_url = "https://first.example/v1"

[[providers.models]]
id = "shared-model"

[[providers]]
name = "selected"
provider_type = "custom"
api_key = "selected-key"
base_url = "https://selected.example/v1"

[[providers.models]]
id = "shared-model"
"#,
        )
        .unwrap()
    }

    #[test]
    fn vision_model_uses_configured_provider_for_duplicate_ids() {
        let registry = duplicate_model_registry();
        let (provider, model) =
            resolve_vision_model(&registry, "shared-model", Some("selected")).unwrap();
        assert_eq!(provider.name, "selected");
        assert_eq!(model.id, "shared-model");
    }

    #[test]
    fn vision_model_keeps_legacy_first_match_without_provider() {
        let registry = duplicate_model_registry();
        let (provider, _) = resolve_vision_model(&registry, "shared-model", None).unwrap();
        assert_eq!(provider.name, "first");
    }

    #[test]
    fn events_join_all_text_deltas() {
        let events = vec![
            StreamEvent::Reasoning("thinking".to_string()),
            StreamEvent::TextDelta("第一行".to_string()),
            StreamEvent::TextDelta("\n第二行".to_string()),
            StreamEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cache_hit_tokens: 0,
            },
            StreamEvent::Done,
        ];
        assert_eq!(events_to_text(events).unwrap(), "第一行\n第二行");
    }

    #[test]
    fn events_error_event_is_not_swallowed() {
        let events = vec![
            StreamEvent::TextDelta("部分".to_string()),
            StreamEvent::Error("HTTP 400: bad request".to_string()),
        ];
        assert_eq!(events_to_text(events).unwrap_err(), "HTTP 400: bad request");
    }

    #[test]
    fn events_empty_text_is_an_error() {
        let events = vec![StreamEvent::Done];
        assert!(events_to_text(events).is_err());
    }
}
