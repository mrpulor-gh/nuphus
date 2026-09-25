//! PendingApprovalStore — pending approval item storage
//!
//! Session-scoped HashMap<String, PendingApproval>, key = action_id (UUID).
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
    cleanup_expired_with_ttl(map, APPROVAL_TTL);
}

/// Clean up entries older than `ttl`.
///
/// TTL is injectable so tests never need an `Instant` in the past: `Instant - Duration`
/// panics (`overflow when subtracting duration from instant`, std
/// `impl Sub<Duration> for Instant`) as soon as the result would precede that instance's
/// zero point, and the zero point is platform/runtime-dependent — on a fresh Windows CI
/// runner it sat less than `APPROVAL_TTL` away from `now`, so `Instant::now() - APPROVAL_TTL`
/// overflowed there while passing on dev machines. Never subtract time: inject the TTL, and
/// let `saturating_duration_since` absorb any (impossible in practice) future timestamp.
fn cleanup_expired_with_ttl(
    map: &mut std::collections::HashMap<String, (PendingApproval, Instant)>,
    ttl: std::time::Duration,
) {
    let now = Instant::now();
    map.retain(|_, &mut (_, timestamp)| now.saturating_duration_since(timestamp) < ttl);
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
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.pending_approvals);
    state
        .security
        .pending_approvals
        .get(action_id)
        .map(|(p, _)| p.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_does_not_return_expired_approval() {
        let signals = crate::state::new_shared_signals();
        let id = add(&signals, "tenet", "title", "content", serde_json::json!({}));
        // TTL 注入为 0：任何已存在的条目都视为过期。
        // 不再用 `Instant::now() - APPROVAL_TTL` 造过期时间戳：该减法一旦越过本实例零点就
        // panic（CI 实测 `overflow when subtracting duration from instant`，std time.rs
        // `impl Sub<Duration> for Instant` 的 expect）。零点与平台/运行时相关——本机实测零点
        // 在 146 年前（减 600s 安全），而 CI 的 Windows runner 零点距 now 不足 600s，故必炸。
        {
            let mut state = crate::state::SignalState::write(&signals);
            cleanup_expired_with_ttl(
                &mut state.security.pending_approvals,
                std::time::Duration::ZERO,
            );
        }
        assert!(get(&signals, &id).is_none());
        assert!(remove(&signals, &id).is_none());
    }

    #[test]
    fn remove_is_single_use_and_scoped_to_signals() {
        let signals = crate::state::new_shared_signals();
        let other = crate::state::new_shared_signals();
        let id = add(&signals, "tenet", "title", "content", serde_json::json!({}));
        assert!(get(&other, &id).is_none());
        assert!(get(&signals, &id).is_some());
        assert!(remove(&signals, &id).is_some());
        assert!(remove(&signals, &id).is_none());
    }
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
