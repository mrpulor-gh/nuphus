//! SecurityGuard — dangerous operation detection and confirmation flow
//!
//! Performs security checks at the tool execution layer, after parameter parsing and before actual execution.
//! When a dangerous operation is detected, notifies the frontend via NuphusEvent::SecurityCheck,
//! displayed in the third view (security dialog) awaiting user confirmation.
//!
//! Design principles:
//! - Normalize input before all pattern matching (collapse spaces, lowercase) to prevent bypass
//! - All security rules require user confirmation (RequireConfirmation), not direct denial
//!   (user always sees prompt, can approve or reject)
//! - Only technical restrictions (e.g. overly long commands) return Deny
//!
//! v4 improvements: rules loaded from embedded YAML + external files, users can customize rules.
//!
//! v4.1 improvements:
//! - Session-level authorization (approve_session): same tool no longer prompts in this session
//! - Only high-risk operations prompt under DangerFullAccess mode

pub mod approval;
pub mod injection;
pub mod user_input;

use crate::api::types::RiskLevel;

use std::time::Instant;

/// Security confirmation result TTL (5 minutes)
const SECURITY_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Clean up expired entries
fn cleanup_expired(map: &mut std::collections::HashMap<String, (bool, Instant)>) {
    let now = Instant::now();
    map.retain(|_, &mut (_, timestamp)| now.duration_since(timestamp) < SECURITY_TTL);
}

/// Set security confirmation result (called by Tauri approve_security/reject_security)
pub fn set_security_result(signals: &crate::state::SharedSignals, action_id: &str, approved: bool) {
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.security_results);
    state
        .security
        .security_results
        .insert(action_id.to_string(), (approved, Instant::now()));
}

/// Query security confirmation result (called by ExecuteAgent polling)
pub fn check_security_result(
    signals: &crate::state::SharedSignals,
    action_id: &str,
) -> Option<bool> {
    let mut state = crate::state::SignalState::write(signals);
    cleanup_expired(&mut state.security.security_results);
    state
        .security
        .security_results
        .remove(action_id)
        .map(|(approved, _)| approved)
}

/// Add tool to this session's authorization set (subsequent calls in this session won't prompt)
pub fn approve_session_tool(signals: &crate::state::SharedSignals, tool: &str) {
    crate::state::SignalState::write(signals)
        .security
        .session_approved_tools
        .insert(tool.to_string());
}

/// Check if tool is already authorized in the session
pub fn is_session_approved(signals: &crate::state::SharedSignals, tool: &str) -> bool {
    crate::state::SignalState::read(signals)
        .security
        .session_approved_tools
        .contains(tool)
}

/// Determine if risk level requires popup even under DangerFullAccess
pub fn is_critical_risk(risk: &RiskLevel) -> bool {
    *risk == RiskLevel::High || *risk == RiskLevel::Critical
}

/// Security check result
#[derive(Debug, Clone)]
pub enum SecurityDecision {
    /// Allow execution
    Allow,
    /// Requires user confirmation
    RequireConfirmation {
        action_id: String,
        risk: RiskLevel,
        reason: String,
    },
    /// Direct denial
    Deny { reason: String },
}

/// Normalize command string: lowercase + collapse consecutive spaces + trim
fn normalize(input: &str) -> String {
    let s = input.to_lowercase();
    // Collapse consecutive spaces, tabs into single space
    let mut result = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch == ' ' || ch == '\t' {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(ch);
            prev_space = false;
        }
    }
    result.trim().to_string()
}

