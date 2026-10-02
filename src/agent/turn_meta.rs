//! 一轮执行的元数据：耗时 / token / 步数。
//!
//! ## 为什么单独成文件
//!
//! 该结构同时被三处消费：**事件下发**（`NuphusEvent`）、**历史持久化**
//! （`HistoryMessage`）、**前端展示**（消息底部 / 执行面板 / ctx 弹窗）。
//! 放在 `agent/` 下作为单一来源，避免三处各自定义漂移。
//!
//! ## 权威性
//!
//! `started_at_ms` 与完成时的 `duration_ms` **由后端给出**——前端不再自行
//! 记起点（原缺陷：刷新后从刷新时刻重新计时，因为起点存在组件 ref 里）。
//! 执行中 `duration_ms` 为 `None`，前端以 `now - started_at_ms` 实时推算。
//!
//! ## 兼容
//!
//! 全部字段带 `#[serde(default)]`：旧前端反序列化新事件时忽略未知字段，
//! 新前端读旧历史时 `meta = None` → 不渲染元数据条（与 `trace_items` 同一策略）。

use serde::{Deserialize, Serialize};

/// 零值不序列化：避免旧消费方看到一堆 `0` 字段，也减小事件体。
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// 一轮执行的元数据（耗时 / token / 步数）。
///
/// 完整地描述「这一轮花了多久、用了多少 token、走了多少步」，
/// 供消息底部、执行面板、ctx 弹窗共用同一个数据源。
///
/// ## 命名契约（缺陷根因，勿改回 snake_case）
///
/// 本结构同时进**两条对外通道**：NuphusEvent 下发（`events.rs`）与
/// HistoryMessage 持久化（`state.rs:445`，session 快照 JSON）。前端镜像
/// 类型 `frontend/src/core/types.ts` 的 `TurnMeta` 是 **camelCase**，
/// 两端之间没有任何转换层——故序列化名必须与前端一致，否则：
/// - 实时轮：`durationMs`/`toolCalls` 靠事件顶层 `total_duration_ms` /
///   `total_calls` fallback 侥幸显示，token 类字段（无 fallback）全丢
///   （实测症状：气泡有耗时/步数、token chip 不显示）；
/// - 历史轮：meta 整条读不出 → `isTurnMetaEmpty` → 元数据条完全不渲染。
///
/// 字段级 `alias` 保旧数据兼容：改名前落盘的 snake_case JSON（session
/// 快照 / 旧前端发的事件）仍可反序列化，只是不再产出该命名。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TurnMeta {
    /// 本轮起点（Unix 毫秒，**后端权威**）。
    ///
    /// 前端据此实时推算耗时（`now - started_at_ms`），刷新 / 重连后依然准确
    /// ——这正是「刷新后计时归零」缺陷的根治点。
    #[serde(skip_serializing_if = "Option::is_none", alias = "started_at_ms")]
    pub started_at_ms: Option<u64>,

    /// 本轮总耗时（毫秒）。执行中为 `None`（前端用起点实时推算），
    /// 完成时由后端给出权威值。
    #[serde(skip_serializing_if = "Option::is_none", alias = "duration_ms")]
    pub duration_ms: Option<u64>,

    /// 输入 token 累计（本轮所有 LLM 调用之和）。
    #[serde(skip_serializing_if = "is_zero_u32", alias = "input_tokens")]
    pub input_tokens: u32,

    /// 输出 token 累计。
    #[serde(skip_serializing_if = "is_zero_u32", alias = "output_tokens")]
    pub output_tokens: u32,

    /// 缓存命中 token 累计。
    #[serde(skip_serializing_if = "is_zero_u32", alias = "cache_hit_tokens")]
    pub cache_hit_tokens: u32,

    /// 迭代轮次（ReAct 循环走了几轮）。
    #[serde(skip_serializing_if = "is_zero_u32", alias = "iterations")]
    pub iterations: u32,

    /// 工具调用次数（本轮实际发生的 call 数）。
    #[serde(skip_serializing_if = "is_zero_u32", alias = "tool_calls")]
    pub tool_calls: u32,

    /// 本轮**上下文增量**（tokens）：轮次结束时的上下文占用 − 轮次开始时的占用。
    ///
    /// 为什么展示它而不是「累计消耗」：同一段上下文会在同轮多次 LLM 调用里被反复
    /// 计入 `input_tokens`，累加和随调用次数线性膨胀（130K 上下文跑 40 次 = 5.2M），
    /// 那是个对用户没有信息量的数字（缺陷实证：气泡曾显示 11.2M）。增量回答的是
    /// 「这一轮让上下文长了多少」。两个占用值都来自 API `usage`
    /// （`Session::context_occupancy`），无本地估算。
    #[serde(
        default,
        skip_serializing_if = "is_zero_u32",
        alias = "context_delta_tokens"
    )]
    pub context_delta_tokens: u32,

    /// 轮次开始时的上下文占用（内部量，用于结束时算增量；0 = 未记录）。
    #[serde(
        default,
        skip_serializing_if = "is_zero_u32",
        alias = "context_start_tokens"
    )]
    pub context_start_tokens: u32,

    /// 解码速度（output tokens / 解码秒数）。与既有 `TokenUsage.gen_tps` 同语义，
    /// 此处保留「最后一次 LLM 调用」的值作为整轮代表。
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "gen_tps")]
    pub gen_tps: Option<f64>,

    /// 首 token 延迟（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "ttft_ms")]
    pub ttft_ms: Option<f64>,
}

