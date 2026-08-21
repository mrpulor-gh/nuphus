//! Vision OCR via user-configured image understanding model
//!
//! Sends screenshot to user's configured vision model (capabilities.vision).
//! Supports two protocols, chosen by `provider_type`:
//! - **OpenAI-compatible** Chat Completions (default) — `image_url` content.
//! - **Anthropic native** Messages API (`provider_type = "anthropic"`) — `/v1/messages`,
//!   `x-api-key` + `anthropic-version`, native `image` source block.
//!
//! `max_tokens` defaults to 1024 — the compatibility floor for Chinese OpenAI-compatible
//! providers (Zhipu GLM-4V-Flash rejects anything above 1024 with HTTP 400). Override per
//! model via `ModelEntry.max_tokens` in the model registry.
//!
//! Returns `Result<String, String>` — Ok(text) on success, Err(message) on failure.
//! Errors are wrapped into NuphusError::Tool by the caller (client::ocr).

use crate::api::ProviderKind;
use crate::config::{self, resolve_vision_strategy, VisionStrategy};

/// OCR via vision model (Chat Completions API with image_url)
///
/// Loads the user's vision model config from capabilities.vision in the
/// model registry, encodes the BMP/PNG image as base64 data URL, and sends it
/// to the model's OpenAI-compatible endpoint.
///
/// 对外行为不变：接受文件路径，内部读取文件 → data URL → 直调内部函数。
pub fn vision_ocr(image_path: &str, prompt: Option<&str>) -> Result<String, String> {
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

    vision_ocr_data_url(&data_url, prompt)
}

/// 视觉模型直调（data URL 版本）
///
/// 用户消息图片在 session 中已是冻结的 base64 data URL（BMP→PNG 在入 session 时
/// 已完成），无需再落临时文件。此函数复用 vision_ocr 的完整调用链，供
/// desktop_vision 工具与按需图片查看使用。
pub fn vision_ocr_data_url(data_url: &str, prompt: Option<&str>) -> Result<String, String> {
    let vision_model_id = resolve_vision_model_id()?;

    // 2. 加载 registry 解析模型配置
    let registry = config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;

    // 3. Resolve model alias to find provider config
    let (provider_config, model_entry) = registry
        .find_model(&vision_model_id)
        .ok_or_else(|| format!("未找到视觉模型: {vision_model_id}"))?;

    // 4. 从内置 ProviderRegistry 获取正确的 auth_header/auth_prefix
    //    TOML ProviderConfig 默认为空，应使用 Provider trait 定义的认证方式
    let builtin_registry = crate::config::registry::ProviderRegistry::builtin();
    let builtin_provider = builtin_registry
        .get(provider_config.provider_type.as_str())
        .ok_or_else(|| {
            format!(
                "未找到内置 Provider: {}",
                provider_config.provider_type.as_str()
            )
        })?;
    let auth_header = builtin_provider.auth_header();
    let auth_prefix = builtin_provider.auth_prefix();

    // 5. 解析 data URL → mime_type + base64 载荷
    let (mime_type, base64_image) = split_data_url(data_url)?;

    // 6. Build request body (OpenAI-compatible or Anthropic native Messages)
    let prompt_text = prompt
        .filter(|p| !p.is_empty())
        .unwrap_or("请识别并输出这张图片中的所有文字，只输出文字内容，不要添加任何解释。");

    // max_tokens 默认 1024：国产 OpenAI 兼容视觉模型（智谱 GLM-4V-Flash 等）上限就是 1024，
    // 硬编码更高值会返回 HTTP 400。按模型在注册表配置 `ModelEntry.max_tokens` 可调高。
    let max_tokens = model_entry.max_tokens.unwrap_or(1024);
    let is_anthropic = provider_config.provider_type == ProviderKind::Anthropic;

    let body = if is_anthropic {
        serde_json::json!({
            "model": model_entry.id,
            "max_tokens": max_tokens,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": mime_type,
                                "data": base64_image
                            }
                        },
                        { "type": "text", "text": prompt_text }
                    ]
                }
            ]
        })
    } else {
        serde_json::json!({
            "model": model_entry.id,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": prompt_text
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": data_url,
                                "detail": "high"
                            }
                        }
                    ]
                }
            ],
            "max_tokens": max_tokens
            // temperature 不传：部分模型（如 Kimi 推理系）强制 temperature=1，传 0.0 会被 400 拒绝
        })
    };

    // 7. Make HTTP POST request
    let client = reqwest::blocking::Client::new();
    let base = provider_config.base_url.trim_end_matches('/');
    let url = if is_anthropic {
        format!("{base}/v1/messages")
    } else {
        format!("{base}/chat/completions")
    };
    let auth_value = if auth_prefix.is_empty() {
        provider_config.api_key.clone()
    } else {
        format!("{}{}", auth_prefix, provider_config.api_key)
    };

    let mut request = client
        .post(&url)
        .header(auth_header, &auth_value)
        .header("Content-Type", "application/json");
    if is_anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = request
        .json(&body)
        .send()
        .map_err(|e| format!("视觉模型请求失败: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let err_body = response.text().unwrap_or_default();
        return Err(format!("视觉模型返回错误 HTTP {status}: {err_body}"));
    }

    // 8. Parse response
    let resp_json: serde_json::Value = response
        .json()
        .map_err(|e| format!("解析视觉模型响应失败: {e}"))?;

    let text = if is_anthropic {
        // Anthropic: join content[*] text blocks (may be split across multiple blocks)
        resp_json["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        resp_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string()
    };

    Ok(text)
}

/// 使用统一判定获取视觉模型 ID（desktop_vision 工具与描述注入共用）
fn resolve_vision_model_id() -> Result<String, String> {
    match resolve_vision_strategy() {
        VisionStrategy::Main => {
            let registry = config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
            Ok(registry.model.clone())
        }
        VisionStrategy::Capability(m) => Ok(m),
        VisionStrategy::None => Err(
            "未配置图像理解模型。请在 Nuphus 设置 → 模型 → 自定义配置 中选择视觉模型。".to_string(),
        ),
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
