//! Shared protection detector
//!
//! Provides three types of detection: dead loop, consecutive errors, over-collection.
//! Pure detection, does not modify Session; Agent decides how to handle detection results.

use crate::ToolCall;

/// Protection detection alert
#[derive(Debug, Clone)]
pub enum ProtectionAlert {
    /// Dead loop: same tool + same params repeated execution
    DeadLoop { tool: String, count: u32 },
    /// Consecutive error: same tool consecutive failures
    ConsecutiveError { tool: String, count: u32 },
    /// Same file read too many times consecutively
    /// count: current consecutive read count, max: upper limit (15)
    SameFileReadOveruse { count: u32, max: u32 },
}

/// Loop protection detector
///
/// Usage:
/// ```ignore
/// let mut guard = ProtectionGuard::new();
///
/// // Check for dead loop or over-collection before each tool call
/// if let Some(alert) = guard.check_pre_call(&call) {
/// // Agent decides: push_system / pending_reminder / return
/// }
///
/// // Mark write operation after successful tool call
/// if call.tool == "Write" { guard.mark_write(); }
///
/// // Check for consecutive errors after failed tool call
/// if let Some(alert) = guard.check_post_call(&call) {
/// // Agent decides
/// }
/// ```
pub struct ProtectionGuard {
    // ── Dead loop detection ──
    last_call_tool: String,
    last_call_params: String,
    last_call_count: u32,
    dead_loop_threshold: u32,

    // ── Consecutive error detection ──
    consecutive_error_tool: String,
    consecutive_error_count: u32,
    consecutive_error_threshold: u32,

    // ── Same file consecutive read detection (by param signature, not just path) ──
    consecutive_file_sig: String,
    consecutive_file_count: u32,
    consecutive_file_triggered: u32, // Highest triggered threshold (0/3/5/7) — originally designed as 5/10/15 three levels, later lowered to 3/5/7 for earlier intervention
}

impl Default for ProtectionGuard {
    fn default() -> Self {
        Self {
            last_call_tool: String::new(),
            last_call_params: String::new(),
            last_call_count: 0,
            dead_loop_threshold: 5,
            consecutive_error_tool: String::new(),
            consecutive_error_count: 0,
            consecutive_error_threshold: 2,
            consecutive_file_sig: String::new(),
            consecutive_file_count: 0,
            consecutive_file_triggered: 0,
        }
    }
}

