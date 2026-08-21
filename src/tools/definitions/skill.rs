//! 技能工具定义
//! skill_query / skill_read

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

impl ToolRegistry {
    /// skill_query — 在已安装 skill 中查询知识（不带 query 则列出全部已安装技能）
    pub(crate) fn register_skill_query(&mut self) {
        self.register(ToolDef {
            name: "skill_query".to_string(),
            description: "Query installed skill knowledge bases. Call without query to list all installed skills.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "查询关键词或问题描述。留空则返回全部已安装技能列表" },
                    "skill": { "type": "string", "description": "指定技能包名称（可选），不指定则搜索所有已安装技能" }
                },
                "required": []
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let skill = params.get("skill").and_then(|v| v.as_str());

                let reg = crate::skill::SkillRegistry::new();

                // 空查询 → 返回全部已安装技能列表（实时扫描，不依赖 prompt 缓存）
                if query.is_empty() {
                    let skills = reg.list();
                    if skills.is_empty() {
                        return Ok(ToolResult::success("暂无已安装技能。可通过 skill_read 了解如何安装。".to_string()));
                    }
                    let lines: Vec<String> = skills.iter().map(|s| {
                        format!("- `{}` — {} (builtin: {})", s.name, s.version, s.builtin)
                    }).collect();
                    return Ok(ToolResult::success(format!("已安装技能 ({} 个):\n{}", skills.len(), lines.join("\n"))));
                }

                let input = crate::skill::types::SkillQueryInput {
                    query: query.to_string(),
                    skill: skill.map(|s| s.to_string()),
                    domain: None,
                };
                let output = reg.query(&input);
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| e.to_string())?;
                Ok(ToolResult::success(json))
            },
            depends_on: vec![],
        });
    }

    /// skill_read — 读取指定 skill 的 SKILL.md 全文
    pub(crate) fn register_skill_read(&mut self) {
        self.register(ToolDef {
            name: "skill_read".to_string(),
            description: "Read full SKILL.md for a skill package".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "技能名称" }
                },
                "required": ["skill"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let skill = params.get("skill").and_then(|v| v.as_str()).unwrap_or("");
                let reg = crate::skill::SkillRegistry::new();
                match reg.get_skill_md(skill) {
                    Some(md) => Ok(ToolResult::success(md)),
                    None => Ok(ToolResult::failure(format!("Skill '{}' not found", skill))),
                }
            },
            depends_on: vec![],
        });
    }
}
