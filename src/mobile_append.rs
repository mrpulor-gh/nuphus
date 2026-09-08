//! Append instruction formatting and shared-queue helpers.
//!
//! All append instructions are stored in `SignalState::append_queue`.  This module
//! deliberately contains no queue-owned global state; execution loops consume the
//! shared queue at their iteration boundary.

/// Append section prefix used by history filtering.
pub const APPEND_MARKER: &str = "[APPEND]";

/// Enqueue one append instruction in the process-wide shared signal state.
///
/// Empty instructions and entries already waiting in the same queue are ignored.
/// Submission-level retry deduplication remains at the message entry points, while
/// this queue-level check protects the single shared queue from concurrent repeats.
pub fn enqueue(signals: &crate::state::SharedSignals, instr: String) -> bool {
    let trimmed = instr.trim();
    if trimmed.is_empty() {
        return false;
    }
    let mut state = crate::state::SignalState::write(signals);
    if state
        .append_queue
        .iter()
        .any(|queued| queued.trim() == trimmed)
    {
        tracing::info!("[AppendQueue] 丢弃重复追加指令（已在队列中）");
        return false;
    }
    state.append_queue.push(instr);
    true
}

/// 判断文本是否为追加指令段（chat_history 过滤用）。
pub fn is_append_section(text: &str) -> bool {
    text.starts_with(APPEND_MARKER)
}

/// 格式化为注入 Agent 上下文的追加指令段。
pub fn format_mobile_append_section(appends: &[String]) -> String {
    let mut body = String::from(APPEND_MARKER);
    body.push_str("\n用户在执行过程中追加了指令，请立即将其纳入当前任务，如有必要调整后续步骤：");
    for append in appends {
        body.push_str("\n- ");
        body.push_str(append);
    }
    body
}

/// 从追加指令段中提取用户原文（供 UI 历史显示）。
pub fn extract_append_user_text(text: &str) -> Option<String> {
    if !is_append_section(text) {
        return None;
    }
    let user_texts: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    if user_texts.is_empty() {
        None
    } else {
        Some(user_texts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_uses_shared_queue_and_deduplicates_waiting_items() {
        let signals = crate::state::new_shared_signals();
        assert!(enqueue(&signals, "第一条".to_string()));
        assert!(!enqueue(&signals, " 第一条 ".to_string()));
        assert_eq!(
            crate::state::SignalState::read(&signals).append_queue,
            ["第一条"]
        );
    }

    #[test]
    fn format_section_lists_all() {
        let section = format_mobile_append_section(&["A".to_string(), "B".to_string()]);
        assert!(section.contains("- A"));
        assert!(section.contains("- B"));
        assert!(section.contains("追加了指令"));
        assert!(is_append_section(&section));
    }

    #[test]
    fn extract_append_text_recovers_user_message() {
        let section = format_mobile_append_section(&["第一条".to_string(), "第二条".to_string()]);
        assert_eq!(
            extract_append_user_text(&section).as_deref(),
            Some("第一条\n第二条")
        );
        assert_eq!(extract_append_user_text("普通消息"), None);
        assert_eq!(
            extract_append_user_text("[APPEND]\n用户在执行过程中追加了指令"),
            None
        );
    }
}