/// Check if normalized shell command contains dangerous patterns (hardcoded replacing YAML rule engine)
fn check_shell_patterns(normalized: &str) -> Option<SecurityDecision> {
    // ── Direct Denial (Deny): extreme dangerous operations like destroying system drive ──
    let deny_patterns: &[&str] = &[
        "format a:",
        "format b:",
        "format c:",
        "format d:",
        "format e:",
        "format f:",
        "format g:",
        "format h:",
        "diskpart",
        "del /f /s a:\\",
        "del /f /s b:\\",
        "del /f /s c:\\",
        "del /f /s d:\\",
        "del /f /s e:\\",
        "del /f /s f:\\",
        "del /f /s g:\\",
        "del /f /s h:\\",
        "rmdir /s /q a:\\",
        "rmdir /s /q b:\\",
        "rmdir /s /q c:\\",
        "rmdir /s /q d:\\",
        "rmdir /s /q e:\\",
        "rmdir /s /q f:\\",
        "rmdir /s /q g:\\",
        "rmdir /s /q h:\\",
        "rd /s /q a:\\",
        "rd /s /q b:\\",
        "rd /s /q c:\\",
        "rd /s /q d:\\",
        "rd /s /q e:\\",
        "rd /s /q f:\\",
        "rd /s /q g:\\",
        "rd /s /q h:\\",
        "rm -rf /",
        "rm -rf --no-preserve-root",
        // 禁止 kill nuphus 自身进程
        "taskkill nuphus",
        "taskkill /f nuphus",
        "taskkill /im nuphus",
        "taskkill /f /im nuphus",
        "stop-process nuphus",
        "stop-process -name nuphus",
        "pkill nuphus",
        "killall nuphus",
    ];
    for &p in deny_patterns {
        if normalized.contains(p) {
            let reason = if p.contains("nuphus") {
                "禁止kill自身进程，请遵守提示词规则。"
            } else {
                "command will destroy system drive, blocked for system safety"
            };
            return Some(SecurityDecision::Deny {
                reason: reason.to_string(),
            });
        }
    }

    // ── Require Confirmation (RequireConfirmation): dangerous operations ──
    let confirm_patterns: &[(&[&str], RiskLevel, &str)] = &[
        (
            &["del /f /s", "rmdir /s /q", "remove-item -recurse -force"],
            RiskLevel::High,
            "recursive deletion of many files may cause data loss",
        ),
        (
            &["reg delete", "reg add"],
            RiskLevel::High,
            "modifying registry may affect system stability",
        ),
        (
            &["shutdown", "restart-computer"],
            RiskLevel::Medium,
            "command involves shutting down or restarting the system",
        ),
        (
            &["taskkill /f /im", "kill -9"],
            RiskLevel::Medium,
            "forcefully terminating processes may cause data loss",
        ),
        (
            &["net user", "net localgroup"],
            RiskLevel::High,
            "modifying user accounts requires caution",
        ),
        (
            &["icacls", "takeown"],
            RiskLevel::High,
            "modifying file permissions may cause system access issues",
        ),
        (
            &["wmic", "sc stop", "sc delete"],
            RiskLevel::Medium,
            "WMI/service operations may affect system operation",
        ),
    ];

    for (patterns, risk, reason) in confirm_patterns {
        for &p in *patterns {
            if normalized.contains(p) {
                return Some(SecurityDecision::RequireConfirmation {
                    action_id: uuid::Uuid::new_v4().to_string(),
                    risk: risk.clone(),
                    reason: reason.to_string(),
                });
            }
        }
    }

    // ── System directory related (triggers confirmation) ──
    let system_dir_patterns: &[&str] = &[
        "c:\\windows",
        "c:\\program files",
        "$env:windir",
        "$env:systemroot",
        "/etc/",
        "/usr/",
        "/bin/",
        "/sbin/",
    ];
    for &p in system_dir_patterns {
        if normalized.contains(p) {
            return Some(SecurityDecision::RequireConfirmation {
                action_id: uuid::Uuid::new_v4().to_string(),
                risk: RiskLevel::Medium,
                reason: "command touches system directory, please confirm scope of operation"
                    .to_string(),
            });
        }
    }

    None
}

