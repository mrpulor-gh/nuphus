//! diagnostics — Diagnostic event emission before and after LLM calls
//!
//! emit_exec / emit_pre_llm_diag / emit_post_llm_diag

use crate::session::ContentBlock;

impl super::ReactAgent {
    pub(crate) fn emit_exec(&self, content: &str) {
        tracing::info!("[EXEC] {}", content);
    }

    /// Diagnostic: only print warning when mixed (some have/don't have reasoning_content)
    pub(crate) fn emit_pre_llm_diag(&self, _iter: usize, messages: &[serde_json::Value]) {
        let mut with_rc = 0u32;
        let mut without_rc = 0u32;
        for msg in messages {
            if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                continue;
            }
            if msg.get("reasoning_content").is_some() {
                with_rc += 1;
            } else {
                without_rc += 1;
            }
        }
        if with_rc > 0 && without_rc > 0 {
            for (i, msg) in messages.iter().enumerate() {
                if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                    continue;
                }
                let rc = msg.get("reasoning_content").is_some();
                let tc = msg.get("tool_calls").is_some();
                self.emit_exec(&format!(
                    "// diag: api_msg[{}] asst rc={} tc={}",
                    i,
                    if rc { "Y" } else { "N" },
                    if tc { "Y" } else { "N" }
                ));
            }
        }
    }

    /// Diagnostic: only print when reasoning is present (don't output meaningless lines in quiet mode)
    pub(crate) fn emit_post_llm_diag(&self, iter: usize, blocks: &[ContentBlock]) {
        let (rc, rc_len) = blocks
            .iter()
            .find_map(|b| match b {
                ContentBlock::Text {
                    reasoning: Some(r), ..
                } => Some((true, r.len())),
                _ => None,
            })
            .unwrap_or((false, 0));
        if rc {
            let tc_count = blocks
                .iter()
                .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
                .count();
            self.emit_exec(&format!(
                "// reasoning: {} chars, {} tool_calls (iter {})",
                rc_len, tc_count, iter
            ));
        }
    }
}
