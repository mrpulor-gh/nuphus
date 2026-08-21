//! append_queue — 追加指令队列（桌面端 + 手机端共用）
//!
//! 执行中（busy 锁占用）用户发送的消息不再拒绝/丢弃，而是入队本队列，
//! 由 react_loop 每轮迭代边界被动 drain 并注入 session（与 handoff 门铃同一
//! 注入位），插入下一迭代——桌面端「执行中发送」与手机端「执行中发送」语义一致：
//! 追加消息只注入后端 LLM 上下文，前端不显示气泡，仅弹窗提示消息内容。
//!
//! ## 设计要点
//!
//! - 消息来源：桌面端（send_message_cmd busy 分支）与手机端（mobile_server busy
//!   分支）均经用户鉴权（本人指令），直接作为 user 消息注入，无需 UNTRUSTED_BOUNDARY
//! - 队列持久性：全局 static，跨 session 存活；正常路径下每轮迭代即 drain 清空
//! - 空队列零开销：drain 返回空 Vec，不注入、不产生日志
//! - 中毒恢复：持锁 panic 不应让追加通道永久不可用（模式对齐 handoff.rs）

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 全局追加指令队列
static PENDING: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// 最近一次注入的追加内容 + 注入时间（跨 drain 批次内容去重，防重复注入）
static LAST_INJECTED: OnceLock<Mutex<(String, Instant)>> = OnceLock::new();

/// 内容去重时间窗：同一追加内容在该窗口内只注入一次
/// （覆盖「刷新/重连后用户重发相同指令」「网络超时后重复提交」等场景）
const DEDUP_WINDOW: Duration = Duration::from_secs(60);