/// Check if write path is a system critical directory (hardcoded replacing YAML rule engine)
fn check_write_patterns(normalized: &str) -> Option<SecurityDecision> {
    let system_paths: &[&str] = &[
        "c:\\windows",
        "c:\\program files",
        "c:\\program files (x86)",
        "$env:windir",
        "$env:systemroot",
        "/etc/",
        "/usr/",
        "/bin/",
        "/sbin/",
        "/boot/",
        "\\system32\\",
        "\\syswow64\\",
    ];
    for &p in system_paths {
        if normalized.contains(p) {
            return Some(SecurityDecision::RequireConfirmation {
                action_id: uuid::Uuid::new_v4().to_string(),
                risk: RiskLevel::High,
                reason: "writing to system path may corrupt system files".to_string(),
            });
        }
    }
    None
}

/// Security checker
pub struct SecurityGuard;

impl SecurityGuard {
    /// Check if tool call is safe
    ///
    /// Returns SecurityDecision:
    /// - Allow: safe, can execute directly
    /// - RequireConfirmation: needs frontend confirmation
    /// - Deny: direct denial
    pub fn check(tool_name: &str, params: &serde_json::Value) -> SecurityDecision {
        // Desktop tools uniformly require popup confirmation
        if crate::tools::ToolRegistry::is_desktop_tool(tool_name) {
            return Self::check_desktop(tool_name, params);
        }
        match tool_name {
            "system_shell" => Self::check_shell(params),
            "Write" | "Edit" | "Rename" | "Delete" => Self::check_write(params),
            "RemoveDir" => Self::check_remove_dir(params),
            _ => SecurityDecision::Allow,
        }
    }

    fn check_shell(params: &serde_json::Value) -> SecurityDecision {
        let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");

        // ── Command length limit: prevent buffer abuse ──
        const MAX_COMMAND_LEN: usize = 8192;
        if command.len() > MAX_COMMAND_LEN {
            return SecurityDecision::Deny {
                reason: format!(
                    "command too long ({} > {} characters), blocked",
                    command.len(),
                    MAX_COMMAND_LEN
                ),
            };
        }

        // ── Self-process kill check (PID-based) ──
        let self_pid = std::process::id();
        let self_pid_str = self_pid.to_string();
        if command.contains(&self_pid_str) {
            let lower = command.to_lowercase();
            let kill_keywords = [
                "taskkill",
                "stop-process",
                "kill ",
                "pkill ",
                "killall",
                "tskill",
                "wmic process",
                "taskkill.exe",
            ];
            for kw in &kill_keywords {
                if lower.contains(kw) {
                    return SecurityDecision::Deny {
                        reason: "禁止kill自身进程，请遵守提示词规则。".to_string(),
                    };
                }
            }
        }

        // Normalize: lowercase + collapse spaces + trim (prevent bypass like format    c: )
        let normalized = normalize(command);

        // ── Hardcoded pattern matching (replacing old YAML rule engine) ──
        if let Some(decision) = check_shell_patterns(&normalized) {
            return decision;
        }

        SecurityDecision::Allow
    }

    fn check_write(params: &serde_json::Value) -> SecurityDecision {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");

        let normalized = normalize(path);

        // ── Hardcoded pattern matching (replacing old YAML rule engine) ──
        if let Some(decision) = check_write_patterns(&normalized) {
            return decision;
        }

        // Non-system path, allow directly (upper PermissionPolicy already checks file_access switch)
        SecurityDecision::Allow
    }

    fn check_remove_dir(params: &serde_json::Value) -> SecurityDecision {
        let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let recursive = params
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !recursive {
            return SecurityDecision::Allow;
        }

        // Recursive directory deletion involves batch deleting all child files, needs confirmation
        SecurityDecision::RequireConfirmation {
            action_id: uuid::Uuid::new_v4().to_string(),
            risk: RiskLevel::Medium,
            reason: format!("will recursively delete directory: {} (including all sub-files and sub-directories)", path),
        }
    }

