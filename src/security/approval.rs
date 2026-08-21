//! PendingApprovalStore — pending approval item storage
//!
//! Global static HashMap<String, PendingApproval>, key = action_id (UUID).
//! TTL 10 minutes auto-expiry to prevent accumulation.
//!
//! Design principles:
//! - 存储于 `crate::state::SignalState`（SharedSignals 显式注入）
//! - For tools like tenet_add to implement "write after user approval" flow
//! - Does not expose internal lock directly, accessed via add/get/remove methods

use serde::{Deserialize, Serialize};
use std::time::Instant;

const APPROVAL_TTL: std::time::Duration = std::time::Duration::from_secs(600); // 10 minutes

/// Pending approval item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique identifier (UUID)
    pub action_id: String,
    /// Kind ("tenet" etc., for frontend to display different dialogs)
    pub kind: String,
    /// Title
    pub title: String,
    /// Content
    pub content: String,
    /// Additional metadata (JSON object, e.g. priority)
    pub metadata: serde_json::Value,
    /// Creation time
    pub created_at: String,
}

/// Clean up expired entries
fn cleanup_expired(map: &mut std::collections::HashMap<String, (PendingApproval, Instant)>) {
    let now = Instant::now();
    map.retain(|_, &mut (_, timestamp)| now.duration_since(timestamp) < APPROVAL_TTL);
}

/// Add pending approval item, returns action_id
pub fn add(
    signals: &crate::state::SharedSignals,
    kind: &str,
    title: &str,
    content: &str,
    metadata: serde_json::Value,
) -> String {
    let action_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let pending = PendingApproval {
        action_id: action_id.clone(),
        kind: kind.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        metadata,
        created_at: now,
    };
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_approvals);
    state
        .security
        .pending_approvals
        .insert(action_id.clone(), (pending, Instant::now()));
    action_id
}

/// Get pending approval item (without removing)
pub fn get(signals: &crate::state::SharedSignals, action_id: &str) -> Option<PendingApproval> {
    let state = crate::state::SignalState::read(signals);
    state
        .security
        .pending_approvals
        .get(action_id)
        .map(|(p, _)| p.clone())
}

/// Remove and return pending approval item (called on approve/reject)
pub fn remove(signals: &crate::state::SharedSignals, action_id: &str) -> Option<PendingApproval> {
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_approvals);
    state
        .security
        .pending_approvals
        .remove(action_id)
        .map(|(p, _)| p)
}