fn with_queue<R>(f: impl FnOnce(&mut Vec<String>) -> R) -> R {
    let m = PENDING.get_or_init(|| Mutex::new(Vec::new()));
    // 中毒恢复：持锁 panic 不应让追加通道永久不可用
    let mut guard = m.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// 入队（mobile_server post_message busy 分支调用）
/// 去重防线：与队列中已有内容相同 → 丢弃；与最近 DEDUP_WINDOW 内已注入内容相同 → 丢弃。
/// 同一指令在 UI 刷新/网络重试后可能被重复提交，重复注入会让 Agent 收到两条相同指令。
pub fn push(instr: String) {
    let trimmed = instr.trim();
    if trimmed.is_empty() {
        return;
    }
    let recently_injected = LAST_INJECTED
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
        .map(|(last, at)| last.trim() == trimmed && at.elapsed() < DEDUP_WINDOW)
        .unwrap_or(false);
    if recently_injected {
        tracing::info!(
            "[AppendQueue] 丢弃重复追加指令（{}s 内已注入）: {}",
            DEDUP_WINDOW.as_secs(),
            trimmed.chars().take(60).collect::<String>()
        );
        return;
    }
    with_queue(|q| {
        let t = instr.trim();
        if q.iter().any(|x| x.trim() == t) {
            tracing::info!("[AppendQueue] 丢弃重复追加指令（已在队列中）");
            return;
        }
        q.push(instr);
    });
}

/// 当前队列是否非空（测试/诊断用）
pub fn has_pending() -> bool {
    with_queue(|q| !q.is_empty())
}

/// 轮次边界被动 drain（react_loop 调用）。空队列返回空 Vec，零日志零开销。
/// 记录批次最后一条为「最近注入」——后续 push 据此做跨批内容去重。
pub fn drain_for_injection() -> Vec<String> {
    let drained = with_queue(std::mem::take);
    if let Some(last) = drained.last() {
        let m = LAST_INJECTED
            .get_or_init(|| Mutex::new((String::new(), Instant::now() - DEDUP_WINDOW)));
        if let Ok(mut g) = m.lock() {
            *g = (last.clone(), Instant::now());
        }
    }
    drained
}

/// 任务收口时清空未消费的追加队列。
/// 追加指令语义 =「发送时正在执行的任务内生效」；任务结束（react_loop/workflow 退出）后
/// 残留指令若不清空，会在下一个任务首轮被 drain 注入——跨任务泄漏（刷新/重连后尤甚）。
pub fn clear() {
    with_queue(|q| q.clear());
}

/// 追加指令段前缀标记：chat_history 据此过滤，追加消息不显示在对话历史中
pub const APPEND_MARKER: &str = "[APPEND]";

/// 判断文本是否为追加指令段（chat_history 过滤用）
pub fn is_append_section(text: &str) -> bool {
    text.starts_with(APPEND_MARKER)
}

/// 格式化为注入 Leader 上下文的追加指令段。
/// 追加指令来源：电脑端执行中发送（send_message_cmd busy 分支）/ 手机端发送
/// （mobile_server busy 分支），均经用户鉴权（本人指令），直接作为 user 消息注入。
/// 前缀 [APPEND] 供 chat_history 过滤——追加消息只进 LLM 上下文，不显示在 UI 历史。
pub fn format_mobile_append_section(appends: &[String]) -> String {
    let mut body = String::from(APPEND_MARKER);
    body.push_str("\n用户在执行过程中追加了指令，请立即将其纳入当前任务，如有必要调整后续步骤：");
    for a in appends {
        body.push_str("\n- ");
        body.push_str(a);
    }
    body
}

/// 从追加指令段中提取用户原文（供 UI 历史显示）。
///
/// [APPEND] 段是内部注入格式：`[APPEND]\n用户在执行过程中追加了指令…\n- 消息A\n- 消息B`。
/// 该段在 LLM 上下文中保留完整指令语义；但在前端历史中应以「用户原文」呈现——
/// 否则执行中发送的消息在刷新/重连后彻底消失（用户看不到自己发过的内容）。
/// 此函数剥离包装前缀，还原用户实际发送的文本；非追加段返回 None。
pub fn extract_append_user_text(text: &str) -> Option<String> {
    if !is_append_section(text) {
        return None;
    }
    let lines = text.lines();
    // 跳过 [APPEND] 首行与说明行，收集 `- ` 开头的用户内容
    let user_texts: Vec<&str> = lines
        .filter_map(|l| l.trim().strip_prefix("- "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if user_texts.is_empty() {
        return None;
    }
    Some(user_texts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_drain_roundtrip() {
        push("第一条".to_string());
        push("第二条".to_string());
        let drained = drain_for_injection();
        assert_eq!(drained, vec!["第一条".to_string(), "第二条".to_string()]);
        assert!(!has_pending());
    }

    #[test]
    fn push_dedup_same_content_in_queue() {
        clear();
        push("重复内容A".to_string());
        push("重复内容A".to_string());
        let drained = drain_for_injection();
        assert_eq!(drained.len(), 1, "同批相同内容只保留一条");
    }

    #[test]
    fn push_dedup_after_injection_window() {
        clear();
        push("重复内容B".to_string());
        let _ = drain_for_injection(); // 记录 LAST_INJECTED = B
        push("重复内容B".to_string());
        assert!(!has_pending(), "注入时间窗内重复内容应被丢弃");
    }

    #[test]
    fn clear_empties_queue() {
        clear();
        push("残留内容C".to_string());
        clear();
        assert!(!has_pending());
    }

    #[test]
    fn drain_empty_is_noop() {
        assert!(drain_for_injection().is_empty());
    }

    #[test]
    fn format_section_lists_all() {
        let s = format_mobile_append_section(&["A".to_string(), "B".to_string()]);
        assert!(s.contains("- A"));
        assert!(s.contains("- B"));
        assert!(s.contains("追加了指令"));
        assert!(is_append_section(&s), "format 输出应带 [APPEND] 标记");
    }

    #[test]
    fn is_append_section_detects_marker() {
        assert!(is_append_section("[APPEND]\n用户在执行过程中追加了指令"));
        assert!(!is_append_section("普通用户消息"));
        assert!(!is_append_section(""));
    }

    #[test]
    fn extract_append_text_recovers_user_message() {
        let s = format_mobile_append_section(&["第一条".to_string(), "第二条".to_string()]);
        let extracted = extract_append_user_text(&s);
        assert_eq!(extracted.as_deref(), Some("第一条\n第二条"));
        // 非追加段：None
        assert_eq!(extract_append_user_text("普通消息"), None);
        // 空追加：None
        assert_eq!(
            extract_append_user_text("[APPEND]\n用户在执行过程中追加了指令"),
            None
        );
    }
}
