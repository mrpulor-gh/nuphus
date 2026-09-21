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

fn active_workflow_session_id(state: &AppState) -> Option<String> {
    if !is_workflow_mode(state) {
        return None;
    }
    if let Some(id) = state.runtime.lock().ok().and_then(|runtime| {
        runtime
            .workflow_agent
            .as_ref()
            .map(|agent| agent.session().id.clone())
    }) {
        return Some(id);
    }
    workflow_backup_session_id(state)
}

pub(crate) fn workflow_backup_session_id(state: &AppState) -> Option<String> {
    let id = state
        .session
        .lock()
        .ok()
        .and_then(|session| session.session_backup.clone())
        .and_then(|json| serde_json::from_str::<nuphus::session::Session>(&json).ok())
        .map(|session| session.id)?;
    let shelf_mode = state
        .shelf
        .lock()
        .ok()
        .and_then(|shelf| shelf.entries.get(&id).map(|entry| entry.mode.clone()));
    let stored_mode = shelf_mode
        .or_else(|| crate::commands::process::shelf::read_mirror(&id).map(|(mode, _)| mode));
    let known_workflow_preference = state
        .workflow_enhanced_modes
        .lock()
        .ok()
        .is_some_and(|modes| modes.contains_key(&id));
    (stored_mode
        .as_deref()
        .is_some_and(|mode| crate::commands::process::shelf::normalize_mode(mode) == "workflow")
        || known_workflow_preference)
        .then_some(id)
}

fn is_workflow_mode(state: &AppState) -> bool {
    state
        .current_mode
        .read()
        .map(|mode| mode.as_str() == "workflow")
        .unwrap_or(false)
}

fn current_workflow_enhanced_preference(state: &AppState) -> bool {
    if !is_workflow_mode(state) {
        return false;
    }
    if let Some(session_id) = active_workflow_session_id(state) {
        return state
            .workflow_enhanced_modes
            .lock()
            .ok()
            .and_then(|modes| modes.get(&session_id).copied())
            .unwrap_or(false);
    }
    state
        .session
        .lock()
        .ok()
        .and_then(|session| session.pending_workflow_enhanced_mode)
        .unwrap_or(false)
}

fn set_workflow_enhanced_preference(state: &AppState, enabled: bool) -> Result<(), String> {
    if !is_workflow_mode(state) {
        return Err("当前不在 Workflow 模式".to_string());
    }
    if let Some(session_id) = active_workflow_session_id(state) {
        state
            .workflow_enhanced_modes
            .lock()
            .map_err(|error| error.to_string())?
            .insert(session_id, enabled);
        if let Ok(mut session) = state.session.lock() {
            session.pending_workflow_enhanced_mode = None;
        }
    } else {
        state
            .session
            .lock()
            .map_err(|error| error.to_string())?
            .pending_workflow_enhanced_mode = Some(enabled);
    }
    state
        .workflow_enhanced_mode
        .store(enabled, Ordering::SeqCst);
    if let Ok(mut runtime) = state.runtime.lock() {
        if let Some(agent) = runtime.workflow_agent.as_mut() {
            agent.set_enhanced_mode(enabled);
        }
    }
    Ok(())
}

/// Bind the current Workflow enhanced-mode preference to a newly-created agent session.
///
/// A backup continuation receives a new core Session id today, so its old preference is
/// moved to that replacement id. A genuinely new session instead consumes the one-shot
/// welcome-page preference and defaults to disabled when the user made no pending choice.
pub(crate) fn bind_workflow_enhanced_mode_to_session(
    state: &AppState,
    new_session_id: &str,
    restored_session_id: Option<&str>,
) -> bool {
    let pending = state
        .session
        .lock()
        .ok()
        .and_then(|mut session| session.pending_workflow_enhanced_mode.take());
    let enabled = if let Some(restored_session_id) = restored_session_id {
        state
            .workflow_enhanced_modes
            .lock()
            .ok()
            .and_then(|mut modes| modes.remove(restored_session_id))
            .unwrap_or(false)
    } else {
        pending.unwrap_or(false)
    };
    if let Ok(mut modes) = state.workflow_enhanced_modes.lock() {
        modes.insert(new_session_id.to_string(), enabled);
    }
    state
        .workflow_enhanced_mode
        .store(enabled, Ordering::SeqCst);
    enabled
}

