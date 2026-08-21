use crate::permissions::ToolCategory;
use crate::security::user_input;
use crate::tools::registry::{ToolCtx, ToolDef, ToolRegistry};
use crate::ToolResult;

fn request_user_input_handler(
    params: &serde_json::Value,
    ctx: &ToolCtx,
) -> Result<ToolResult, String> {
    let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let sensitive = params
        .get("sensitive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let input_type = params
        .get("input_type")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    // ── icon_confirm 专用参数 ──
    let icon_path = params.get("icon_path").and_then(|v| v.as_str());
    let default_name = params.get("default_name").and_then(|v| v.as_str());
    let default_shortcut = params.get("default_shortcut").and_then(|v| v.as_str());
    let rel_x = params
        .get("rel_x")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let rel_y = params
        .get("rel_y")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let default_note = params.get("default_note").and_then(|v| v.as_str());

    if title.is_empty() {
        return Ok(ToolResult::failure("title 不能为空"));
    }
    if prompt.is_empty() {
        return Ok(ToolResult::failure("prompt 不能为空"));
    }

    let action_id = user_input::add(
        &ctx.signals,
        title,
        prompt,
        sensitive,
        input_type,
        icon_path,
        default_name,
        default_shortcut,
        rel_x,
        rel_y,
        default_note,
    );

    Ok(ToolResult::success(format!(
        "已向用户请求输入。action_id={}。等待用户提交后继续。",
        action_id,
    )))
}

impl ToolRegistry {
    pub(crate) fn register_request_user_input(&mut self) {
        self.register(ToolDef {
            name: "request_user_input".to_string(),
            description: "向用户索要输入内容（文本、截图、坐标、颜色等）。\
text 类型用于 API Key/密码/验证码等敏感文本；screenshot/region/mouse_pos/color 类型用于视觉定位——\
WorkAgent 应先尝试 OCR 自动定位，失败后再用这些类型向用户请求视觉输入。\
icon_confirm 类型用于确认纯图标功能：显示图标预览 + 功能名称/快捷键/坐标/备注表单，一次交互获取精准结构化数据。\
用户提交后返回所输入的值（非文本类型返回 JSON 字符串）。用户输入不会出现在日志中。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "输入框标题（简短概括，如「API Key」「密码」「确认图标功能」）"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "向用户展示的提示文本，说明需要什么信息以及用途"
                    },
                    "sensitive": {
                        "type": "boolean",
                        "default": true,
                        "description": "是否敏感内容（true=密码输入框遮盖显示，false=普通文本输入框）"
                    },
                    "input_type": {
                        "type": "string",
                        "enum": ["text", "screenshot", "region", "mouse_pos", "color", "icon_confirm"],
                        "default": "text",
                        "description": "输入类型：text=文本, screenshot=截图, region=框选坐标, mouse_pos=鼠标坐标, color=取色, icon_confirm=图标确认(复合表单)"
                    },
                    "icon_path": {
                        "type": "string",
                        "description": "(icon_confirm) 图标截图路径，前端显示预览"
                    },
                    "default_name": {
                        "type": "string",
                        "description": "(icon_confirm) 预填功能名称（OCR 推断结果）"
                    },
                    "default_shortcut": {
                        "type": "string",
                        "description": "(icon_confirm) 预填快捷键"
                    },
                    "rel_x": {
                        "type": "integer",
                        "description": "(icon_confirm) 预填相对 X 坐标"
                    },
                    "rel_y": {
                        "type": "integer",
                        "description": "(icon_confirm) 预填相对 Y 坐标"
                    },
                    "default_note": {
                        "type": "string",
                        "description": "(icon_confirm) 预装备注"
                    }
                },
                "required": ["title", "prompt"]
            }),
            category: ToolCategory::Core,
            executor: request_user_input_handler,
            depends_on: vec![],
        });
    }
}
