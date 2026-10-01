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

// ── Refine 分档 ────────────────────────────────────────────────────────────

/// 按模型上下文窗口分档——refine 策略的唯一分档依据。
///
/// 为什么必须分档：小窗口模型「随便执行一下就达到阈值」，甚至强制线与撑爆之间
/// 只差一个任务的长度，给它可调区间毫无意义（调与不调都改变不了什么），提示更是
/// 纯噪音；而大窗口模型的主要矛盾是**防漂移**——上下文缓慢劣化比撑爆更早发生，
/// 需要「尽早被告知 + 自行决定何时动手」。
///
/// 档位边界取 256K / 600K：256K 恰在现有元数据最高档（Anthropic 200K）之上的
/// 2 的幂，**现有所有模型一律落入 Small 档，零行为回归**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefineTier {
    /// cw ≤ 256K：无提示，75% 强制提炼。
    Small,
    /// 256K < cw ≤ 600K：50% 提示，80% 强制提炼（两条线均固定）。
    Medium,
    /// cw > 600K：30% 提示，强制线可调（默认 50% = 范围下限，范围 50%–80%）。
    Large,
}

/// 大窗口强制线的可调范围（比例）。
pub const LARGE_FORCE_MIN: f64 = 0.50;
pub const LARGE_FORCE_MAX: f64 = 0.80;
/// 大窗口强制线默认值——**取范围下限**。
///
/// 这个默认值必须是**范围下限**而不是某个中间值：这套设置的目的是**延后与防漂移**
/// （用户按实际任务自行决定何时动手），不是兜底。若默认给到 80%，它与 Medium 档的
/// 固定值毫无区别，用户根本没有去碰它的理由——设置就白做了。
pub const LARGE_FORCE_DEFAULT: f64 = LARGE_FORCE_MIN;

impl RefineTier {
    pub fn for_window(context_window: usize) -> Self {
        if context_window <= 256_000 {
            RefineTier::Small
        } else if context_window <= 600_000 {
            RefineTier::Medium
        } else {
            RefineTier::Large
        }
    }

    /// 提示线比例；`None` = 该档没有提示阶段（到达强制线直接提炼）。
    pub fn prompt_ratio(&self) -> Option<f64> {
        match self {
            RefineTier::Small => None,
            RefineTier::Medium => Some(0.50),
            RefineTier::Large => Some(0.30),
        }
    }

    /// 强制线比例。仅 [`RefineTier::Large`] 接受用户配置，其余档返回固定值。
    /// 传入值会被 clamp 进 `LARGE_FORCE_MIN..=LARGE_FORCE_MAX`，
    /// 防止越界配置把强制线压到提示线以下（那会让提示永不触发）。
    ///
    /// 另有一层隐式保护：本方法**只对 large 档读配置**，small/medium 一律返回
    /// 设计定值。因此用户在大窗口模型上把线调到 50% 之后切到 600K 以下的模型，
    /// 新档位完全不受那个配置影响——不存在"大模型设置污染小模型"的问题。
    pub fn force_ratio(&self, configured_large: f64) -> f64 {
        match self {
            RefineTier::Small => 0.75,
            RefineTier::Medium => 0.80,
            RefineTier::Large => configured_large.clamp(LARGE_FORCE_MIN, LARGE_FORCE_MAX),
        }
    }

    /// 档位标识，随事件下发供前端决定 UI 形态（是否给 slider）。
    pub fn as_str(&self) -> &'static str {
        match self {
            RefineTier::Small => "small",
            RefineTier::Medium => "medium",
            RefineTier::Large => "large",
        }
    }
}

