//! ui-maps — 屏幕布局与操作经验存储工具
//!
//! Leader 在 workflow 工作流中逐场景产出：
//! - 阶段一：探索完一个屏幕就调用 `ui_maps_save_screen` 存布局
//! - 阶段四：工作流确认后调用 `ui_maps_save_experience` 存操作链
//! - 设计新流程时：`ui_maps_search` 跨类别递归检索屏幕布局和操作经验
//!
//! 存储路径: plugin/ui-maps/{app_category}/{app_name}.json
//! 按"大类/应用"组织，同类 app（如都是 IM）可复用操作经验。
//! search 递归遍历 ui-maps 目录，同时匹配 screen（返回 region 概要）和 experience（完整 summary），
//! 结果按 app 分组返回 matched_screens + matched_experiences。
//! coordinates/params 仍在对应 workflow 的 params.json 中，ui-maps 只存布局锚点和经验。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::utils::workspace_root;
use crate::ToolResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ═══════════════════════════════════════════
// Data types
// ═══════════════════════════════════════════

#[derive(Debug, Serialize, Deserialize)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Anchor {
    #[serde(rename = "type")]
    anchor_type: String,
    value: String,
    rel_x: i32,
    rel_y: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Element {
    name: String,
    #[serde(rename = "type")]
    elem_type: String,
    rel_x: i32,
    rel_y: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Region {
    name: String,
    rect: Rect,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor: Option<Anchor>,
    #[serde(default)]
    elements: Vec<Element>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ScreenInfo {
    #[serde(default)]
    regions: Vec<Region>,
    #[serde(default)]
    anchors: Vec<Anchor>,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Experience {
    id: String,
    name: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    tool_chain: Vec<String>,
    summary: String,
    screen_ref: String,
    workflow_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UiMapsFile {
    app_name: String,
    app_category: String,
    updated_at: String,
    #[serde(default)]
    screens: HashMap<String, ScreenInfo>,
    #[serde(default)]
    experiences: Vec<Experience>,
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

fn ui_maps_dir() -> std::path::PathBuf {
    workspace_root().join("plugin").join("ui-maps")
}

fn file_path(category: &str, app_name: &str) -> std::path::PathBuf {
    let name = app_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    ui_maps_dir().join(category).join(format!("{}.json", name))
}

fn read_file(category: &str, app_name: &str) -> Option<UiMapsFile> {
    let path = file_path(category, app_name);
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_file(data: &UiMapsFile) -> Result<(), String> {
    let path = file_path(&data.app_category, &data.app_name);
    if let Err(e) = std::fs::create_dir_all(path.parent().unwrap()) {
        return Err(format!("创建目录失败: {}", e));
    }
    let json = serde_json::to_string_pretty(data).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&path, &json).map_err(|e| format!("写入失败: {}", e))?;
    Ok(())
}

/// 递归收集 ui-maps 目录下的 JSON 文件
fn collect_json_files(
    dir: &std::path::Path,
    category_filter: &str,
    out: &mut Vec<std::path::PathBuf>,
) {
    if !dir.exists() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // If category filter is set, only recurse into matching dir
            if category_filter.is_empty()
                || path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_lowercase() == category_filter.to_lowercase())
                    .unwrap_or(false)
            {
                collect_json_files(&path, "", out); // once in category dir, no further filter
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

// ═══════════════════════════════════════════
// Tool implementations
// ═══════════════════════════════════════════

impl ToolRegistry {
    /// 注册所有 ui-maps 工具
    pub(crate) fn register_ui_maps_tools(&mut self) {
        self.register_ui_maps_save_screen();
        self.register_ui_maps_save_experience();
        self.register_ui_maps_search();
    }

    // ── ui_maps_save_screen ──

    fn register_ui_maps_save_screen(&mut self) {
        self.register(ToolDef {
            name: "ui_maps_save_screen".to_string(),
            description: "保存/更新一个屏幕的布局解析结果（regions + anchors）。Leader 在阶段一探索完一个屏幕后立即调用。同名 screen 会覆盖更新（不丢失其他屏幕数据）。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_category": {
                        "type": "string",
                        "description": "应用大类，如 im / mail / ide / browser，自由填写不限"
                    },
                    "app_name": {
                        "type": "string",
                        "description": "应用名称，如 微信 / 企业微信 / Outlook"
                    },
                    "screen_name": {
                        "type": "string",
                        "description": "屏幕/界面名称，如 chat-list / chat-window / login"
                    },
                    "regions": {
                        "type": "array",
                        "description": "该屏幕的区域列表（布局解析产出）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "区域名称" },
                                "rect": { "type": "object", "description": "{x, y, width, height} 相对窗口左上角" },
                                "description": { "type": "string", "description": "功能职责描述" },
                                "anchor": { "type": "object", "description": "可选视觉锚点 {type, value, rel_x, rel_y}" },
                                "elements": { "type": "array", "description": "可选区域内交互元素列表" }
                            }
                        }
                    },
                    "anchors": {
                        "type": "array",
                        "description": "可选该屏幕的全局锚点列表",
                        "items": { "type": "object" }
                    }
                },
                "required": ["app_category", "app_name", "screen_name", "regions"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let app_category = params.get("app_category").and_then(|v| v.as_str()).unwrap_or("");
                let app_name = params.get("app_name").and_then(|v| v.as_str()).unwrap_or("");
                let screen_name = params.get("screen_name").and_then(|v| v.as_str()).unwrap_or("");

                if app_category.is_empty() || app_name.is_empty() || screen_name.is_empty() {
                    return Ok(ToolResult::failure("app_category / app_name / screen_name 不能为空"));
                }

                let regions: Vec<Region> = params.get("regions")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter().filter_map(|r| serde_json::from_value(r.clone()).ok()).collect()
                    })
                    .unwrap_or_default();

                let anchors: Vec<Anchor> = params.get("anchors")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter().filter_map(|a| serde_json::from_value(a.clone()).ok()).collect()
                    })
                    .unwrap_or_default();

                if regions.is_empty() {
                    return Ok(ToolResult::failure("regions 不能为空"));
                }

                let now = chrono::Utc::now().to_rfc3339();

                // 读取已有文件或创建新文件
                let mut data = read_file(app_category, app_name).unwrap_or(UiMapsFile {
                    app_name: app_name.to_string(),
                    app_category: app_category.to_string(),
                    updated_at: now.clone(),
                    screens: HashMap::new(),
                    experiences: Vec::new(),
                });

                // 更新 screen
                data.screens.insert(screen_name.to_string(), ScreenInfo {
                    regions,
                    anchors,
                    updated_at: now.clone(),
                });
                data.updated_at = now;

                match write_file(&data) {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "屏幕「{}」布局已保存: plugin/ui-maps/{}/{}.json",
                        screen_name, app_category, app_name
                    ))),
                    Err(e) => Ok(ToolResult::failure(e)),
                }
            },
            depends_on: vec![],
        });
    }

    // ── ui_maps_save_experience ──

    fn register_ui_maps_save_experience(&mut self) {
        self.register(ToolDef {
            name: "ui_maps_save_experience".to_string(),
            description: "保存一个操作经验到 app 的 ui-maps。Leader 在工作流确认后提取关键操作链和判断依据。同一 exp_id 会覆盖更新。不存坐标参数（坐标在 workflow 的 params.json 中）。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "app_category": {
                        "type": "string",
                        "description": "应用大类，与 save_screen 一致"
                    },
                    "app_name": {
                        "type": "string",
                        "description": "应用名称"
                    },
                    "experience": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "操作经验唯一 ID，如 send-text-message" },
                            "name": { "type": "string", "description": "简短名称，如「发文字消息」" },
                            "keywords": {
                                "type": "array", "items": { "type": "string" },
                                "description": "搜索关键词，用于跨应用检索匹配"
                            },
                            "tool_chain": {
                                "type": "array", "items": { "type": "string" },
                                "description": "操作涉及的工具名序列，如 [desktop_window_activate, desktop_mouse, desktop_input]"
                            },
                            "summary": {
                                "type": "string",
                                "description": "操作流程描述 + 关键判断依据（可跨应用复用的方法论）"
                            },
                            "screen_ref": {
                                "type": "string",
                                "description": "引用已保存的 screen_name"
                            },
                            "workflow_id": {
                                "type": "string",
                                "description": "关联的工作流 ID"
                            }
                        },
                        "required": ["id", "name", "keywords", "summary", "screen_ref", "workflow_id"]
                    }
                },
                "required": ["app_category", "app_name", "experience"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let app_category = params.get("app_category").and_then(|v| v.as_str()).unwrap_or("");
                let app_name = params.get("app_name").and_then(|v| v.as_str()).unwrap_or("");

                if app_category.is_empty() || app_name.is_empty() {
                    return Ok(ToolResult::failure("app_category / app_name 不能为空"));
                }

                let exp_val = match params.get("experience") {
                    Some(v) if v.is_object() => v,
                    _ => return Ok(ToolResult::failure("experience 参数无效或缺失")),
                };

                let exp: Experience = match serde_json::from_value(exp_val.clone()) {
                    Ok(e) => e,
                    Err(err) => return Ok(ToolResult::failure(format!("experience 格式错误: {}", err))),
                };

                if exp.id.is_empty() || exp.name.is_empty() {
                    return Ok(ToolResult::failure("experience.id 和 experience.name 不能为空"));
                }

                let exp_name = exp.name.clone();
                let now = chrono::Utc::now().to_rfc3339();
                let mut data = read_file(app_category, app_name).unwrap_or(UiMapsFile {
                    app_name: app_name.to_string(),
                    app_category: app_category.to_string(),
                    updated_at: now.clone(),
                    screens: HashMap::new(),
                    experiences: Vec::new(),
                });

                // 按 id 替换或追加
                if let Some(pos) = data.experiences.iter().position(|e| e.id == exp.id) {
                    data.experiences[pos] = exp;
                } else {
                    data.experiences.push(exp);
                }
                data.updated_at = now;

                match write_file(&data) {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "操作经验「{}」已保存: plugin/ui-maps/{}/{}.json",
                        exp_name, app_category, app_name
                    ))),
                    Err(e) => Ok(ToolResult::failure(e)),
                }
            },
            depends_on: vec![],
        });
    }

    // ── ui_maps_search ──

    fn register_ui_maps_search(&mut self) {
        self.register(ToolDef {
            name: "ui_maps_search".to_string(),
            description: "两级搜索 ui-maps：不传 screen_name 时返回匹配 app 的骨架（screen 名列表 + experience 名列表）；传 screen_name 时返回该屏幕的完整 regions 数据和关联 experiences。递归遍历所有子目录。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索关键词（空格分词，AND 匹配，大小写不敏感）"
                    },
                    "app_category": {
                        "type": "string",
                        "description": "按应用大类过滤（可选），如 im / mail / ide"
                    },
                    "app_name": {
                        "type": "string",
                        "description": "按应用名精确匹配（可选）"
                    },
                    "screen_name": {
                        "type": "string",
                        "description": "指定屏幕名获取完整 regions 数据。不传则只返回骨架（screen 名列表）。"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 10,
                        "minimum": 1,
                        "maximum": 50,
                        "description": "最多返回 app 数量"
                    }
                },
                "required": ["query"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let app_category = params.get("app_category").and_then(|v| v.as_str()).unwrap_or("");
                let app_name_filter = params.get("app_name").and_then(|v| v.as_str()).unwrap_or("");
                let screen_name_filter = params.get("screen_name").and_then(|v| v.as_str()).unwrap_or("");
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let detail_mode = !screen_name_filter.is_empty();

                if query.trim().is_empty() {
                    return Ok(ToolResult::failure("query 不能为空"));
                }

                let query_words: Vec<String> = query.split_whitespace()
                    .map(|w| w.to_lowercase())
                    .collect();

                let base_dir = ui_maps_dir();
                if !base_dir.exists() {
                    return Ok(ToolResult::success("无匹配结果（目录不存在）"));
                }

                let mut json_files: Vec<std::path::PathBuf> = Vec::new();
                collect_json_files(&base_dir, app_category, &mut json_files);

                if json_files.is_empty() {
                    return Ok(ToolResult::success("无匹配结果"));
                }

                let mut app_results: Vec<serde_json::Value> = Vec::new();

                for path in &json_files {
                    // 如果指定了 app_name，文件名不匹配则跳过
                    if !app_name_filter.is_empty() {
                        let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        if file_stem != app_name_filter { continue; }
                    }

                    let content = match std::fs::read_to_string(path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let data: UiMapsFile = match serde_json::from_str(&content) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };

                    // App 级别匹配
                    let app_name_lower = data.app_name.to_lowercase();
                    let app_match = query_words.iter().all(|qw| app_name_lower.contains(qw.as_str()));

                    // Screen 匹配
                    let mut matched_screens: Vec<serde_json::Value> = Vec::new();
                    if detail_mode {
                        // detail_mode: 只匹配指定 screen_name，返回完整 regions（含 elements 和 anchor）
                        if let Some(screen_info) = data.screens.get(screen_name_filter) {
                            let regions_full: Vec<serde_json::Value> = screen_info.regions.iter().map(|r| {
                                let elements: Vec<serde_json::Value> = r.elements.iter().map(|el| {
                                    let mut elem = serde_json::json!({
                                        "name": el.name,
                                        "type": el.elem_type,
                                        "rel_x": el.rel_x,
                                        "rel_y": el.rel_y,
                                    });
                                    if let Some(ref act) = el.activation {
                                        elem["activation"] = serde_json::Value::String(act.clone());
                                    }
                                    elem
                                }).collect();
                                let mut region = serde_json::json!({
                                    "name": r.name,
                                    "description": r.description,
                                    "rect": { "x": r.rect.x, "y": r.rect.y, "width": r.rect.width, "height": r.rect.height },
                                    "elements": elements,
                                });
                                if let Some(ref a) = r.anchor {
                                    region["anchor"] = serde_json::json!({
                                        "type": a.anchor_type,
                                        "value": a.value,
                                        "rel_x": a.rel_x,
                                        "rel_y": a.rel_y,
                                    });
                                }
                                region
                            }).collect();
                            matched_screens.push(serde_json::json!({
                                "screen_name": screen_name_filter,
                                "regions": regions_full,
                            }));
                        }
                    } else {
                        // 骨架模式：返回匹配 screen 的 [{screen_name, region_count}]
                        for (screen_name, screen_info) in &data.screens {
                            let sn_lower = screen_name.to_lowercase();
                            if query_words.iter().all(|qw| sn_lower.contains(qw.as_str())) {
                                matched_screens.push(serde_json::json!({
                                    "screen_name": screen_name,
                                    "region_count": screen_info.regions.len(),
                                }));
                            }
                        }
                    }

                    // Experience 匹配
                    let mut matched_experiences: Vec<serde_json::Value> = Vec::new();
                    if detail_mode {
                        // detail_mode: 只匹配 screen_ref==screen_name_filter，返回完整数据
                        for exp in &data.experiences {
                            if exp.screen_ref != screen_name_filter { continue; }
                            let mut match_score = 0i32;

                            // keywords 匹配: +4 per keyword
                            for qw in &query_words {
                                if exp.keywords.iter().any(|kw| kw.to_lowercase().contains(qw.as_str())) {
                                    match_score += 4;
                                }
                            }

                            // summary 匹配: +2 per keyword
                            let summary_lower = exp.summary.to_lowercase();
                            for qw in &query_words {
                                if summary_lower.contains(qw.as_str()) {
                                    match_score += 2;
                                }
                            }

                            // name 匹配: +3
                            let exp_name_lower = exp.name.to_lowercase();
                            if query_words.iter().all(|qw| exp_name_lower.contains(qw.as_str())) {
                                match_score += 3;
                            }

                            // app 匹配: +3
                            if app_match {
                                match_score += 3;
                            }

                            // screen_ref 匹配: +2
                            match_score += 2;

                            if match_score > 0 {
                                let mut item = serde_json::json!({
                                    "id": exp.id,
                                    "name": exp.name,
                                    "keywords": exp.keywords,
                                    "tool_chain": exp.tool_chain,
                                    "summary": exp.summary,
                                    "screen_ref": exp.screen_ref,
                                    "workflow_id": exp.workflow_id,
                                });
                                item["match_score"] = serde_json::Value::Number(serde_json::Number::from(match_score));
                                matched_experiences.push(item);
                            }
                        }
                    } else {
                        // 骨架模式：返回匹配 experience 的 [{id, name, keywords}]
                        for exp in &data.experiences {
                            let mut match_score = 0i32;

                            // keywords 匹配: +4 per keyword
                            for qw in &query_words {
                                if exp.keywords.iter().any(|kw| kw.to_lowercase().contains(qw.as_str())) {
                                    match_score += 4;
                                }
                            }

                            // summary 匹配: +2 per keyword
                            let summary_lower = exp.summary.to_lowercase();
                            for qw in &query_words {
                                if summary_lower.contains(qw.as_str()) {
                                    match_score += 2;
                                }
                            }

                            // name 匹配: +3
                            let exp_name_lower = exp.name.to_lowercase();
                            if query_words.iter().all(|qw| exp_name_lower.contains(qw.as_str())) {
                                match_score += 3;
                            }

                            // app 匹配: +3
                            if app_match {
                                match_score += 3;
                            }

                            // screen_ref 匹配: +2
                            if matched_screens.iter()
                                .any(|s| s.get("screen_name").and_then(|v| v.as_str()) == Some(&exp.screen_ref))
                            {
                                match_score += 2;
                            }

                            if match_score > 0 {
                                let mut item = serde_json::json!({
                                    "id": exp.id,
                                    "name": exp.name,
                                    "keywords": exp.keywords,
                                });
                                item["match_score"] = serde_json::Value::Number(serde_json::Number::from(match_score));
                                matched_experiences.push(item);
                            }
                        }
                    }

                    // 有匹配的 screen 或 experience 才纳入结果
                    if !matched_screens.is_empty() || !matched_experiences.is_empty() {
                        app_results.push(serde_json::json!({
                            "app_name": data.app_name,
                            "app_category": data.app_category,
                            "matched_screens": matched_screens,
                            "matched_experiences": matched_experiences,
                        }));
                    }
                }

                if app_results.is_empty() {
                    return Ok(ToolResult::success("无匹配结果"));
                }

                // 按 matched_experiences 的总 match_score 降序排列 app
                app_results.sort_by(|a, b| {
                    let total_score = |v: &serde_json::Value| -> i64 {
                        v.get("matched_experiences")
                            .and_then(|exps| exps.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|e| e.get("match_score").and_then(|s| s.as_i64()))
                                    .sum::<i64>()
                            })
                            .unwrap_or(0)
                    };
                    total_score(b).cmp(&total_score(a))
                });
                app_results.truncate(limit);

                // 移除 match_score（内部排序用，不暴露给 LLM）
                for app in &mut app_results {
                    if let Some(arr) = app.get_mut("matched_experiences").and_then(|e| e.as_array_mut()) {
                        for exp in arr {
                            exp.as_object_mut().map(|obj| obj.remove("match_score"));
                        }
                    }
                }

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&app_results).unwrap_or_else(|_| "[]".to_string())
                ))
            },
            depends_on: vec![],
        });
    }
}