impl ProtectionGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check before each tool call: dead loop + same file consecutive read
    pub fn check_pre_call(&mut self, call: &ToolCall) -> Option<ProtectionAlert> {
        // ── Same file consecutive read detection (by param signature) ──
        // Signature = path#offset#limit, reading different offsets doesn't count as duplicate
        let t = call.tool.clone();
        if t == "Read" {
            let path = call
                .params
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let offset = call
                .params
                .get("offset")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            let limit = call
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(500);
            let sig = if path.is_empty() {
                String::new()
            } else {
                format!("{}#{}#{}", path, offset, limit)
            };

            if !sig.is_empty() && sig == self.consecutive_file_sig {
                self.consecutive_file_count += 1;
                let triggered = self.consecutive_file_triggered;
                // Level 3: 7 times → system error
                if self.consecutive_file_count >= 7 && triggered < 7 {
                    self.consecutive_file_triggered = 7;
                    tracing::warn!(
                        guard = "file_read_overuse",
                        signature = %sig,
                        count = 7,
                        "protection: repeated file read (level 3)"
                    );
                    return Some(ProtectionAlert::SameFileReadOveruse { count: 7, max: 7 });
                }
                // Level 2: 5 times → reminder + mention of limit
                if self.consecutive_file_count >= 5 && triggered < 5 {
                    self.consecutive_file_triggered = 5;
                    tracing::warn!(
                        guard = "file_read_overuse",
                        signature = %sig,
                        count = 5,
                        "protection: repeated file read (level 2)"
                    );
                    return Some(ProtectionAlert::SameFileReadOveruse { count: 5, max: 7 });
                }
                // Level 1: 3 times → reminder
                if self.consecutive_file_count >= 3 && triggered < 3 {
                    self.consecutive_file_triggered = 3;
                    tracing::warn!(
                        guard = "file_read_overuse",
                        signature = %sig,
                        count = 3,
                        "protection: repeated file read (level 1)"
                    );
                    return Some(ProtectionAlert::SameFileReadOveruse { count: 3, max: 7 });
                }
            } else if !sig.is_empty() {
                // Switch file/params → reset counter
                self.consecutive_file_sig = sig;
                self.consecutive_file_count = 1;
                self.consecutive_file_triggered = 0;
            }
            // read_file is not checked by DeadLoop (SameFileReadOveruse already covers duplicate path+params)
            return None;
        } else {
            // Non-read_file tool resets file counter
            self.consecutive_file_sig.clear();
            self.consecutive_file_count = 0;
            self.consecutive_file_triggered = 0;
        }

        // Dead loop detection (non-read_file tools only)
        let params_str = serde_json::to_string(&call.params).unwrap_or_default();
        if call.tool == self.last_call_tool && params_str == self.last_call_params {
            self.last_call_count += 1;
        } else {
            self.last_call_count = 0;
        }
        self.last_call_tool = call.tool.clone();
        self.last_call_params = params_str;

        if self.last_call_count >= self.dead_loop_threshold {
            let count = self.last_call_count;
            self.last_call_count = 0;
            tracing::warn!(
                guard = "dead_loop",
                tool = %call.tool,
                count = count,
                "protection: dead loop detected"
            );
            return Some(ProtectionAlert::DeadLoop {
                tool: call.tool.clone(),
                count,
            });
        }

        None
    }

    /// Check for consecutive errors after tool execution failure
    pub fn check_post_call(&mut self, call: &ToolCall) -> Option<ProtectionAlert> {
        if call.tool == self.consecutive_error_tool {
            self.consecutive_error_count += 1;
        } else {
            self.consecutive_error_count = 1;
            self.consecutive_error_tool = call.tool.clone();
        }

        if self.consecutive_error_count >= self.consecutive_error_threshold {
            let count = self.consecutive_error_count;
            self.consecutive_error_count = 0;
            tracing::warn!(
                guard = "consecutive_error",
                tool = %call.tool,
                count = count,
                "protection: consecutive tool errors"
            );
            return Some(ProtectionAlert::ConsecutiveError {
                tool: call.tool.clone(),
                count,
            });
        }

        None
    }

    /// Reset consecutive error count (call after successful tool execution)
    pub fn reset_consecutive_errors(&mut self) {
        self.consecutive_error_count = 0;
    }

    /// Reset dead loop count (auto-resets on param/tool change, but can also be called manually)
    pub fn reset_dead_loop(&mut self) {
        self.last_call_count = 0;
    }
}

/// Generate corresponding prompt text based on alert type
impl ProtectionAlert {
    /// Generate prompt text for Session injection (executor.rs style)
    pub fn to_session_warning(&self) -> String {
        match self {
            ProtectionAlert::DeadLoop { tool, .. } => {
                format!(
                    "检测到死循环：工具 `{}` 重复调用且参数相同。请更换方法或直接给出最终结果。",
                    tool
                )
            }
            ProtectionAlert::ConsecutiveError { tool, .. } => {
                format!(
                    "工具 `{}` 连续失败，请尝试其他方法或检查参数后重试。",
                    tool,
                )
            }
            ProtectionAlert::SameFileReadOveruse { count, max } => {
                match count {
                    3 => "检测到同一文件连续读取 3 次。如需反复读取，请确认是否必要，或考虑基于已掌握信息继续推进。".to_string(),
                    5 => format!("同一文件已连续读取 5 次（上限 {} 次）。请停止读取该文件，基于已掌握信息做决策。", max),
                    _ => format!("同一文件已连续读取 {} 次。请立即停止读取该文件，基于已掌握信息做决策。", max),
                }
            }
        }
    }