impl TurnMeta {
    /// 以「本轮起点」开一个元数据块；其余字段待累积。
    pub fn started(started_at_ms: u64) -> Self {
        Self {
            started_at_ms: Some(started_at_ms),
            ..Default::default()
        }
    }

    /// 累计一次 LLM 调用的 token 用量（同轮内多次调用逐次累加）。
    pub fn add_usage(&mut self, input: u32, output: u32, cache_hit: u32) {
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
        self.cache_hit_tokens = self.cache_hit_tokens.saturating_add(cache_hit);
    }

    /// 是否有值得展示的数据（全空则前端不渲染元数据条）。
    pub fn is_empty(&self) -> bool {
        self.duration_ms.is_none()
            && self.input_tokens == 0
            && self.output_tokens == 0
            && self.iterations == 0
            && self.tool_calls == 0
    }

    /// 记录「轮次开始时的上下文占用」（tokens）。结束时据此算增量。
    pub fn set_context_start(&mut self, tokens: u64) {
        self.context_start_tokens = tokens.min(u32::MAX as u64) as u32;
    }

    /// 落定为「本轮完成」：权威耗时 / 步数 / 上下文增量。
    ///
    /// 所有 `ExecutionCompleted` 发射点**必须**经此收口（此前 9 处各写一遍
    /// `duration_ms` + `tool_calls`，新增字段时必然漏改）。`context_now` 传
    /// `Session::context_occupancy()`——末轮无工具调用，会话里最后追加的内容正是
    /// 那一次调用的产出，故该值就是「轮次结束时的上下文规模」。
    pub fn finish(&mut self, total_duration_ms: u64, tool_calls: usize, context_now: u64) {
        self.duration_ms = Some(total_duration_ms);
        self.tool_calls = tool_calls as u32;
        if self.context_start_tokens > 0 {
            let now = context_now.min(u32::MAX as u64) as u32;
            self.context_delta_tokens = now.saturating_sub(self.context_start_tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_only_sets_origin() {
        let m = TurnMeta::started(1_700_000_000_000);
        assert_eq!(m.started_at_ms, Some(1_700_000_000_000));
        assert!(m.duration_ms.is_none());
        assert!(m.is_empty(), "仅有起点不算有数据可展示");
    }

    #[test]
    fn add_usage_accumulates_across_calls() {
        let mut m = TurnMeta::started(1);
        m.add_usage(100, 20, 30);
        m.add_usage(50, 10, 5);
        assert_eq!(m.input_tokens, 150);
        assert_eq!(m.output_tokens, 30);
        assert_eq!(m.cache_hit_tokens, 35);
        assert!(!m.is_empty());
    }

    #[test]
    fn serde_roundtrip_skips_zero_and_none() {
        let m = TurnMeta::started(42);
        let json = serde_json::to_value(&m).unwrap();
        // 起点保留；零值字段不下发，避免噪声
        // 键名是 camelCase（前端镜像契约，见 `serializes_camel_case_for_frontend_contract`）
        assert_eq!(json["startedAtMs"], 42);
        assert!(json.get("inputTokens").is_none());
        assert!(json.get("durationMs").is_none());

        let back: TurnMeta = serde_json::from_value(json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn old_event_without_meta_deserializes_to_default() {
        // 模拟「旧历史 / 旧事件」：完全没有 meta 字段
        let legacy: TurnMeta = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy, TurnMeta::default());
        assert!(legacy.is_empty());
    }

    #[test]
    fn finish_computes_context_delta() {
        let mut m = TurnMeta::started(1_700_000_000_000);
        // 轮次开始：上一轮结束后的上下文占用
        m.set_context_start(130_801);
        m.add_usage(130_801, 199, 129_000);
        // 末轮结束：占用 = 末次 input + output
        m.finish(94_000, 7, 131_000 + 3_400);
        assert_eq!(m.duration_ms, Some(94_000));
        assert_eq!(m.tool_calls, 7);
        // 增量 = 结束占用 − 开始占用（都是官方 usage，无估算）
        assert_eq!(m.context_delta_tokens, 131_000 + 3_400 - 130_801);
        // 累计消耗仍是原始累加值（不进界面，仅供诊断）
        assert_eq!(m.input_tokens, 130_801);
    }

    #[test]
    fn finish_without_context_start_leaves_delta_zero() {
        // 未记录起点（旧路径 / 子任务未 set）→ 增量 0，前端不渲染该项，不猜
        let mut m = TurnMeta::started(1);
        m.finish(1_000, 2, 999_999);
        assert_eq!(m.context_delta_tokens, 0);
        assert_eq!(m.tool_calls, 2);
    }

    #[test]
    fn finish_delta_never_negative() {
        // 提炼后占用归零再增长 / 异常回落 → 饱和到 0，不出现负增量
        let mut m = TurnMeta::started(1);
        m.set_context_start(500);
        m.finish(10, 1, 100);
        assert_eq!(m.context_delta_tokens, 0);
    }

    /// 序列化命名必须是 camelCase——与前端镜像类型（core/types.ts TurnMeta）
    /// 逐字对齐。曾因缺这条钉：后端 snake_case 下发、前端读 camelCase，
    /// token 类字段（无顶层 fallback）全丢 → 气泡耗时/步数在、token chip 不在；
    /// 历史轮 meta 整条读不出 → 元数据条完全不渲染。
    #[test]
    fn serializes_camel_case_for_frontend_contract() {
        let mut m = TurnMeta::started(1_700_000_000_000);
        m.add_usage(100, 20, 30);
        m.set_context_start(1_000);
        m.finish(5_000, 3, 1_500);
        let json = serde_json::to_value(&m).expect("serialize");
        let obj = json.as_object().expect("object");
        // fixture 里这些字段全部非零/Some → 必然出现在输出中，逐个钉名字：
        // 前端 TurnMetaBar / applyHistory / useEvents 直接读这些 camelCase 键。
        for key in [
            "startedAtMs",
            "durationMs",
            "inputTokens",
            "outputTokens",
            "toolCalls",
            "contextDeltaTokens",
            "contextStartTokens",
        ] {
            assert!(obj.contains_key(key), "输出缺少 {key}——前端将读空");
        }
        // 显式钉住三个对外消费字段的值
        assert_eq!(json["startedAtMs"], 1_700_000_000_000u64);
        assert_eq!(json["durationMs"], 5_000);
        assert_eq!(json["contextDeltaTokens"], 500);
        // snake_case 不得再出现在输出里（出现即前端读不出）
        for (k, _) in obj {
            assert!(
                !k.contains('_'),
                "输出含 snake_case 键 {k}——前端镜像类型是 camelCase，必然读空"
            );
        }
    }

    /// 旧持久化数据（改名前落盘的 snake_case JSON）必须仍能反序列化：
    /// session 快照 / 历史事件里存量 meta 不该因改名整体变缺省。
    #[test]
    fn deserializes_legacy_snake_case_meta() {
        let legacy = serde_json::json!({
            "started_at_ms": 1_700_000_000_000u64,
            "duration_ms": 5_000,
            "input_tokens": 100,
            "output_tokens": 20,
            "tool_calls": 3,
            "context_delta_tokens": 500,
        });
        let m: TurnMeta = serde_json::from_value(legacy).expect("legacy deserialize");
        assert_eq!(m.started_at_ms, Some(1_700_000_000_000));
        assert_eq!(m.duration_ms, Some(5_000));
        assert_eq!(m.tool_calls, 3);
        assert_eq!(m.context_delta_tokens, 500);
    }
}