pub(crate) fn clear_pending_workflow_enhanced_mode(state: &AppState) {
    if let Ok(mut session) = state.session.lock() {
        session.pending_workflow_enhanced_mode = None;
    }
}

fn clear_all_workflow_enhanced_preferences(state: &AppState) {
    state.workflow_enhanced_mode.store(false, Ordering::SeqCst);
    if let Ok(mut modes) = state.workflow_enhanced_modes.lock() {
        modes.clear();
    }
    clear_pending_workflow_enhanced_mode(state);
    if let Ok(mut runtime) = state.runtime.lock() {
        if let Some(agent) = runtime.workflow_agent.as_mut() {
            agent.set_enhanced_mode(false);
        }
    }
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

fn validate_policy(timeout_ms: u64, max_retries: u32, confidence_floor: f64) -> Result<(), String> {
    if !(100..=120_000).contains(&timeout_ms) {
        return Err("Jev 请求超时必须在 100 到 120000 毫秒之间".to_string());
    }
    if max_retries > 10 {
        return Err("Jev 最大重试次数不得超过 10".to_string());
    }
    if !confidence_floor.is_finite() || !(0.0..=1.0).contains(&confidence_floor) {
        return Err("Jev 低置信回退阈值必须在 0 到 1 之间".to_string());
    }
    Ok(())
}

fn update_jev_table(
    state: &AppState,
    api_key: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    enabled: Option<bool>,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
    fallback_to_primary_model: Option<bool>,
    confidence_floor: Option<f64>,
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
    if let Some(value) = confidence_floor {
        table.insert("confidence_floor".into(), toml::Value::Float(value));
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
    timeout_ms: u64,
    max_retries: u32,
    fallback_to_primary_model: bool,
    confidence_floor: f64,
) -> Result<nuphus::config::JevConfigStatus, String> {
    let base_url = validate_base_url(&base_url)?;
    let model = model.trim();
    if model.is_empty() || model.len() > 128 {
        return Err("Jev 模型名称不能为空且不得超过 128 个字符".to_string());
    }
    validate_policy(timeout_ms, max_retries, confidence_floor)?;
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if key.is_none() && load_jev(&state)?.api_key.trim().is_empty() {
        return Err("请先配置 Jev API Key".to_string());
    }
    update_jev_table(
        &state,
        key,
        Some(&base_url),
        Some(model),
        Some(true),
        Some(timeout_ms),
        Some(max_retries),
        Some(fallback_to_primary_model),
        Some(confidence_floor),
    )
}

#[tauri::command]
pub fn clear_jev_api_key(state: State<'_, AppState>) -> Result<(), String> {
    update_jev_table(
        &state,
        Some(""),
        None,
        None,
        Some(false),
        None,
        None,
        None,
        None,
    )?;
    clear_all_workflow_enhanced_preferences(&state);
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
        recent_actions: vec![],
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
    let enabled = configured && current_workflow_enhanced_preference(&state);
    state
        .workflow_enhanced_mode
        .store(enabled, Ordering::SeqCst);
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
    set_workflow_enhanced_preference(&state, enabled)?;
    Ok(WorkflowEnhancedModeStatus {
        enabled,
        configured,
        status: if enabled { "ready" } else { "disabled" },
    })
}

#[cfg(test)]
mod tests {
    use super::{
        active_workflow_session_id, bind_workflow_enhanced_mode_to_session,
        clear_all_workflow_enhanced_preferences, current_workflow_enhanced_preference,
        set_workflow_enhanced_preference, validate_base_url, validate_policy,
    };
    use crate::commands::process::shelf::ShelfEntry;
    use crate::state::AppState;
    use std::sync::atomic::Ordering;

    #[test]
    fn jev_endpoint_requires_https_except_localhost() {
        assert!(validate_base_url("https://api.typesafe.ai").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://example.com").is_err());
    }

    #[test]
    fn jev_policy_fields_are_bounded_routing_configuration() {
        assert!(validate_policy(10_000, 2, 0.20).is_ok());
        assert!(validate_policy(99, 2, 0.20).is_err());
        assert!(validate_policy(120_001, 2, 0.20).is_err());
        assert!(validate_policy(10_000, 11, 0.20).is_err());
        assert!(validate_policy(10_000, 2, -0.01).is_err());
        assert!(validate_policy(10_000, 2, 1.01).is_err());
        assert!(validate_policy(10_000, 2, f64::NAN).is_err());
    }

    #[test]
    fn active_workflow_session_falls_back_to_switched_backup() {
        let state = AppState::default();
        *state.current_mode.write().unwrap() = "workflow".into();
        let session = nuphus::session::Session::new();
        let expected = session.id.clone();
        state.shelf.lock().unwrap().put(
            ShelfEntry {
                id: expected.clone(),
                mode: "workflow".into(),
                title: String::new(),
                preview: String::new(),
                message_count: 0,
                updated_at: 0,
            },
            session.clone(),
        );
        state.session.lock().unwrap().session_backup =
            Some(serde_json::to_string(&session).unwrap());

        assert_eq!(active_workflow_session_id(&state), Some(expected));
    }

    #[test]
    fn non_workflow_backup_does_not_block_welcome_page_preference() {
        let state = AppState::default();
        *state.current_mode.write().unwrap() = "workflow".into();
        let session = nuphus::session::Session::new();
        let id = session.id.clone();
        state.shelf.lock().unwrap().put(
            ShelfEntry {
                id,
                mode: "leader".into(),
                title: String::new(),
                preview: String::new(),
                message_count: 0,
                updated_at: 0,
            },
            session.clone(),
        );
        state.session.lock().unwrap().session_backup =
            Some(serde_json::to_string(&session).unwrap());

        assert_eq!(active_workflow_session_id(&state), None);
        set_workflow_enhanced_preference(&state, true).unwrap();
        assert_eq!(
            state.session.lock().unwrap().pending_workflow_enhanced_mode,
            Some(true)
        );
    }

    #[test]
    fn pending_enhanced_mode_binds_once_and_later_new_sessions_default_off() {
        let state = AppState::default();
        *state.current_mode.write().unwrap() = "workflow".into();

        set_workflow_enhanced_preference(&state, true).unwrap();
        assert!(current_workflow_enhanced_preference(&state));
        assert_eq!(
            state.session.lock().unwrap().pending_workflow_enhanced_mode,
            Some(true)
        );

        assert!(bind_workflow_enhanced_mode_to_session(
            &state,
            "workflow-new-1",
            None
        ));
        assert_eq!(
            state
                .workflow_enhanced_modes
                .lock()
                .unwrap()
                .get("workflow-new-1"),
            Some(&true)
        );
        assert_eq!(
            state.session.lock().unwrap().pending_workflow_enhanced_mode,
            None
        );

        assert!(!bind_workflow_enhanced_mode_to_session(
            &state,
            "workflow-new-2",
            None
        ));
        assert_eq!(
            state
                .workflow_enhanced_modes
                .lock()
                .unwrap()
                .get("workflow-new-2"),
            Some(&false)
        );
    }

    #[test]
    fn backup_replacement_session_inherits_and_migrates_enhanced_mode() {
        let state = AppState::default();
        *state.current_mode.write().unwrap() = "workflow".into();
        state
            .workflow_enhanced_modes
            .lock()
            .unwrap()
            .insert("workflow-old".into(), true);
        state.session.lock().unwrap().pending_workflow_enhanced_mode = Some(false);

        assert!(bind_workflow_enhanced_mode_to_session(
            &state,
            "workflow-replacement",
            Some("workflow-old")
        ));
        let modes = state.workflow_enhanced_modes.lock().unwrap();
        assert!(!modes.contains_key("workflow-old"));
        assert_eq!(modes.get("workflow-replacement"), Some(&true));
        drop(modes);
        assert!(state.workflow_enhanced_mode.load(Ordering::SeqCst));
        assert_eq!(
            state.session.lock().unwrap().pending_workflow_enhanced_mode,
            None
        );
    }

    #[test]
    fn clearing_jev_preferences_disables_sessions_and_pending_choice() {
        let state = AppState::default();
        state
            .workflow_enhanced_modes
            .lock()
            .unwrap()
            .insert("workflow-a".into(), true);
        state.session.lock().unwrap().pending_workflow_enhanced_mode = Some(true);
        state.workflow_enhanced_mode.store(true, Ordering::SeqCst);

        clear_all_workflow_enhanced_preferences(&state);

        assert!(state.workflow_enhanced_modes.lock().unwrap().is_empty());
        assert!(!state.workflow_enhanced_mode.load(Ordering::SeqCst));
        assert_eq!(
            state.session.lock().unwrap().pending_workflow_enhanced_mode,
            None
        );
    }
}
