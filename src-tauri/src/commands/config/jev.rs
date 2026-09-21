//! Jev / TypeSafe System One configuration and Workflow enhanced-mode state.
//!
//! Jev is intentionally separate from chat-completions providers. The API key
//! is stored in the canonical providers.toml `[jev]` table and passes through
//! the same local encryption helper as provider keys. Commands only return a
//! safe status projection; the key is never returned to the frontend.

use crate::state::AppState;
use nuphus::desktop_automation::{
    ActionCandidate, AppIdentity, CandidateKind, DecisionInput, DecisionProvider, JevClient,
    Observation, RiskClass, WindowIdentity,
};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct JevConnectionStatus {
    pub status: &'static str,
    pub message: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowEnhancedModeStatus {
    pub enabled: bool,
    pub configured: bool,
    pub status: &'static str,
}

fn load_jev(state: &AppState) -> Result<nuphus::config::JevConfig, String> {
    if !state.llm_config_path.exists() {
        return Ok(nuphus::config::JevConfig::default());
    }
    nuphus::config::ModelRegistry::from_toml(
        state
            .llm_config_path
            .to_str()
            .ok_or_else(|| "配置路径不是有效 UTF-8".to_string())?,
    )
    .map(|registry| registry.jev)
    .map_err(|error| format!("读取 Jev 配置失败: {error}"))
}

fn validate_base_url(raw: &str) -> Result<String, String> {
    let value = raw.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(value).map_err(|_| "Jev API 地址格式无效".to_string())?;
    let local_http =
        url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !local_http {
        return Err("Jev API 地址必须使用 HTTPS（本机 localhost 调试除外）".to_string());
    }
    Ok(value.to_string())
}

fn update_jev_table(
    state: &AppState,
    api_key: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    enabled: Option<bool>,
) -> Result<nuphus::config::JevConfigStatus, String> {
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
        .entry("jev".to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "[jev] 配置不是 table".to_string())?;

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

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let serialized = toml::to_string_pretty(&doc)
        .map_err(|error| format!("序列化 providers.toml 失败: {error}"))?;
    std::fs::write(&state.llm_config_path, serialized)
        .map_err(|error| format!("写入 providers.toml 失败: {error}"))?;
    Ok(load_jev(state)?.status())
}

#[tauri::command]
pub fn get_jev_config(
    state: State<'_, AppState>,
) -> Result<nuphus::config::JevConfigStatus, String> {
    Ok(load_jev(&state)?.status())
}

#[tauri::command]
pub fn save_jev_config(
    state: State<'_, AppState>,
    api_key: Option<String>,
    base_url: String,
    model: String,
) -> Result<nuphus::config::JevConfigStatus, String> {
    let base_url = validate_base_url(&base_url)?;
    let model = model.trim();
    if model.is_empty() || model.len() > 128 {
        return Err("Jev 模型名称不能为空且不得超过 128 个字符".to_string());
    }
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if key.is_none() && load_jev(&state)?.api_key.trim().is_empty() {
        return Err("请先配置 Jev API Key".to_string());
    }
    update_jev_table(&state, key, Some(&base_url), Some(model), Some(true))
}

#[tauri::command]
pub fn clear_jev_api_key(state: State<'_, AppState>) -> Result<(), String> {
    update_jev_table(&state, Some(""), None, None, Some(false))?;
    state.workflow_enhanced_mode.store(false, Ordering::SeqCst);
    if let Ok(mut runtime) = state.runtime.lock() {
        if let Some(agent) = runtime.workflow_agent.as_mut() {
            agent.set_enhanced_mode(false);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn test_jev_connection(
    state: State<'_, AppState>,
) -> Result<JevConnectionStatus, String> {
    let mut config = load_jev(&state)?;
    config.enabled = true;
    let client = JevClient::from_config(config).map_err(|error| error.to_string())?;
    let input = DecisionInput {
        goal: "Verify that the configured Jev endpoint can make a bounded choice.".into(),
        observation: Observation {
            revision: 1,
            fingerprint: "connection-test".into(),
            app: AppIdentity {
                id: "nuphus-settings".into(),
                display_name: "Nuphus Settings".into(),
            },
            window: WindowIdentity {
                id: "jev-settings".into(),
                title: "Jev Settings".into(),
            },
            nodes: vec![],
            captured_at_ms: 0,
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
                public_description: "The request cannot be evaluated".into(),
                local_risk: RiskClass::ReadOnly,
                preconditions: vec![],
                expected_effects: vec![],
            },
        ],
        recent_candidate_ids: vec![],
    };
    let decision = client
        .choose(input)
        .await
        .map_err(|error| error.to_string())?;
    Ok(JevConnectionStatus {
        status: "ready",
        message: "Jev System One 连接正常".into(),
        model: decision.actual_model,
    })
}

#[tauri::command]
pub fn get_workflow_enhanced_mode(
    state: State<'_, AppState>,
) -> Result<WorkflowEnhancedModeStatus, String> {
    let config = load_jev(&state)?;
    let configured = !config.api_key.trim().is_empty();
    let enabled = configured && state.workflow_enhanced_mode.load(Ordering::SeqCst);
    Ok(WorkflowEnhancedModeStatus {
        enabled,
        configured,
        status: if !configured {
            "unconfigured"
        } else if enabled {
            "ready"
        } else {
            "disabled"
        },
    })
}

#[tauri::command]
pub fn set_workflow_enhanced_mode(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<WorkflowEnhancedModeStatus, String> {
    let config = load_jev(&state)?;
    let configured = !config.api_key.trim().is_empty();
    if enabled && !configured {
        return Err("Jev 尚未配置，请先设置 API Key".to_string());
    }
    if state.busy.load(Ordering::SeqCst) {
        return Err("任务执行中，停止后才能切换增强模式".to_string());
    }
    state
        .workflow_enhanced_mode
        .store(enabled, Ordering::SeqCst);
    if let Ok(mut runtime) = state.runtime.lock() {
        if let Some(agent) = runtime.workflow_agent.as_mut() {
            agent.set_enhanced_mode(enabled);
        }
    }
    Ok(WorkflowEnhancedModeStatus {
        enabled,
        configured,
        status: if enabled { "ready" } else { "disabled" },
    })
}

#[cfg(test)]
mod tests {
    use super::validate_base_url;

    #[test]
    fn jev_endpoint_requires_https_except_localhost() {
        assert!(validate_base_url("https://api.typesafe.ai").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://example.com").is_err());
    }
}
