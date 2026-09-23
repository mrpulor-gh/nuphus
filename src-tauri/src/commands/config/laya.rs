//! Tauri commands for the self-hosted Laya decision backend.
//!
//! Laya is an alternative to the hosted Jev endpoint, not a replacement for it:
//! the two are separate models that happen to share the `POST /v1/systemone`
//! wire protocol. Each keeps its own table in `providers.toml`, and
//! `ModelRegistry::decision_backend` decides which one a run uses.
//!
//! Mirrors `config/jev.rs` so the two settings surfaces behave identically.
//! The one deliberate difference: a self-hosted Laya needs no API key, since
//! `laya-serve` demands a bearer token only when `LAYA_API_KEY` is set on the
//! server. Requiring one here would block the common local setup.

use crate::state::AppState;
use nuphus::config::{LayaConfigStatus, ModelRegistry};
use nuphus::desktop_automation::{
    ActionCandidate, AppIdentity, CandidateKind, DecisionInput, DecisionProvider, LayaClient,
    LayaConfig, Observation, RiskClass, WindowIdentity,
};
use serde::Serialize;
use tauri::State;

/// Result of a Laya connectivity probe. Reports the routed checkpoint when the
/// service answers, so the user can see which one their deployment selected.
#[derive(Debug, Clone, Serialize)]
pub struct LayaConnectionStatus {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn load_laya(state: &AppState) -> Result<nuphus::config::LayaConfig, String> {
    if !state.llm_config_path.exists() {
        return Ok(nuphus::config::LayaConfig::default());
    }
    ModelRegistry::from_toml(
        state
            .llm_config_path
            .to_str()
            .ok_or_else(|| "配置路径不是有效 UTF-8".to_string())?,
    )
    .map(|registry| registry.laya)
    .map_err(|error| format!("读取 Laya 配置失败: {error}"))
}

fn validate_base_url(raw: &str) -> Result<String, String> {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() {
        return Err("请输入 Laya 服务地址".to_string());
    }
    let url = reqwest::Url::parse(value).map_err(|_| "Laya 服务地址格式无效".to_string())?;
    let local_http =
        url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !local_http {
        return Err(
            "Laya 服务地址必须使用 HTTPS；仅本机 localhost 允许 HTTP（自托管默认场景）".to_string(),
        );
    }
    Ok(value.to_string())
}

fn validate_policy(timeout_ms: u64, max_retries: u32) -> Result<(), String> {
    if !(100..=120_000).contains(&timeout_ms) {
        return Err("Laya 请求超时必须在 100 到 120000 毫秒之间".to_string());
    }
    if max_retries > 10 {
        return Err("Laya 最大重试次数不得超过 10".to_string());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update_laya_table(
    state: &AppState,
    api_key: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    enabled: Option<bool>,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
    fallback_to_primary_model: Option<bool>,
) -> Result<LayaConfigStatus, String> {
    if let Some(parent) = state.llm_config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建配置目录失败: {error}"))?;
    }
    let content = std::fs::read_to_string(&state.llm_config_path).unwrap_or_default();
    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::value::Table::new())
    } else {
        content
            .parse()
            .map_err(|error| format!("解析 providers.toml 失败: {error}"))?
    };
    let root = doc
        .as_table_mut()
        .ok_or_else(|| "providers.toml 根节点必须是 table".to_string())?;
    let table = root
        .entry("laya".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "[laya] 配置不是 table".to_string())?;

    if let Some(key) = api_key {
        table.insert("api_key".into(), toml::Value::String(key.to_string()));
    }
    if let Some(value) = base_url {
        table.insert("base_url".into(), toml::Value::String(value.to_string()));
    }
    if let Some(value) = model {
        table.insert("model".into(), toml::Value::String(value.to_string()));
    }
    if let Some(value) = enabled {
        table.insert("enabled".into(), toml::Value::Boolean(value));
    }
    if let Some(value) = timeout_ms {
        table.insert("timeout_ms".into(), toml::Value::Integer(value as i64));
    }
    if let Some(value) = max_retries {
        table.insert("max_retries".into(), toml::Value::Integer(value as i64));
    }
    if let Some(value) = fallback_to_primary_model {
        table.insert(
            "fallback_to_primary_model".into(),
            toml::Value::Boolean(value),
        );
    }

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let serialized = toml::to_string_pretty(&doc)
        .map_err(|error| format!("序列化 providers.toml 失败: {error}"))?;
    std::fs::write(&state.llm_config_path, serialized)
        .map_err(|error| format!("写入 providers.toml 失败: {error}"))?;
    Ok(load_laya(state)?.status())
}

