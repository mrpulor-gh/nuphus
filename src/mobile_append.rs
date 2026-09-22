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

/// 判定「执行中提交」是否与本轮主指令重复 —— **整轮有效，不设时间窗口**。
///
/// 旧实现用 30 秒窗口（`elapsed_since_process_start < 30`），在分钟级长任务中形同虚设：
/// 实测出现过「正常发出的指令在执行中被重复提交（界面重载 / 前端热更新），随后以
/// `[APPEND]` 段再次注入 Agent 上下文」。执行期间提交同一内容没有合法语义
/// （要追加新内容，内容必然不同），故整轮无条件拒绝。
///
/// trim 后比较：前后空白差异不应绕过去重（与 [`enqueue`] 的队列内比较同口径）。
/// 三条提交/入队路径（桌面 busy 分支、`append_instruction`、手机端 busy 与竞态兜底）
/// 统一走本函数，避免各处自行维护时间窗口导致口径漂移。
pub fn is_duplicate_of_last(last_message: &str, message: &str) -> bool {
    let m = message.trim();
    !m.is_empty() && last_message.trim() == m
}

/// 判断文本是否为追加指令段（chat_history 过滤用）。
pub fn is_append_section(text: &str) -> bool {
    text.starts_with(APPEND_MARKER)
}

/// 格式化为注入 Agent 上下文的追加指令段。
pub fn format_mobile_append_section(appends: &[String]) -> String {
    let mut body = String::from(APPEND_MARKER);
    body.push_str("\n用户在执行过程中追加了指令（以下为最新指令）。它代表当前最高优先级意图；如与旧计划冲突，立即放弃冲突的旧计划并按新指令收敛：");
    for append in appends {
        body.push_str("\n- ");
        body.push_str(append);
    }
    body
}

/// Format the same append batch as a short critical reminder. Execution loops use
/// this after clearing stale deviation reminders so the newest user intent cannot
/// be buried below an older exploration plan.
pub fn format_mobile_append_priority(appends: &[String]) -> String {
    let mut body = String::from("最新用户追加指令必须立即执行；它覆盖所有与之冲突的旧探索计划：");
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
    fn duplicate_of_last_has_no_time_window_and_ignores_padding() {
        // 整轮有效：无时间参数，任何时刻同内容都判重（旧实现 30s 后放行 = 形同虚设）
        assert!(is_duplicate_of_last("继续", "继续"));
        assert!(is_duplicate_of_last(" 继续 ", "继续"));
        assert!(is_duplicate_of_last("继续", " 继续"));
        // 不同内容 → 放行（多条追加不合并）
        assert!(!is_duplicate_of_last("继续", "换个话题"));
        // 空提交 → 不判重（空消息由入口层校验拒绝，不由此处伪装成重复）
        assert!(!is_duplicate_of_last("继续", "   "));
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
    fn append_section_explicitly_overrides_conflicting_old_plan() {
        let section = format_mobile_append_section(&["立即保存工作流".to_string()]);
        assert!(section.contains("最高优先级"));
        assert!(section.contains("放弃冲突的旧计划"));

        let priority = format_mobile_append_priority(&["立即保存工作流".to_string()]);
        assert!(priority.contains("覆盖所有与之冲突的旧探索计划"));
        assert!(priority.contains("立即保存工作流"));
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
