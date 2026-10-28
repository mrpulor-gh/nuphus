//! Session distillation — generic context refinement engine
//!
//! Standalone functions usable by any Agent type (Leader, WorkflowAgent, etc.).
//! ReactAgent methods below are thin wrappers.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::agent::events::{EventEmitter, NuphusEvent};
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
- 决策链：已验证结论与证据、事实因果、备选方案\n\
- 确认的决策：拍板过的事项与口径\n\
- 成果/有效方法：达成的成果、可复用的方法\n\
- 文件与路径：已梳理架构关系、关键文件、各自角色、修改历史\n\
- 当前状态：进行到哪一步、下一步计划、待完成任务目标\n\
\n\
要求：\n\
- 丢弃纯问候、已完成的确认、重复说明等无信息量内容\n\
- 所有内容正向语义表达\n\
- 未验证结论与证据明确标注\n\
\n\
输出格式：\n\
标题\n\
（空一行）\n\
详细摘要...\n\
\n\
只输出提炼内容文本，禁止携带任何无关语句，结果将自动保存记忆。";

/// 执行中提炼提示词（in-loop refine）——Leader / WorkflowAgent 在 ReAct 循环
/// 内上下文越线时使用。与标准 [`REFINE_PROMPT`] 的关键差异：**任务未闭合**。
///
/// 为什么必须另写一份：`replace_with_distill`/`accumulate_distill` 会清空全部
/// 消息（session.rs），提炼后摘要将是 agent 唯一的任务记忆。标准版把
/// 「当前状态/下一步」放在保留列表里一带而过；未闭合场景下目标/进度/待办
/// 必须升级为**强制首部结构化**，否则续跑会偏离原始目标或重复已完成工作
/// （2026-10-25 大王定调：目标、已完成、未完成、待办、关键路径要具体清晰）。
pub const REFINE_PROMPT_UNFINISHED: &str = "[执行中上下文提炼 — 任务未完成]\n\
\n\
以上是一个正在执行中的任务会话，上下文用量已达阈值。现在把会话提炼为紧凑摘要，提炼完成后你将立即继续执行未尽事宜。禁止调用任何工具，直接输出最终文本。\n\
\n\
**第一行必须是简短标题（10-30字），概括任务核心内容。** 标题后空一行，输出摘要，摘要必须按以下结构：\n\
\n\
## 原始任务目标\n\
用户/上级下达的原始目标原文或其忠实转述。这是你提炼后唯一的任务记忆，必须完整保留意图与约束，不得丢失或改写。\n\
\n\
## 已完成\n\
逐项列出：完成了什么、产出在哪个文件/路径、关键结论、验证状态。\n\
\n\
## 当前进度\n\
进行到哪一步、手停在哪、上一步的产出是什么。\n\
\n\
## 待办与下一步\n\
按优先级列出未尽事宜与下一步行动——提炼后你将从这里直接继续。\n\
\n\
## 关键文件与路径\n\
涉及的文件/目录、各自角色、修改历史。\n\
\n\
## 关键中间数据与决策依据\n\
重要数值、配置、决策链与备选方案、阻塞与解决状态——防止重复探索。\n\
\n\
丢弃：纯问候、已完成的确认、重复说明等无信息量内容。\n\
只输出提炼内容文本，禁止携带任何无关语句。\n\
如果前文已有以「[当前 session 对话内容已触发提炼」开头的 System 消息，那是之前的提炼摘要——在其基础上增量更新（吸收其中仍然有效的内容），不要从零重写。";

/// 同一轮次内 in-loop refine 的次数上限。
///
/// 落地 `refine_count` 的真实约束（修复 B9：此前计数器只 `+= 1` 从不比较，
/// workflow_agent.rs 注释声称的「max 2 auto-refines」是过期声明）。超过上限
/// 后 in-loop 不再提炼，降级为软劝导 + turn-end 正常 refine 兜底——防
/// 「提炼→续跑→再越线→再提炼」无限循环烧超时预算。
///
/// 2026-10-25 定权：85% 是 in-loop 的唯一硬止损线与**唯一接力点**——force 线
/// （分档 75/80/50）只做软劝导（模型自己收敛，turn-end 正常 refine，不接力）；
/// 85% 越线当场提炼、压缩后从摘要就地续跑（任务未闭合，必须接力）。
///
/// 2026-10-28 上调：2 → 10（用户反馈长任务后段「超线却再无强制提炼」——2 次
/// 用尽后 85% 只剩劝导；10 次覆盖常规长任务全程，上限保护仍存）。
pub const MAX_IN_LOOP_REFINES: u32 = 10;

/// in-loop refine 的提交动作：摘要落地（压缩 + 归档）。
///
/// 与 turn-end 路径共用压缩本体（`accumulate_distill`：再次提炼只留最新
/// refine 消息，防累加撑爆小窗口）与归档（`save_refine_entry`）。
/// 返回 false = 摘要为空（调用方应回滚 session）。
pub fn commit_in_loop_refine(session: &mut Session, summary: &str, agent_type: AgentType) -> bool {
    let summary = summary.trim();
    if summary.is_empty() {
        return false;
    }
    session.accumulate_distill(summary);
    let _ = save_refine_entry(
        &session.id,
        &session.current_turn_id(),
        summary,
        "in_loop",
        agent_type,
    );
    true
}

/// Refine 内部流程的过滤发射器：只放行 LlmTextDelta（提炼气泡流式渲染），
/// 静默其余事件（ExecutionStarted / ExecutionCompleted / ToolCall* 等）。
///
/// 从 src-tauri 下沉至此：turn-end 的 execute_session_refine 与 lib 层
/// refine 流程共用同一过滤器——两路径必须同构，禁止各写一套（原定义处
/// 改为 `pub use` 引用本件）。
#[derive(Clone)]
pub struct RefineStreamFilter {
    pub inner: Arc<dyn EventEmitter>,
}

impl EventEmitter for RefineStreamFilter {
    fn emit(&self, event: NuphusEvent) {
        if matches!(event, NuphusEvent::LlmTextDelta { .. }) {
            self.inner.emit(event);
        }
    }
}

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