#[tauri::command]
pub fn get_laya_config(state: State<'_, AppState>) -> Result<LayaConfigStatus, String> {
    Ok(load_laya(&state)?.status())
}

#[tauri::command]
pub fn save_laya_config(
    state: State<'_, AppState>,
    base_url: String,
    model: Option<String>,
    enabled: bool,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
    fallback_to_primary_model: Option<bool>,
    api_key: Option<String>,
) -> Result<LayaConfigStatus, String> {
    let base_url = validate_base_url(&base_url)?;
    let timeout = timeout_ms.unwrap_or(10_000);
    let retries = max_retries.unwrap_or(2);
    validate_policy(timeout, retries)?;
    // Unlike Jev, no key is required: a self-hosted Laya normally runs open on
    // loopback. An empty submitted key keeps whatever is already stored.
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    update_laya_table(
        &state,
        key,
        Some(&base_url),
        model.as_deref(),
        Some(enabled),
        Some(timeout),
        Some(retries),
        fallback_to_primary_model,
    )
}

#[tauri::command]
pub fn clear_laya_api_key(state: State<'_, AppState>) -> Result<(), String> {
    update_laya_table(&state, Some(""), None, None, None, None, None, None)?;
    Ok(())
}

/// Probe the configured service with one bounded Choice.
///
/// The request mirrors what the desktop loop sends, so a success here means the
/// deployment can actually answer the questions this app asks. It cannot prove
/// anything about a real desktop session — only that the service is reachable
/// and its payload parses.
#[tauri::command]
pub async fn test_laya_connection(
    state: State<'_, AppState>,
) -> Result<LayaConnectionStatus, String> {
    let config = load_laya(&state)?;
    let client = LayaClient::from_config(LayaConfig {
        base_url: config.base_url,
        model: config.model,
        timeout_ms: config.timeout_ms,
        max_retries: config.max_retries,
        api_key: config.api_key,
    })
    .map_err(|error| error.to_string())?;

    let input = DecisionInput {
        goal: "Verify that the configured Laya endpoint can make a bounded choice.".into(),
        observation: Observation {
            revision: 1,
            fingerprint: "connection-test".into(),
            app: AppIdentity {
                id: "nuphus-settings".into(),
                display_name: "Nuphus Settings".into(),
            },
            window: WindowIdentity {
                id: "laya-settings".into(),
                title: "Laya Settings".into(),
            },
            nodes: vec![],
            captured_at_ms: 0,
            truncated: false,
        },
        candidates: vec![
            ActionCandidate {
                id: "connection_ready".into(),
                observation_revision: 1,
                target: None,
                kind: CandidateKind::Done,
                public_description: "The connection test request is valid and can finish".into(),
                local_risk: RiskClass::ReadOnly,
                preconditions: vec![],
                expected_effects: vec![],
            },
            ActionCandidate {
                id: "cannot_proceed".into(),
                observation_revision: 1,
                target: None,
                kind: CandidateKind::CannotProceed,
                public_description: "The request is malformed and should be re-examined".into(),
                local_risk: RiskClass::ReadOnly,
                preconditions: vec![],
                expected_effects: vec![],
            },
        ],
        recent_actions: vec![],
    };

    match client.choose(input).await {
        Ok(decision) => Ok(LayaConnectionStatus {
            status: "ok".into(),
            model: decision.actual_model,
            message: None,
        }),
        Err(error) => Ok(LayaConnectionStatus {
            status: "error".into(),
            model: None,
            // Surface the class of failure without leaking the base URL back.
            message: Some(error.to_string()),
        }),
    }
}
