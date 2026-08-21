use serde::{Deserialize, Serialize};
use std::time::Instant;

const INPUT_TTL: std::time::Duration = std::time::Duration::from_secs(600);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInput {
    pub action_id: String,
    pub title: String,
    pub prompt: String,
    pub sensitive: bool,
    pub input_type: String,
    pub created_at: String,
    // ── icon_confirm 专用字段 ──
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_shortcut: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_note: Option<String>,
}

fn cleanup_expired(map: &mut std::collections::HashMap<String, crate::state::StoredInput>) {
    let now = Instant::now();
    map.retain(|_, s| now.duration_since(s.timestamp) < INPUT_TTL);
}

pub fn add(
    signals: &crate::state::SharedSignals,
    title: &str,
    prompt: &str,
    sensitive: bool,
    input_type: &str,
    icon_path: Option<&str>,
    default_name: Option<&str>,
    default_shortcut: Option<&str>,
    rel_x: Option<i32>,
    rel_y: Option<i32>,
    default_note: Option<&str>,
) -> String {
    let action_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let input = PendingInput {
        action_id: action_id.clone(),
        title: title.to_string(),
        prompt: prompt.to_string(),
        sensitive,
        input_type: input_type.to_string(),
        created_at: now,
        icon_path: icon_path.map(|s| s.to_string()),
        default_name: default_name.map(|s| s.to_string()),
        default_shortcut: default_shortcut.map(|s| s.to_string()),
        rel_x,
        rel_y,
        default_note: default_note.map(|s| s.to_string()),
    };
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_inputs);
    state.security.pending_inputs.insert(
        action_id.clone(),
        crate::state::StoredInput {
            input,
            response: None,
            timestamp: Instant::now(),
        },
    );
    action_id
}

pub fn get(signals: &crate::state::SharedSignals, action_id: &str) -> Option<PendingInput> {
    let state = crate::state::SignalState::read(signals);
    state
        .security
        .pending_inputs
        .get(action_id)
        .map(|s| s.input.clone())
}

pub fn submit(
    signals: &crate::state::SharedSignals,
    action_id: &str,
    value: String,
) -> Option<PendingInput> {
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_inputs);
    if let Some(stored) = state.security.pending_inputs.get_mut(action_id) {
        stored.response = Some(value);
        return Some(stored.input.clone());
    }
    None
}

pub fn poll_response(signals: &crate::state::SharedSignals, action_id: &str) -> Option<String> {
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_inputs);
    if let Some(stored) = state.security.pending_inputs.get(action_id) {
        if stored.response.is_some() {
            return state
                .security
                .pending_inputs
                .remove(action_id)
                .and_then(|s| s.response);
        }
    }
    None
}

pub fn remove(signals: &crate::state::SharedSignals, action_id: &str) -> Option<PendingInput> {
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_inputs);
    state
        .security
        .pending_inputs
        .remove(action_id)
        .map(|s| s.input)
}

/// Cancel a pending input request — sets a cancelled flag that poll will detect.
/// Unlike remove(), this ensures the waiting side wakes immediately instead of timing out.
pub fn cancel(signals: &crate::state::SharedSignals, action_id: &str) {
    let mut state = crate::state::SignalState::write(signals);
    if let Some(entry) = state.security.pending_inputs.get_mut(action_id) {
        entry.response = Some("__CANCELLED__".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_poll() {
        let signals = crate::state::new_shared_signals();
        let id = add(
            &signals,
            "API Key",
            "请输入你的 API Key",
            true,
            "text",
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let input = get(&signals, &id).unwrap();
        assert_eq!(input.title, "API Key");
        assert!(input.sensitive);

        let result = poll_response(&signals, &id);
        assert!(result.is_none());

        submit(&signals, &id, "sk-test-123".to_string());
        let result = poll_response(&signals, &id).unwrap();
        assert_eq!(result, "sk-test-123");

        let result = poll_response(&signals, &id);
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_expired() {
        let signals = crate::state::new_shared_signals();
        let id = add(
            &signals,
            "test",
            "test prompt",
            false,
            "text",
            None,
            None,
            None,
            None,
            None,
            None,
        );
        remove(&signals, &id);
        let result = poll_response(&signals, &id);
        assert!(result.is_none());
    }
}
