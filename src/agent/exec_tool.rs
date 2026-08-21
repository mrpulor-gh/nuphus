//! exec_tool — Unified tool execution + safety checks
//!
//! Merges duplicated safety check/permission logic between Leader (`execute_tool_with_permission`)
//! and ExecuteAgent (`check_tool_safety`) into this module.
//!
//! Architecture:
//! - `check_permission_with_approval()` — permission check + async user approval
//! - `check_security_guard()` — SecurityGuard standardized check
//! - `wait_for_security_approval_*()` — dialog waiting for user decision
//! - `execute_tool_only()` — pure tool execution (no safety checks)

use crate::{
    agent::events::{EventEmitter, NuphusEvent, RiskLevel},
    permissions::{PermissionPolicy, ToolPermissions},
    security::{SecurityDecision, SecurityGuard},
    tools::ToolRegistry,
    ToolResult,
};
use std::sync::atomic::AtomicBool;

/// Safety check result
pub struct SafetyCheck {
    pub should_block: bool,
    pub block_reason: Option<String>,
    pub warnings: Vec<String>,
    pub breaker_triggered: bool,
}

impl SafetyCheck {
    pub fn allow() -> Self {
        Self {
            should_block: false,
            block_reason: None,
            warnings: vec![],
            breaker_triggered: false,
        }
    }
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            should_block: true,
            block_reason: Some(reason.into()),
            warnings: vec![],
            breaker_triggered: false,
        }
    }
    pub fn breaker(reason: impl Into<String>) -> Self {
        Self {
            should_block: true,
            block_reason: Some(reason.into()),
            warnings: vec![],
            breaker_triggered: true,
        }
    }
}

