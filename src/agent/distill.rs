//! Session distillation — generic context refinement engine
//!
//! Standalone functions usable by any Agent type (Leader, WorkflowAgent, etc.).
//! ReactAgent methods below are thin wrappers.

use std::sync::atomic::AtomicBool;

use crate::agent::events::EventEmitter;
use crate::agent::ReactAgent;
use crate::memory::entry::AgentType;
use crate::session::Session;

// ── Standalone distillation functions ───────────────────────────────────────

/// Standard refine prompt sent to agents for session context refinement
/// Refine prompt: user-level request to distill the session into a compact context summary.
/// Must read as a user message (not a system instruction) to prevent the LLM from
/// treating it as a meta-directive that only triggers internal reasoning without text output.
pub const REFINE_PROMPT: &str = "开始进行上下文提炼，把以上session内完整会话提炼成一份紧凑的上下文摘要，禁止调用任何工具，直接输出最终文本。\n\
\n\
**第一行必须是这段会话的简短标题（10-30字），概括本轮任务的核心内容。**\n\
\n\
标题后空一行，再输出详细摘要。\n\
\n\
保留：\n\
- 决策链：每个决定的依据、推理过程、备选方案\n\
- 阻塞与解决：遇到什么问题、如何定位、解决状态\n\
- 文件与路径：涉及的关键文件、各自角色、修改历史\n\
- 当前状态：进行到哪一步、下一步计划、待验证假设\n\
\n\
丢弃：\n\
- 纯问候、已完成的确认、重复说明等无信息量内容\n\
\n\
输出格式：\n\
标题\n\
（空一行）\n\
详细摘要...\n\
\n\
只输出提炼内容文本，禁止携带任何无关语句，结果将自动保存记忆。\n\
\n\
注意：如果前文已有以「[当前 session 对话内容已触发提炼」开头的 System 消息，那是之前的提炼摘要——不要重复摘要它们已经涵盖的内容，只提炼其后出现的新消息。";

/// Write refinement result to memory system — usable by any Agent type
pub fn save_refine_entry(
    session_id: &str,
    turn_id: &str,
    summary: &str,
    source: &str,
    agent_type: AgentType,
) -> std::result::Result<(), String> {
    let mut entry = crate::memory::entry::MemoryEntry::new(
        format!("refine-{}-{}", session_id, chrono::Utc::now().timestamp()),
        session_id.to_string(),
        turn_id.to_string(),
        agent_type,
        crate::memory::entry::MemoryKind::Distill,
    );
    let intent = summary
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with('[') && !l.starts_with('`'))
        .unwrap_or(summary);
    entry.intent = intent.chars().take(100).collect();
    entry.summary = summary.chars().take(8000).collect();
    entry.success = true;
    entry.goal_type = Some("session_refine".to_string());
    entry.tags = vec!["session_refine".to_string(), source.to_string()];

    match crate::store::memory::insert_entry(&entry) {
        Ok(_) => {
            tracing::info!("Refined entry saved to memory: source={}", source);
            Ok(())
        }
        Err(e) => {
            let msg = format!("Failed to save refine entry (source={}): {}", source, e);
            tracing::warn!("{}", msg);
            Err(msg)
        }
    }
}

/// Unified refinement check — usable by any Agent type after ReAct loop ends
/// Emits RefinePrompt (forced=true for auto-refine, forced=false to ask user)
pub async fn maybe_refine_session(
    session: &mut Session,
    context_window: usize,
    refine_threshold: f64,
    emitter: Option<&dyn EventEmitter>,
    refine_count: &mut u32,
) {
    let actual_tokens = if session.api_input_tokens > 0 {
        session.api_input_tokens as usize
    } else {
        session.estimate_token_usage()
    };

    let refine_limit = ((context_window as f64) * refine_threshold) as usize;
    let effective_limit = refine_limit.min(300_000);
    let force_limit = ((context_window as f64) * 0.80) as usize;
    let force_limit = force_limit.min(500_000);

    if actual_tokens < effective_limit {
        return;
    }

    if actual_tokens >= force_limit {
        tracing::warn!(
            "[REFINE] Force refine triggered: tokens={} >= force_limit={}",
            actual_tokens,
            force_limit
        );

        if let Some(em) = emitter {
            em.emit(crate::agent::events::NuphusEvent::RefinePrompt {
                current_tokens: actual_tokens as u32,
                refine_limit: effective_limit as u32,
                force_limit: force_limit as u32,
                threshold: refine_threshold,
                context_window: context_window as u32,
                forced: true,
            });
        }
        *refine_count += 1;
        return;
    }

    tracing::info!(
        "[REFINE] Prompt user: tokens={} >= limit={}, threshold={:.0}%",
        actual_tokens,
        effective_limit,
        refine_threshold * 100.0
    );
    if let Some(em) = emitter {
        em.emit(crate::agent::events::NuphusEvent::RefinePrompt {
            current_tokens: actual_tokens as u32,
            refine_limit: effective_limit as u32,
            force_limit: force_limit as u32,
            threshold: refine_threshold,
            context_window: context_window as u32,
            forced: false,
        });
    }
}

impl ReactAgent {
    pub fn save_refine_entry(
        &self,
        summary: &str,
        source: &str,
    ) -> std::result::Result<(), String> {
        save_refine_entry(
            &self.session.id,
            &self.session.current_turn_id(),
            summary,
            source,
            AgentType::Leader,
        )
    }

    pub async fn maybe_refine_session(
        &mut self,
        cancel_flag: &AtomicBool,
        context_window: usize,
        refine_threshold: f64,
    ) {
        let _ = cancel_flag;
        let emitter_ref: Option<&dyn EventEmitter> = self.exec_emitter.as_deref();
        maybe_refine_session(
            &mut self.session,
            context_window,
            refine_threshold,
            emitter_ref,
            &mut self.refine_count,
        )
        .await;
    }
}
