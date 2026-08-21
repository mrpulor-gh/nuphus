//! 关系标注工具定义
//!
//! Leader 通过这几个工具管理关系关键词映射表（替代硬编码）。
//! 数据存储在 .nuphus/annotations.json。

use crate::annotation::store::AnnotationStore;
use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

impl ToolRegistry {
    /// 注册 annotation_add 工具
    pub(crate) fn register_annotation_add(&mut self) {
        self.register(ToolDef {
            name: "annotation_add".to_string(),
            description: "Add keyword-triggered annotation. ⚠️ Auto-injected into prompt on keyword match — use with extreme restraint. Before adding, verify all three: (1) Will the user actually say this keyword? (2) Will this info change the Agent's decision right now? (3) Isn't this already in the Agent's Constitution? If any answer is No, skip. Max 400 chars, reserve for truly reusable decision-critical knowledge.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "keyword": {"type": "string", "maxLength": 50, "description": "Trigger keyword (primary, case-insensitive substring match)"},
                    "keywords": {"type": "array", "items": {"type": "string", "maxLength": 50}, "description": "Additional trigger keywords (optional)"},
                    "description": {"type": "string", "maxLength": 400, "description": "Annotation description (≤400 chars)"},
                    "paths": {"type": "array", "items": {"type": "string"}, "description": "Related file paths (optional)"},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Classification tags for filtering (optional)"},
                    "group": {"type": "string", "description": "Group: system/user/custom (optional, default: custom)"},
                    "priority": {"type": "integer", "description": "Sort priority, higher = earlier match (optional, default: 0)"},
                    "memory_entry_id": {"type": "string", "description": "Linked memory entry ID for cross-reference (optional)"}
                },
                "required": ["keyword", "description"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let keyword: String = params.get("keyword").and_then(|v| v.as_str()).unwrap_or("").chars().take(50).collect();
                let keywords: Vec<String> = params.get("keywords")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let description: String = params.get("description").and_then(|v| v.as_str()).unwrap_or("").chars().take(400).collect();
                let paths: Vec<String> = params.get("paths")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let tags: Vec<String> = params.get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let group = params.get("group").and_then(|v| v.as_str().map(String::from));
                let priority = params.get("priority").and_then(|v| v.as_i64().map(|n| n as i32));
                let memory_entry_id = params.get("memory_entry_id").and_then(|v| v.as_str().map(String::from));
                match AnnotationStore::add(&keyword, &description, keywords, paths, tags, group, priority, memory_entry_id) {
                    Ok(ann) => Ok(ToolResult::success(format!("Annotation '{}' saved (id={})", keyword, ann.id))),
                    Err(e) => Ok(ToolResult::failure(e)),
                }
            },
            depends_on: vec![],
        });
    }

    /// 注册 annotation_remove 工具
    pub(crate) fn register_annotation_remove(&mut self) {
        self.register(ToolDef {
            name: "annotation_remove".to_string(),
            description: "Remove an annotation by keyword".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "keyword": {"type": "string", "description": "Keyword to remove"}
                },
                "required": ["keyword"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let keyword = params.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
                match AnnotationStore::remove(keyword) {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Annotation '{}' removed",
                        keyword
                    ))),
                    Err(e) => Ok(ToolResult::failure(e)),
                }
            },
            depends_on: vec![],
        });
    }

    /// 注册 annotation_search 工具
    pub(crate) fn register_annotation_search(&mut self) {
        self.register(ToolDef {
            name: "annotation_search".to_string(),
            description: "Search annotations by keyword (case-insensitive match on keyword field)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "keyword": { "type": "string", "description": "Keyword to search in annotation keywords" }
                },
                "required": ["keyword"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let keyword = params.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
                if keyword.is_empty() {
                    return Ok(ToolResult::failure("keyword is required"));
                }
                let list = AnnotationStore::search(keyword);
                let output = if list.is_empty() {
                    format!("No annotations matching '{}'.", keyword)
                } else {
                    let mut s = format!("## Annotations matching '{}' (total: {}, builtin: {})\n\n", keyword, list.len(), list.iter().filter(|a| a.builtin).count());
                    for a in &list {
                        let builtin_tag = if a.builtin { " [builtin]" } else { "" };
                        s.push_str(&format!("- **{}**{}: {}\n", a.keyword, builtin_tag, a.description));
                        if !a.paths.is_empty() {
                            s.push_str(&format!("  - paths: {}\n", a.paths.join(", ")));
                        }
                        if !a.tags.is_empty() {
                            s.push_str(&format!("  - tags: {}\n", a.tags.join(", ")));
                        }
                        s.push_str(&format!("  - group: {} | priority: {}\n", a.group, a.priority));
                        if let Some(meid) = &a.memory_entry_id {
                            s.push_str(&format!("  - memory_entry: {}\n", meid));
                            // Try to fetch linked entry summary
                            if let Ok(Some(entry)) = crate::store::memory::get_entry_by_id(meid) {
                                s.push_str(&format!("    → {}\n", entry.summary.chars().take(80).collect::<String>()));
                            }
                        }
                    }
                    s
                };
                Ok(ToolResult::success(output))
            },
            depends_on: vec![],
        });
    }
}
