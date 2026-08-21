//! Cloud STT backend — OpenAI-compatible batch transcription.
//!
//! Routing (full contract in commands.rs header): when `capabilities.stt`
//! resolves to a concrete provider+model in the model registry, stt sessions
//! transcribe via `POST {base_url}/audio/transcriptions` instead of the local
//! sherpa-onnx pipeline. The API is batch (no streaming): mic audio buffers
//! in memory (same MicCapture/Resampler, same 120s cap — doubles as cost
//! control for a paid API), one wav upload on stop, recognized text emitted
//! as a single stt:final. No VAD segmenting, no partials — the event
//! contract already allows both, the frontend needs zero changes.

use serde::Deserialize;
use std::time::Duration;

/// Transcription endpoint appended to the provider base_url (OpenAI-compatible).
const TRANSCRIPTIONS_PATH: &str = "/audio/transcriptions";

/// Resolved cloud endpoint + credentials. Auth fields are provider-driven,
/// mirroring how the chat_completions transport consumes ProviderConfig.
pub struct CloudSttConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub auth_header: String,
    pub auth_prefix: String,
}

/// Resolve `capabilities.stt` → provider/model through the existing registry
/// bridge (`get_config_path` → `ModelRegistry::from_toml`, the same path
/// commands::process and commands::config use — no new config pipeline).
///
/// Returns None when the capability is unset OR does not resolve to a
/// provider with a base_url; the caller then routes to the local engine.
pub fn resolve_cloud_config() -> Option<CloudSttConfig> {
    let path = crate::commands::config::get_config_path()?;
    let registry = nuphus::config::ModelRegistry::from_toml(path.to_str()?).ok()?;
    let model_id = registry.capabilities.stt.trim();
    if model_id.is_empty() {
        return None;
    }
    let (provider, model) = registry.find_model(model_id)?;
    if provider.base_url.trim().is_empty() {
        return None;
    }
    Some(CloudSttConfig {
        base_url: provider.base_url.trim_end_matches('/').to_string(),
        api_key: provider.api_key.clone(),
        model: model.id.clone(),
        // ProviderConfig leaves these empty when the TOML omits them; the
        // OpenAI-compatible default mirrors transports/chat_completions.
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

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

/// Upload one 16kHz mono wav and return the recognized text.
///
/// Blocking by design: called from the stt-session worker thread, never the
/// UI thread. connect_timeout bounds the stall case; the total timeout
/// matches the 120s session cap (payload ≤3.8MB + server decode) so a hung
/// request can never wedge the session slot in Decoding (download.rs's
/// no-total-timeout tradeoff is wrong here — a stuck download retries, a
/// stuck STT session blocks the next one).
pub fn transcribe(config: &CloudSttConfig, wav_bytes: Vec<u8>) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("cloud stt client build failed: {e}"))?;

    let part = reqwest::blocking::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("cloud stt multipart failed: {e}"))?;
    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", config.model.clone());

    let url = format!("{}{}", config.base_url, TRANSCRIPTIONS_PATH);
    let auth_value = format!("{}{}", config.auth_prefix, config.api_key);
    let resp = client
        .post(&url)
        .header(&config.auth_header, &auth_value)
        .multipart(form)
        .send()
        .map_err(|e| format!("云端识别请求失败: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .map_err(|e| format!("云端识别响应读取失败: {e}"))?;
    if !status.is_success() {
        // OpenAI-compatible error body {"error":{"message":"..."}} — surface
        // the server message when present, else the raw body (truncated).
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| body.chars().take(200).collect());
        return Err(format!("云端识别失败 (HTTP {status}): {detail}"));
    }
    let parsed: TranscriptionResponse =
        serde_json::from_str(&body).map_err(|e| format!("云端识别响应解析失败: {e}"))?;
    Ok(parsed.text)
}
