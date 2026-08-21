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

/// tenet_add 工具处理函数
///
/// 1. 校验 title/content 非空
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
            description: "Propose a user teaching tenet for approval. ⚠️ Tenets are injected into the L1 system prompt every session — use with extreme restraint. Before proposing, verify all three: (1) Is this a general behavioral principle the user explicitly stated? (2) Will it guide long-term decisions? (3) Is it NOT already in the Constitution? Reject temporary preferences, one-time instructions, and Constitution-covered rules. Write tenet content as a prompt-ready directive statement.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "教导标题（简短概括）"
                    },
                    "content": {
                        "type": "string",
                        "description": "教导内容（用户原话或你的归纳）"
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