    fn check_desktop(tool_name: &str, _params: &serde_json::Value) -> SecurityDecision {
        SecurityDecision::RequireConfirmation {
            action_id: uuid::Uuid::new_v4().to_string(),
            risk: RiskLevel::Medium,
            reason: format!(
                "desktop operation '{}' is prohibited unless explicitly requested by user, needs confirmation.",
                tool_name,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_safe_command() {
        let params = serde_json::json!({"command": "dir C:\\Users"});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Allow => {}
            other => panic!("expected Allow, got {:?}", other),
        }
    }

    #[test]
    fn test_deny_format() {
        let params = serde_json::json!({"command": "format c:"});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Deny { .. } => {}
            other => panic!("expected Deny, got {:?}", other),
        }
    }

    #[test]
    fn test_confirm_dangerous() {
        let params = serde_json::json!({"command": "shutdown /s /t 0"});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::RequireConfirmation { .. } => {}
            other => panic!("expected RequireConfirmation, got {:?}", other),
        }
    }

    #[test]
    fn test_allow_safe_write() {
        let test_path = format!(
            "{}/Desktop/nonexistent.md",
            std::env::var("USERPROFILE").unwrap_or_default()
        );
        let params = serde_json::json!({"path": test_path});
        match SecurityGuard::check("Write", &params) {
            SecurityDecision::Allow => {}
            other => panic!("expected Allow, got {:?}", other),
        }
    }

    #[test]
    fn test_allow_overwrite_existing_file() {
        // Normal user file overwrite no longer pops up (controlled by upper PermissionPolicy)
        let tmp = std::env::temp_dir().join("nuphus_test_overwrite.txt");
        std::fs::write(&tmp, "test content").unwrap();
        let params = serde_json::json!({"path": tmp.to_string_lossy().to_string()});
        match SecurityGuard::check("Write", &params) {
            SecurityDecision::Allow => {}
            other => panic!("expected Allow for overwrite, got {:?}", other),
        }
        std::fs::remove_file(&tmp).unwrap();
    }

    #[test]
    fn test_allow_write_new_file() {
        let tmp = std::env::temp_dir().join("nuphus_test_new_file_that_does_not_exist.txt");
        // Ensure it doesn't exist
        let _ = std::fs::remove_file(&tmp);
        let params = serde_json::json!({"path": tmp.to_string_lossy().to_string()});
        match SecurityGuard::check("Write", &params) {
            SecurityDecision::Allow => {}
            other => panic!("expected Allow for new file, got {:?}", other),
        }
    }

    #[test]
    fn test_confirm_system_write() {
        let params = serde_json::json!({"path": "C:\\Windows\\System32\\evil.dll"});
        match SecurityGuard::check("Write", &params) {
            SecurityDecision::RequireConfirmation { .. } => {}
            other => panic!("expected RequireConfirmation, got {:?}", other),
        }
    }

    #[test]
    fn test_deny_format_with_extra_spaces() {
        let params = serde_json::json!({"command": "format    c:"});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Deny { .. } => {}
            other => panic!("expected Deny for bypass attempt, got {:?}", other),
        }
    }

    #[test]
    fn test_deny_format_with_tabs() {
        let params = serde_json::json!({"command": "format\t\tc:"});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Deny { .. } => {}
            other => panic!("expected Deny for tab bypass, got {:?}", other),
        }
    }

    #[test]
    fn test_deny_format_with_leading_trailing_spaces() {
        let params = serde_json::json!({"command": "  format c:  "});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Deny { .. } => {}
            other => panic!("expected Deny for leading/trailing spaces, got {:?}", other),
        }
    }

    #[test]
    fn test_deny_rm_rf() {
        let params = serde_json::json!({"command": "rm -rf /"});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Deny { .. } => {}
            other => panic!("expected Deny for rm -rf /, got {:?}", other),
        }
    }

    #[test]
    fn test_block_oversized_command() {
        let long_cmd = "x".repeat(9000);
        let params = serde_json::json!({"command": long_cmd});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Deny { .. } => {}
            other => panic!("expected Deny for long command, got {:?}", other),
        }
    }

    #[test]
    fn test_allow_maxlen_command() {
        let max_cmd = "dir C:\\Users".to_string() + &"x".repeat(8000);
        let params = serde_json::json!({"command": max_cmd});
        match SecurityGuard::check("system_shell", &params) {
            SecurityDecision::Allow => {}
            other => panic!("expected Allow for near-max command, got {:?}", other),
        }
    }
}