/// Unified refinement check — usable by any Agent type after ReAct loop ends
///
/// 三分档策略见 [`RefineTier`]。实际 token 优先取官方读数
/// （`api_input_tokens`），取不到才回落到字符估算。
pub async fn maybe_refine_session(
    session: &mut Session,
    context_window: usize,
    large_force_threshold: f64,
    emitter: Option<&dyn EventEmitter>,
    refine_count: &mut u32,
) {
    let actual_tokens = if session.api_input_tokens > 0 {
        session.api_input_tokens as usize
    } else {
        session.estimate_token_usage()
    };

    let tier = RefineTier::for_window(context_window);
    let force_ratio = tier.force_ratio(large_force_threshold);
    let force_limit = ((context_window as f64) * force_ratio) as usize;

    // 无提示档：到达强制线才动作，且直接执行（不问用户）。
    let Some(prompt_ratio) = tier.prompt_ratio() else {
        if actual_tokens < force_limit {
            return;
        }
        tracing::info!(
            "[REFINE] tier=small cw={} tokens={} >= force_limit={} ({:.0}%) — refine without prompting",
            context_window,
            actual_tokens,
            force_limit,
            force_ratio * 100.0
        );
        if let Some(em) = emitter {
            em.emit(crate::agent::events::NuphusEvent::RefinePrompt {
                current_tokens: actual_tokens as u32,
                refine_limit: force_limit as u32,
                force_limit: force_limit as u32,
                threshold: force_ratio,
                context_window: context_window as u32,
                tier: tier.as_str().to_string(),
                force_threshold: force_ratio,
                force_min: LARGE_FORCE_MIN,
                force_max: LARGE_FORCE_MAX,
                forced: true,
            });
        }
        *refine_count += 1;
        return;
    };

    let prompt_limit = ((context_window as f64) * prompt_ratio) as usize;

    if actual_tokens < prompt_limit {
        return;
    }

    // 已越过强制线：不再询问，直接执行。
    //
    // 曾想区分「水位自己涨上来的」与「用户把线拖到当前水位以下」两种触发并
    // 分开记日志，但后端不持有「用户上次设置值」的状态，该区分条件恒假
    // （force_limit 已被 clamp 到 >= 默认线）——2026-10 删除该死分支，
    // 统一记 warn，「为什么触达」由前后端事件链本身可解释。
    if actual_tokens >= force_limit {
        tracing::warn!(
            "[REFINE] tier={} cw={} tokens={} >= force_limit={} ({:.0}%) — forced refine",
            tier.as_str(),
            context_window,
            actual_tokens,
            force_limit,
            force_ratio * 100.0
        );
        if let Some(em) = emitter {
            em.emit(crate::agent::events::NuphusEvent::RefinePrompt {
                current_tokens: actual_tokens as u32,
                refine_limit: prompt_limit as u32,
                force_limit: force_limit as u32,
                threshold: prompt_ratio,
                context_window: context_window as u32,
                tier: tier.as_str().to_string(),
                force_threshold: force_ratio,
                force_min: LARGE_FORCE_MIN,
                force_max: LARGE_FORCE_MAX,
                forced: true,
            });
        }
        *refine_count += 1;
        return;
    }

    tracing::info!(
        "[REFINE] tier={} cw={} tokens={} >= prompt_limit={} ({:.0}%), force at {:.0}% — asking user",
        tier.as_str(),
        context_window,
        actual_tokens,
        prompt_limit,
        prompt_ratio * 100.0,
        force_ratio * 100.0
    );
    if let Some(em) = emitter {
        em.emit(crate::agent::events::NuphusEvent::RefinePrompt {
            current_tokens: actual_tokens as u32,
            refine_limit: prompt_limit as u32,
            force_limit: force_limit as u32,
            threshold: prompt_ratio,
            context_window: context_window as u32,
            tier: tier.as_str().to_string(),
            force_threshold: force_ratio,
            force_min: LARGE_FORCE_MIN,
            force_max: LARGE_FORCE_MAX,
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
        large_force_threshold: f64,
    ) {
        let _ = cancel_flag;
        let emitter_ref: Option<&dyn EventEmitter> = self.exec_emitter.as_deref();
        maybe_refine_session(
            &mut self.session,
            context_window,
            large_force_threshold,
            emitter_ref,
            &mut self.refine_count,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 档位边界：256K / 600K 是闭区间/开区间的分界，必须逐点钉住。
    #[test]
    fn tier_boundaries() {
        assert_eq!(RefineTier::for_window(0), RefineTier::Small);
        assert_eq!(
            RefineTier::for_window(128_000),
            RefineTier::Small,
            "主流 128K"
        );
        assert_eq!(
            RefineTier::for_window(200_000),
            RefineTier::Small,
            "Anthropic 200K"
        );
        assert_eq!(
            RefineTier::for_window(256_000),
            RefineTier::Small,
            "256K 含在上界内"
        );
        assert_eq!(RefineTier::for_window(256_001), RefineTier::Medium);
        assert_eq!(
            RefineTier::for_window(600_000),
            RefineTier::Medium,
            "600K 含在上界内"
        );
        assert_eq!(RefineTier::for_window(600_001), RefineTier::Large);
        assert_eq!(RefineTier::for_window(1_000_000), RefineTier::Large);
    }

    /// 三档的提示/强制比例必须与设计一致。
    #[test]
    fn tier_ratios_match_design() {
        assert_eq!(RefineTier::Small.prompt_ratio(), None, "小窗口无提示");
        assert_eq!(RefineTier::Small.force_ratio(0.0), 0.75, "小窗口固定 75%");

        assert_eq!(RefineTier::Medium.prompt_ratio(), Some(0.50));
        assert_eq!(RefineTier::Medium.force_ratio(0.0), 0.80, "中窗口固定 80%");

        assert_eq!(
            RefineTier::Large.prompt_ratio(),
            Some(0.30),
            "大窗口 30% 提示"
        );
        assert_eq!(
            RefineTier::Large.force_ratio(LARGE_FORCE_DEFAULT),
            LARGE_FORCE_DEFAULT
        );
    }

    /// 可调值必须被 clamp 进 [50%, 80%]：越界配置不能让强制线压到提示线以下
    /// （那会让提示永不触发），也不能放到 80% 以上（失去"强制"意义）。
    #[test]
    fn large_force_ratio_is_clamped() {
        assert_eq!(
            RefineTier::Large.force_ratio(0.10),
            LARGE_FORCE_MIN,
            "低于下限被抬回"
        );
        assert_eq!(
            RefineTier::Large.force_ratio(0.99),
            LARGE_FORCE_MAX,
            "高于上限被压回"
        );
        assert_eq!(RefineTier::Large.force_ratio(0.65), 0.65, "区间内原样生效");
        // 小/中档必须忽略用户配置——它们的比例是设计定值，不随配置漂移
        assert_eq!(RefineTier::Small.force_ratio(0.10), 0.75);
        assert_eq!(RefineTier::Medium.force_ratio(0.99), 0.80);
    }

    /// 大窗口默认值即范围下限：设置的意义是"按需延后/提前"，默认给范围上限
    /// 会让它和 Medium 档的固定值没有区别，用户没有理由去调。
    #[test]
    fn large_default_is_range_minimum() {
        assert_eq!(LARGE_FORCE_DEFAULT, LARGE_FORCE_MIN);
        assert_eq!(RefineTier::Large.force_ratio(LARGE_FORCE_DEFAULT), 0.50);
        // 默认值必须不等于 Medium 的固定值——否则 large 与 medium 默认行为无差异
        assert_ne!(LARGE_FORCE_DEFAULT, RefineTier::Medium.force_ratio(0.0));
    }
}