    /// Generate forced output reminder
    pub fn to_force_output_reminder(&self) -> Option<&'static str> {
        match self {
            ProtectionAlert::DeadLoop { .. } => {
                Some("检测到重复操作循环。请立即停止工具调用，直接回复用户当前进展和结论。")
            }
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProtectionAlert::DeadLoop { .. } => "dead-loop",
            ProtectionAlert::ConsecutiveError { .. } => "consecutive-error",
            ProtectionAlert::SameFileReadOveruse { .. } => "file-read-overuse",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_call(tool: &str, params: serde_json::Value) -> ToolCall {
        ToolCall::new(tool, params)
    }

    fn shell_cmd(cmd: &str) -> ToolCall {
        ToolCall::new("system_shell", serde_json::json!({"command": cmd}))
    }

    /// Dead-loop detection: same tool+params 6 consecutive times triggers (using non-file_read tool to avoid SameFileReadOveruse interference)
    #[test]
    fn test_dead_loop_detection() {
        let mut guard = ProtectionGuard::new();
        let call = shell_cmd("echo test");

        for i in 1..=6 {
            let alert = guard.check_pre_call(&call);
            if i < 6 {
                assert!(alert.is_none(), "alert fired at iteration {}", i);
            } else {
                assert!(alert.is_some());
                match alert.unwrap() {
                    ProtectionAlert::DeadLoop { tool, count } => {
                        assert_eq!(tool, "system_shell");
                        assert_eq!(count, 5);
                    }
                    _ => panic!("wrong alert type"),
                }
            }
        }
    }

    #[test]
    fn test_dead_loop_resets_on_param_change() {
        let mut guard = ProtectionGuard::new();
        let call1 = shell_cmd("cmd_a");
        let call2 = shell_cmd("cmd_b");

        // 5x call1 — 6th time triggers
        for i in 1..=5 {
            assert!(
                guard.check_pre_call(&call1).is_none(),
                "premature alert at {}",
                i
            );
        }
        // Param change resets counter
        assert!(guard.check_pre_call(&call2).is_none());
    }

    #[test]
    fn test_consecutive_error() {
        let mut guard = ProtectionGuard::new();
        let call = make_call("system_shell", serde_json::json!({"command": "bad_cmd"}));

        assert!(guard.check_post_call(&call).is_none()); // 1st failure
        let alert = guard.check_post_call(&call); // 2nd consecutive
        assert!(alert.is_some());
        match alert.unwrap() {
            ProtectionAlert::ConsecutiveError { tool, .. } => {
                assert_eq!(tool, "system_shell");
            }
            _ => panic!("wrong alert type"),
        }
    }

    #[test]
    fn test_consecutive_error_resets_on_tool_change() {
        let mut guard = ProtectionGuard::new();
        let call1 = make_call("tool_a", serde_json::json!({}));
        let call2 = make_call("tool_b", serde_json::json!({}));

        guard.check_post_call(&call1);
        assert!(guard.check_post_call(&call2).is_none()); // different tool
    }

    /// Same file read 3 times consecutively → Level 1 warning
    #[test]
    fn test_same_file_read_level1() {
        let mut guard = ProtectionGuard::new();
        for i in 1..=3 {
            let call = make_call("Read", serde_json::json!({"path": "big.txt"}));
            let alert = guard.check_pre_call(&call);
            if i < 3 {
                assert!(alert.is_none(), "premature alert at call {}", i);
            } else {
                assert!(alert.is_some());
                match alert.unwrap() {
                    ProtectionAlert::SameFileReadOveruse { count, max } => {
                        assert_eq!(count, 3);
                        assert_eq!(max, 7);
                    }
                    other => panic!("wrong alert type: {:?}", other),
                }
            }
        }
    }

    /// Same file read 5 times consecutively → Level 2 warning
    #[test]
    fn test_same_file_read_level2() {
        let mut guard = ProtectionGuard::new();
        // First trigger Level 1 (3 times)
        for _ in 0..3 {
            let call = make_call("Read", serde_json::json!({"path": "big.txt"}));
            let _ = guard.check_pre_call(&call);
        }
        // Read 2 more times to 5 → Level 2
        for i in 4..=5 {
            let call = make_call("Read", serde_json::json!({"path": "big.txt"}));
            let alert = guard.check_pre_call(&call);
            if i < 5 {
                assert!(alert.is_none(), "premature alert at call {}", i);
            } else {
                assert!(alert.is_some());
                match alert.unwrap() {
                    ProtectionAlert::SameFileReadOveruse { count, max } => {
                        assert_eq!(count, 5);
                        assert_eq!(max, 7);
                    }
                    other => panic!("wrong alert type: {:?}", other),
                }
            }
        }
    }

    /// Same file read 7 times consecutively → Level 3 error
    #[test]
    fn test_same_file_read_level3() {
        let mut guard = ProtectionGuard::new();
        // First 3 times → Level 1
        for _ in 0..3 {
            let call = make_call("Read", serde_json::json!({"path": "big.txt"}));
            let _ = guard.check_pre_call(&call);
        }
        // Another 2 times to 5 → Level 2
        for _ in 0..2 {
            let call = make_call("Read", serde_json::json!({"path": "big.txt"}));
            let _ = guard.check_pre_call(&call);
        }
        // Another 2 times to 7 → Level 3
        for i in 6..=7 {
            let call = make_call("Read", serde_json::json!({"path": "big.txt"}));
            let alert = guard.check_pre_call(&call);
            if i < 7 {
                assert!(alert.is_none(), "premature alert at call {}", i);
            } else {
                assert!(alert.is_some());
                match alert.unwrap() {
                    ProtectionAlert::SameFileReadOveruse { count, max } => {
                        assert_eq!(count, 7);
                        assert_eq!(max, 7);
                    }
                    other => panic!("wrong alert type: {:?}", other),
                }
            }
        }
    }

    /// Switch file path → counter reset
    #[test]
    fn test_same_file_read_resets_on_path_change() {
        let mut guard = ProtectionGuard::new();
        // Read file A 2 times (no trigger, threshold 3)
        for _ in 0..2 {
            let call = make_call("Read", serde_json::json!({"path": "a.txt"}));
            assert!(guard.check_pre_call(&call).is_none());
        }
        // Switch to file B (reset)
        let call = make_call("Read", serde_json::json!({"path": "b.txt"}));
        assert!(guard.check_pre_call(&call).is_none());
        // Read A again 2 times (starting from 0)
        for _ in 0..2 {
            let call = make_call("Read", serde_json::json!({"path": "a.txt"}));
            assert!(guard.check_pre_call(&call).is_none());
        }
    }

    /// Non-file_read tool resets file counter
    #[test]
    fn test_file_counter_resets_on_other_tool() {
        let mut guard = ProtectionGuard::new();
        // 2 file_read calls (no trigger, threshold 3)
        for _ in 0..2 {
            let call = make_call("Read", serde_json::json!({"path": "a.txt"}));
            assert!(guard.check_pre_call(&call).is_none());
        }
        // Non-file_read tool should reset counter
        let shell = shell_cmd("echo hello");
        assert!(guard.check_pre_call(&shell).is_none());
        // Read A again should start counting from 1
        let call = make_call("Read", serde_json::json!({"path": "a.txt"}));
        assert!(guard.check_pre_call(&call).is_none()); // 第一次，count=1
    }

    #[test]
    fn test_same_file_read_no_false_positive_on_different_files() {
        let mut guard = ProtectionGuard::new();
        for i in 1..=20 {
            let call = make_call(
                "Read",
                serde_json::json!({"path": format!("file_{}.txt", i)}),
            );
            assert!(
                guard.check_pre_call(&call).is_none(),
                "unexpected alert at file {}",
                i
            );
        }
    }

    /// Segmented reads (same path, different offsets) should not trigger SameFileReadOveruse
    #[test]
    fn test_segmented_read_does_not_trigger() {
        let mut guard = ProtectionGuard::new();
        let offsets = [1, 500, 1000, 1500, 2000];
        for i in 0..20 {
            let call = make_call(
                "Read",
                serde_json::json!({
                    "path": "big.rs",
                    "offset": offsets[i % offsets.len()],
                    "limit": 500
                }),
            );
            assert!(
                guard.check_pre_call(&call).is_none(),
                "segmented read should not trigger, but did at call {}",
                i + 1
            );
        }
    }

    /// Identical params repeated reads → should trigger SameFileReadOveruse
    #[test]
    fn test_identical_call_triggers_overuse() {
        let mut guard = ProtectionGuard::new();
        let call = make_call(
            "Read",
            serde_json::json!({
                "path": "big.rs",
                "offset": 100,
                "limit": 200
            }),
        );
        // First 2 times no trigger (threshold 3)
        for _ in 0..2 {
            assert!(guard.check_pre_call(&call).is_none());
        }
        // 3rd time triggers Level 1
        let alert = guard.check_pre_call(&call);
        assert!(alert.is_some());
        match alert.unwrap() {
            ProtectionAlert::SameFileReadOveruse { count, max: _ } => {
                assert_eq!(count, 3);
            }
            _ => panic!("expected SameFileReadOveruse"),
        }
    }

    #[test]
    fn test_no_false_positive_on_clean_run() {
        let mut guard = ProtectionGuard::new();
        let tools = [
            ("Read", serde_json::json!({"path": "a.txt"})),
            ("system_shell", serde_json::json!({"command": "echo 1"})),
            ("Write", serde_json::json!({"path": "out.txt"})),
            ("Read", serde_json::json!({"path": "b.txt"})),
            ("system_shell", serde_json::json!({"command": "echo 2"})),
        ];

        for (tool, params) in &tools {
            let call = make_call(tool, params.clone());
            assert!(guard.check_pre_call(&call).is_none());
        }
    }
}