/// Permission check → emit SecurityCheck event → wait for user approval
pub async fn check_permission_with_approval(
    signals: &crate::state::SharedSignals,
    tool: &str,
    params: &serde_json::Value,
    policy: &PermissionPolicy,
    emitter: Option<&dyn EventEmitter>,
    cancel_flag: &std::sync::atomic::AtomicBool,
) -> Option<String> {
    let (allowed, msg) = policy.authorize_with_message(tool);
    if allowed {
        return None;
    }

    let action_id = uuid::Uuid::new_v4().to_string();
    if let Some(emitter) = emitter {
        emitter.emit(NuphusEvent::SecurityCheck {
            action_id: action_id.clone(),
            tool: tool.to_string(),
            params: serde_json::to_string(params).unwrap_or_default(),
            risk: RiskLevel::Medium,
            reason: format!("工具 '{}' 需要更高权限：{}", tool, msg),
        });
        emitter.emit(NuphusEvent::HudUpdate {
            text: format!("{} — 需要确认", tool),
            phase: "security".into(),
            step_kind: None,
        });
    }

    for _ in 0..600 {
        if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(emitter) = emitter {
                emitter.emit(NuphusEvent::PromptTimeout {
                    action_id: action_id.clone(),
                });
            }
            return Some("用户取消了操作".to_string());
        }
        if let Some(result) = crate::security::check_security_result(signals, &action_id) {
            if result {
                crate::security::approve_session_tool(signals, tool);
                return None;
            }
            return Some(format!("用户拒绝: {}", msg));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    if let Some(emitter) = emitter {
        emitter.emit(NuphusEvent::PromptTimeout {
            action_id: action_id.clone(),
        });
    }
    Some(format!("权限审批超时: {}", msg))
}

/// SecurityGuard check result
pub enum SecurityCheckResult {
    AllowAndContinue,
    Blocked(String),
    RequireConfirm(String, RiskLevel, String),
}

/// Execute SecurityGuard check
pub fn check_security_guard(tool: &str, params: &serde_json::Value) -> SecurityCheckResult {
    match SecurityGuard::check(tool, params) {
        SecurityDecision::Allow => SecurityCheckResult::AllowAndContinue,
        SecurityDecision::Deny { reason } => SecurityCheckResult::Blocked(reason),
        SecurityDecision::RequireConfirmation {
            action_id,
            risk,
            reason,
        } => SecurityCheckResult::RequireConfirm(action_id, risk, reason),
    }
}

/// 向前端广播弹窗超时/关闭事件（幂等，无 emitter 时静默跳过）
fn emit_prompt_timeout(emitter: Option<&dyn EventEmitter>, action_id: &str) {
    if let Some(emitter) = emitter {
        emitter.emit(NuphusEvent::PromptTimeout {
            action_id: action_id.to_string(),
        });
    }
}

/// Wait for user security approval (async version — for Leader)
pub async fn wait_for_security_approval_async(
    signals: &crate::state::SharedSignals,
    action_id: &str,
    cancel_flag: &std::sync::atomic::AtomicBool,
    emitter: Option<&dyn EventEmitter>,
) -> bool {
    for _ in 0..600 {
        if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
            emit_prompt_timeout(emitter, action_id);
            return false;
        }
        if let Some(result) = crate::security::check_security_result(signals, action_id) {
            return result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    emit_prompt_timeout(emitter, action_id);
    false
}

/// Wait for user input response (after request_user_input tool returns action_id)
pub async fn wait_for_user_input_async(
    signals: &crate::state::SharedSignals,
    action_id: &str,
    cancel_flag: &std::sync::atomic::AtomicBool,
    emitter: Option<&dyn EventEmitter>,
) -> Option<String> {
    for _ in 0..600 {
        if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
            emit_prompt_timeout(emitter, action_id);
            return None;
        }
        if let Some(response) = crate::security::user_input::poll_response(signals, action_id) {
            if response == "__CANCELLED__" {
                return None; // 用户取消，立即返回
            }
            return Some(response);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    emit_prompt_timeout(emitter, action_id);
    None
}

/// Extract action_id from tool output (format: "action_id=xxx")
pub fn extract_action_id(output: &str) -> Option<String> {
    let re = regex::Regex::new(r"action_id=([^\s，。、]+)").ok()?;
    re.captures(output)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Build security warning text
pub fn build_security_warning(reason: &str) -> String {
    format!(
        "⚠ 安全拦截：操作被安全规则拒绝（{}）。此路径不可行，请立即改用其他方法。",
        reason
    )
}

// ════════════════════════════════════════════════════════════
// SafetyBreaker — 分级熔断阈值
// ════════════════════════════════════════════════════════════
//
// 连续安全检查拒绝计数达到不同阈值时触发不同响应：
//   L3 WARN    — 注入警告，LLM 应改变策略
//   L5 RESTRICT — 注入强警告 + 限制写操作
//   L7 BREAK   — 硬熔断，终止执行

/// 首次警告阈值：LLM 被连续拒绝 3 次，注入警告提示
pub const BREAKER_WARN: u32 = 3;
/// 限制阈值：连续拒绝 5 次，注入强警告 + 建议只读操作
pub const BREAKER_RESTRICT: u32 = 5;
/// 熔断阈值：连续拒绝 7 次，终止执行
pub const BREAKER_HALT: u32 = 7;

/// 根据计数返回分级响应
pub enum BreakerAction {
    None,
    Warn,
    Restrict,
    Halt,
}

pub fn breaker_check(count: u32) -> BreakerAction {
    if count >= BREAKER_HALT {
        BreakerAction::Halt
    } else if count >= BREAKER_RESTRICT {
        BreakerAction::Restrict
    } else if count >= BREAKER_WARN {
        BreakerAction::Warn
    } else {
        BreakerAction::None
    }
}

/// 分级警告消息：L3 提示改变策略，L5 提示危险并限制
pub fn breaker_message(count: u32) -> Option<String> {
    match breaker_check(count) {
        BreakerAction::Warn => Some(format!(
            "安全检查警告：你已被连续拒绝 {} 次。请立即停止当前策略，用完全不同的方法重新尝试。不要再尝试类似的操作。",
            count
        )),
        BreakerAction::Restrict => Some(format!(
            "🚫 安全检查严重警告：你已被连续拒绝 {} 次。从现在开始只允许读取操作（Read/Glob/Grep等），禁止写入、删除或执行系统命令。如果再被拒绝 2 次，本次执行将被强制终止。",
            count
        )),
        _ => None,
    }
}

/// Unified safety check chain
///
/// Merges ReactAgent::execute_tool_with_permission and SubTaskRunner::check_tool_safety
/// duplicated safety logic. Check chain:
///   1. Session-approved bypass
///   2. PermissionPolicy check (check_permission_with_approval)
///   3. Category permission switch check (ToolPermissions: file_access/web_search/system_automation)
///   4. SecurityGuard check (check_security_guard + wait_for_security_approval)
///
/// # Arguments
/// - `tool_permissions`: Some enables category switch check (SubTaskRunner behavior);
///   None skips this (ReactAgent already covered via PermissionPolicy)
/// - `safety_counter`: auto-incremented on block, caller uses breaker_check() for graded response
/// - `warnings`: appends friendly hints on block
///
/// # Returns
/// - `None` = safety passed
/// - `Some(ToolResult::failure)` = blocked, reason in result.error
pub async fn check_tool_security(
    signals: &crate::state::SharedSignals,
    tool: &str,
    params: &serde_json::Value,
    policy: &PermissionPolicy,
    emitter: Option<&dyn EventEmitter>,
    cancel_flag: &AtomicBool,
    tool_permissions: Option<&ToolPermissions>,
    safety_counter: &mut u32,
    warnings: &mut Vec<String>,
) -> Option<ToolResult> {
    // 1. Session-approved bypass
    if crate::security::is_session_approved(signals, tool) {
        return None;
    }

    // 2. PermissionPolicy check (async polling dialog)
    if let Some(reason) =
        check_permission_with_approval(signals, tool, params, policy, emitter, cancel_flag).await
    {
        warnings.push(build_security_warning(&reason));
        *safety_counter += 1;
        return Some(ToolResult::failure(reason));
    }

    // 3. Category permission switch check (SubTaskRunner compatibility)
    if let Some(tp) = tool_permissions {
        if crate::security::is_session_approved(signals, tool) {
            return None;
        }
        let cat = categorize_tool(tool);
        let disabled = match cat {
            Some(crate::permissions::ToolCategory::FileAccess) => !tp.file_access,
            Some(crate::permissions::ToolCategory::WebSearch) => !tp.web_search,
            Some(crate::permissions::ToolCategory::SystemAutomation) => !tp.system_automation,
            _ => false,
        };
        if disabled {
            let cat_name = match cat {
                Some(crate::permissions::ToolCategory::FileAccess) => "文件读写",
                Some(crate::permissions::ToolCategory::WebSearch) => "网络搜索",
                Some(crate::permissions::ToolCategory::SystemAutomation) => "系统网页自动化",
                _ => "未知",
            };
            let action_id = uuid::Uuid::new_v4().to_string();
            if let Some(emitter) = emitter {
                emitter.emit(NuphusEvent::SecurityCheck {
                    action_id: action_id.clone(),
                    tool: tool.to_string(),
                    params: serde_json::to_string(params).unwrap_or_default(),
                    risk: RiskLevel::Medium,
                    reason: format!(
                        "工具 '{}' 的「{}」权限已关闭。是否临时放行此操作?",
                        tool, cat_name
                    ),
                });
                emitter.emit(NuphusEvent::HudUpdate {
                    text: format!("{} — 权限已关闭", tool),
                    phase: "security".into(),
                    step_kind: None,
                });
            }
            let approved =
                wait_for_security_approval_async(signals, &action_id, cancel_flag, emitter).await;
            if !approved {
                *safety_counter += 1;
                return Some(ToolResult::failure(format!(
                    "权限不足: 工具 '{}' 的「{}」权限已关闭",
                    tool, cat_name,
                )));
            }
            crate::security::approve_session_tool(signals, tool);
        }
    }

    // 4. SecurityGuard check
    match check_security_guard(tool, params) {
        SecurityCheckResult::AllowAndContinue => {}
        SecurityCheckResult::Blocked(reason) => {
            warnings.push(build_security_warning(&reason));
            *safety_counter += 1;
            return Some(ToolResult::failure(reason));
        }
        SecurityCheckResult::RequireConfirm(_action_id, risk, reason) => {
            if crate::security::is_session_approved(signals, tool) {
                return None;
            }
            // Auto-allow non-critical operations when settings page permission is enabled
            if let Some(tp) = tool_permissions {
                let cat = categorize_tool(tool);
                let permission_granted = cat.is_some_and(|c| match c {
                    crate::permissions::ToolCategory::FileAccess => tp.file_access,
                    crate::permissions::ToolCategory::SystemAutomation => tp.system_automation,
                    crate::permissions::ToolCategory::WebSearch => tp.web_search,
                    crate::permissions::ToolCategory::Core => true,
                });
                if permission_granted && !crate::security::is_critical_risk(&risk) {
                    return None;
                }
            }

            // SecurityGuard RequireConfirm: auto-deny without pushing to frontend.
            // Only circuit-breaker Halt notifies the user; intermediate denials are
            // counted silently and injected as warnings to the LLM.
            warnings.push(format!(
                "⚠ 安全规则拦截：{}。此路径不可行，请改用替代方法。",
                reason
            ));
            *safety_counter += 1;
            return Some(ToolResult::failure(format!("安全规则拦截:{}", reason)));
        }
    }

    None
}

/// Determine tool category (for category permission switch check + auto-allow decision)
fn categorize_tool(tool: &str) -> Option<crate::permissions::ToolCategory> {
    if tool.starts_with("desktop_") || tool.starts_with("system_") {
        Some(crate::permissions::ToolCategory::SystemAutomation)
    } else if tool.starts_with("browser_") || tool.starts_with("web_") {
        Some(crate::permissions::ToolCategory::WebSearch)
    } else {
        match tool {
            // File operation tools
            "Read" | "Write" | "Edit" | "Rename" | "Delete" | "Copy" | "CreateDir"
            | "RemoveDir" | "FilesInfo" | "list_dir" | "grep_file" | "glob" | "Diff"
            | "file_find" | "search_files" | "read_multiple" => {
                Some(crate::permissions::ToolCategory::FileAccess)
            }
            // System automation tools
            "system_shell"
            | "process_list"
            | "process_kill"
            | "task_dispatch"
            | "schedule_task"
            | "leader::memory_update" => Some(crate::permissions::ToolCategory::SystemAutomation),
            // Web search tools
            "search_web"
            | "web_extract"
            | "web_search"
            | "video_subtitle_extract"
            | "http_request" => Some(crate::permissions::ToolCategory::WebSearch),
            _ => None,
        }
    }
}

/// Execute tool (without safety checks)
/// `on_line` callback receives (line, is_stderr), tool output is called line by line (used for ToolOutputLine event)
pub async fn execute_tool_only(
    tools: &ToolRegistry,
    tool: &str,
    params: &serde_json::Value,
    on_line: Option<Box<dyn Fn(String, bool) + Send>>,
    emitter: Option<&dyn EventEmitter>,
) -> ToolResult {
    tracing::info!(tool = %tool, "tool execution start");

    // ── HUD: browser/desktop 操作统一推送，所有 Agent 共用 ──
    let is_ui_op = tool.starts_with("browser_") || tool.starts_with("desktop_");
    if is_ui_op {
        if let Some(em) = emitter {
            em.emit(NuphusEvent::HudUpdate {
                text: format!("{}…", tool),
                phase: "running".into(),
                step_kind: None,
            });
        }
    }

    let exec_start = std::time::Instant::now();

    let result = if tool.starts_with("browser_") {
        match tools.execute_browser_tool(tool, params).await {
            Ok(result) => result,
            Err(e) => return ToolResult::failure(e),
        }
    } else {
        match tools.execute(tool, params).await {
            Ok(result) => result,
            Err(e) => return ToolResult::failure(e),
        }
    };

    let elapsed_ms = exec_start.elapsed().as_millis() as u64;
    tracing::info!(tool = %tool, duration_ms = elapsed_ms, success = result.success, "tool execution end");
    // If on_line callback is provided, emit output line by line
    if let Some(cb) = &on_line {
        if let Some(ref output) = result.output {
            for line in output.lines() {
                cb(line.to_string(), false);
            }
        }
        if let Some(ref error) = result.error {
            for line in error.lines() {
                cb(line.to_string(), true);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_security_warning_generic_contains_reason() {
        let msg = build_security_warning("路径包含 Nuphus 自身源码");
        assert!(msg.contains("自身源码"));
    }

    #[test]
    fn test_build_security_warning_generic() {
        let msg = build_security_warning("高危命令");
        assert!(msg.contains("安全拦截"));
    }

    #[test]
    fn test_safety_checks() {
        let r = SafetyCheck::allow();
        assert!(!r.should_block);
        assert!(r.block_reason.is_none());

        let r = SafetyCheck::block("权限不足");
        assert!(r.should_block);
        assert_eq!(r.block_reason.unwrap(), "权限不足");

        let r = SafetyCheck::breaker("连续3次失败");
        assert!(r.breaker_triggered);
    }
}
