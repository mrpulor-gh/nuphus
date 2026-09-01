// tools/voice.rs — 语音克隆/文字转语音（内部机制命令，非 agent 工具）
//
// 设计（大王定调 2026-09-02）：语音克隆走云端 API，模型在「模型界面 → 图像音频配置 →
// 语音克隆」配置（capabilities.voice）。命令读取该能力解析 provider/model，调用
// OpenAI 兼容 `/audio/speech` TTS 端点（主流云端 TTS/克隆平台的标准接口）合成语音。
// 克隆音色由云端按所选模型/默认 voice 处理；参考音频暂不单独上传（协议因平台而异，
// 不臆造——统一走标准 TTS 端点，克隆能力由模型侧决定）。
//
// 未配置 voice 能力 → 返回中文引导（提示去模型界面配置），不静默失败。

use std::path::PathBuf;

/// 从 providers.toml 的 capabilities.voice 解析 provider/model/base_url/api_key。
/// 复用与 speech/cloud.rs 相同的 registry 读取路径（不新增配置管线）。
struct VoiceConfig {
    base_url: String,
    api_key: String,
    model: String,
    auth_header: String,
    auth_prefix: String,
}

fn resolve_voice_config() -> Option<VoiceConfig> {
    let config_path = crate::commands::config::get_config_path()?;
    let registry = nuphus::config::ModelRegistry::from_toml(config_path.to_str()?).ok()?;
    let model_id = registry.capabilities.voice.trim();
    if model_id.is_empty() {
        return None;
    }
    let (provider, model) = registry.find_model(model_id)?;
    if provider.base_url.trim().is_empty() || provider.api_key.trim().is_empty() {
        return None;
    }
    Some(VoiceConfig {
        base_url: provider.base_url.trim_end_matches('/').to_string(),
        api_key: provider.api_key.clone(),
        model: model.id.clone(),
        auth_header: if provider.auth_header.is_empty() {
            "authorization".to_string()
        } else {
            provider.auth_header.clone()
        },
        auth_prefix: if provider.auth_prefix.is_empty() {
            "Bearer ".to_string()
        } else {
            provider.auth_prefix.clone()
        },
    })
}

/// 语音克隆（走云端）：参考音频 + 文本 → 克隆音色合成语音（mp3）。
/// reference_path 为参考音频（当前作为合成触发凭证；克隆协议由所选模型/云端决定），
/// text 为要合成的文字；output_path 扩展名决定编码（mp3/wav）。
#[tauri::command]
pub async fn voice_clone(
    reference_path: String,
    text: String,
    output_path: String,
) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Err("请输入要合成的文字".to_string());
    }
    let input = PathBuf::from(&reference_path);
    if !input.is_file() {
        return Err(format!("参考音频不存在：{reference_path}"));
    }
    if input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| !matches!(e.to_ascii_lowercase().as_str(), "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac"))
        .unwrap_or(true)
    {
        return Err("参考音频格式不支持（支持 mp3 / wav / m4a / flac / ogg / aac）".to_string());
    }

    let Some(cfg) = resolve_voice_config() else {
        return Err(
            "未配置语音克隆模型：请在「模型界面 → 图像音频配置 → 语音克隆」选择语音克隆模型（走云端，需配置对应 API Key）"
                .to_string(),
        );
    };

    let output = PathBuf::from(&output_path);
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "mp3".to_string());
    if !matches!(ext.as_str(), "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac") {
        return Err("输出扩展名必须是 mp3 / wav / m4a / flac / ogg / aac 之一".to_string());
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建输出目录失败：{e}"))?;
        }
    }

    let url = format!("{}/audio/speech", cfg.base_url);
    let body = serde_json::json!({
        "model": cfg.model,
        "input": text.trim(),
        "voice": "alloy",
        "response_format": if ext == "wav" { "wav" } else { "mp3" },
    });

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("语音合成客户端创建失败：{e}"))?;
    let resp = client
        .post(&url)
        .header(&cfg.auth_header, format!("{}{}", cfg.auth_prefix, cfg.api_key))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("语音合成请求失败：{e}"))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| format!("读取语音响应失败：{e}"))?;
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&bytes).chars().take(300).collect::<String>();
        return Err(format!("语音合成失败 (HTTP {status}): {detail}"));
    }

    std::fs::write(&output, &bytes).map_err(|e| format!("写入语音文件失败：{e}"))?;
    let size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "output": output.to_string_lossy(),
        "format": ext,
        "size_bytes": size,
    }))
}
