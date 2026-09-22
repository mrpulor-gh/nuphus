//! tenet_add — 用户教导保存提议工具
//!
//! Leader 在对话中识别到用户明确表达了偏好、约束或工作习惯时，
//! 调用此工具向用户提议保存为教导原则。需用户审批后才真正写入 TenetStore。
//!
//! 数据流:
//!    Leader 调 tenet_add → 存 PendingApprovalStore → 前端弹窗
//!    → 用户批准 → approve_pending → TenetStore::add()
//!    → 用户拒绝 → reject_pending → 丢弃

use crate::permissions::ToolCategory;
use crate::security::approval;
use crate::tools::registry::{ToolCtx, ToolDef, ToolRegistry};
use crate::ToolResult;

/// 教导内容字数上限（字符数，非字节）。
///
/// 教导原则会进入每一轮的系统提示词，篇幅直接稀释模型注意力：写得越长，
/// 关键约束越容易被忽略。因此上限写在**工具描述**（让模型生成时就收敛）与
/// **handler 校验**（防止绕过描述超写）两处，口径必须一致。
pub const TENET_CONTENT_MAX_CHARS: usize = 200;

/// 教导标题字数上限。
pub const TENET_TITLE_MAX_CHARS: usize = 20;

/// tenet_add 工具处理函数
///
/// 1. 校验 title/content 非空且不超字数上限
/// 2. 生成 action_id 存入 PendingApprovalStore
/// 3. 返回 success(action_id)，前端检测后弹出审批弹窗
fn tenet_add_handler(params: &serde_json::Value, ctx: &ToolCtx) -> Result<ToolResult, String> {
    let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let priority = params
        .get("priority")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");

    if title.is_empty() {
        return Ok(ToolResult::failure("title 不能为空"));
    }
    if content.is_empty() {
        return Ok(ToolResult::failure("content 不能为空"));
    }
    // 字数口径：按字符数（中文一字算一个），与工具描述里的「≤200 字」一致
    let title_chars = title.chars().count();
    if title_chars > TENET_TITLE_MAX_CHARS {
        return Ok(ToolResult::failure(format!(
            "title 超长：{title_chars} 字，上限 {TENET_TITLE_MAX_CHARS} 字。请压缩为简短概括。"
        )));
    }
    let content_chars = content.chars().count();
    if content_chars > TENET_CONTENT_MAX_CHARS {
        return Ok(ToolResult::failure(format!(
            "content 超长：{content_chars} 字，上限 {TENET_CONTENT_MAX_CHARS} 字。\
             教导原则会进入每轮系统提示词，请只保留可执行的规则陈述，删除理由、背景与示例。"
        )));
    }

    let metadata = serde_json::json!({
        "priority": priority,
    });

    let action_id = approval::add(&ctx.signals, "tenet", title, content, metadata);

    Ok(ToolResult::success(format!(
        "已提交用户审批。action_id={}。等待用户确认后保存为教导原则。",
        action_id,
    )))
}

// ── 注册到 ToolRegistry ──

impl ToolRegistry {
    pub(crate) fn register_tenet_add(&mut self) {
        self.register(ToolDef {
            name: "tenet_add".to_string(),
            description: "Propose a user teaching tenet for approval. Tenets enter the system prompt every session, so keep them short: content ≤200 chars — state the rule, drop rationale and examples. Uses: user explicitly stated a general behavioral principle that will guide long-term decisions. Do NOT use for: temporary preferences, one-time instructions, or rules already covered by the Constitution / L0 prompt.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "教导标题（≤20 字）"
                    },
                    "content": {
                        "type": "string",
                        "maxLength": 200,
                        "description": "教导内容。≤200 字，写成可执行的规则陈述；不写理由、背景、示例"
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["low", "medium", "high", "critical"],
                        "default": "medium",
                        "description": "优先级"
                    }
                },
                "required": ["title", "content"]
            }),
            category: ToolCategory::Core,
            executor: tenet_add_handler,
            depends_on: vec![],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    /// 构造纯参数校验用的 ToolCtx（本用例只走「非空 + 字数」校验，不落盘）。
    fn ctx() -> ToolCtx {
        ToolCtx {
            signals: Arc::new(RwLock::new(crate::state::SignalState::default())),
            schedule_tool: None,
        }
    }

    /// 边界：恰好等于上限必须放行（避免把「≤N」实现成「<N」）。
    #[test]
    fn content_at_limit_passes_validation() {
        let content = "原".repeat(TENET_CONTENT_MAX_CHARS);
        let params = serde_json::json!({ "title": "标题", "content": content });
        let r = tenet_add_handler(&params, &ctx()).expect("handler 不应报错");
        assert!(r.success, "达到上限的内容应通过长度校验");
    }

    /// 超一字也不放行：这是「禁止长文污染系统提示词」的执行点。
    #[test]
    fn content_over_limit_is_rejected() {
        let content = "原".repeat(TENET_CONTENT_MAX_CHARS + 1);
        let params = serde_json::json!({ "title": "标题", "content": content });
        let r = tenet_add_handler(&params, &ctx()).expect("handler 不应 panic");
        assert!(!r.success, "超长 content 必须被拒绝");
        let err = r.error.unwrap_or_default();
        assert!(
            err.contains("上限"),
            "拒绝信息应说明上限，便于模型自我修正：{err}"
        );
    }

    /// 字数按**字符**算，不按字节：中文 200 字是 600 字节，若按字节判定会被误拒。
    #[test]
    fn length_is_counted_in_chars_not_bytes() {
        let content = "原".repeat(TENET_CONTENT_MAX_CHARS);
        assert_eq!(content.len(), 600, "前提：该内容 600 字节");
        let params = serde_json::json!({ "title": "标题", "content": content });
        let r = tenet_add_handler(&params, &ctx()).expect("handler 不应 panic");
        assert!(r.success, "按字符数计算时应放行，不能按字节误拒");
    }

    #[test]
    fn empty_title_or_content_is_rejected() {
        let r = tenet_add_handler(&serde_json::json!({ "title": "", "content": "x" }), &ctx())
            .expect("handler 不应 panic");
        assert!(!r.success, "空 title 必须被拒绝");

        let r = tenet_add_handler(&serde_json::json!({ "title": "t", "content": "" }), &ctx())
            .expect("handler 不应 panic");
        assert!(!r.success, "空 content 必须被拒绝");
    }

    /// 标题超限同样拦截。
    #[test]
    fn title_over_limit_is_rejected() {
        let title = "标".repeat(TENET_TITLE_MAX_CHARS + 1);
        let params = serde_json::json!({ "title": title, "content": "rule" });
        let r = tenet_add_handler(&params, &ctx()).expect("handler 不应 panic");
        assert!(!r.success, "超长 title 必须被拒绝");
    }
}
