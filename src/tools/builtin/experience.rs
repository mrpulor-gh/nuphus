//! experience — 通用操作经验存储与查询工具
//!
//! 存储**跨应用可复用的通用操作经验和方法论**，而非某个具体应用的参数。
//!
//! 用途：
//! - `experience_save` — 保存通用操作技巧、布局解析模式、排查经验
//! - `experience_query` — 关键词检索经验库（keywords + content全文）
//! - `experience_list` — 按 domain/category 列举经验条目
//! - `experience_delete` — 删除指定经验条目
//!
//! 存储路径: plugin/experiences/{domain}/{category}/{name}.json
//!
//! 典型组织方式：
//!   plugin/experiences/desktop/technique/layout-parsing.json    — 布局解析通用方法论
//!   plugin/experiences/desktop/technique/tab-bar-detection.json — Tab bar 识别技巧
//!   plugin/experiences/desktop/technique/hover-exploration.json — Hover 探索元素的通用方法
//!   plugin/experiences/desktop/troubleshooting/ocr-ghost-text.json — OCR 幽灵文本排查
//!   plugin/experiences/desktop/pattern/login-flow.json         — 桌面登录流程通用模式
//!   plugin/experiences/browser/technique/spa-state-cleanup.json — SPA 状态残留清理
//!
//! 设计原则：存「法」不存「案」，存通用方法论不存具体应用实例。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::utils::workspace_root;
use crate::ToolResult;
use serde::{Deserialize, Serialize};

/// 经验条目 JSON 结构
#[derive(Debug, Serialize, Deserialize)]
struct ExperienceEntry {
    name: String,
    domain: String,
    category: String,
    keywords: Vec<String>,
    /// 自由标签（用于更灵活的分类/过滤）
    #[serde(default)]
    tags: Vec<String>,
    content: String,
    created: String,
    updated: String,
}

/// 经验存储根目录: {project_root}/plugin/experiences
fn experiences_dir() -> std::path::PathBuf {
    workspace_root().join("plugin").join("experiences")
}

/// 清理文件名中的非法字符（路径分隔符）
fn sanitize_name(name: &str) -> String {
    name.replace(['/', '\\'], "_")
}

impl ToolRegistry {
    /// 注册所有经验工具
    pub(crate) fn register_experience_tools(&mut self) {
        self.register_experience_save();
        self.register_experience_query();
        self.register_experience_list();
        self.register_experience_delete();
    }

    // ── experience_save ──

    fn register_experience_save(&mut self) {
        self.register(ToolDef {
            name: "experience_save".to_string(),
            description: "保存通用操作经验到 JSON 文件。存「法」不存「案」——只保存可跨应用复用的通用方法论，不保存某个具体应用的参数实例。同名存在时保留 created 时间，更新 updated 为当前时间并覆盖 content/keywords/tags。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "domain": {
                        "type": "string",
                        "description": "经验领域，如 desktop / browser / mobile / terminal，自由填写不限"
                    },
                    "category": {
                        "type": "string",
                        "description": "经验类别，如 technique（技巧） / pattern（模式） / troubleshooting（排查） / workflow（流程编排），自由填写不限"
                    },
                    "name": {
                        "type": "string",
                        "description": "唯一标识名称（不含扩展名，路径分隔符会被替换为下划线）"
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "搜索关键词数组（用于 experience_query AND 匹配）"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "可选自由标签，用于 experience_list 按标签过滤"
                    },
                    "content": {
                        "type": "string",
                        "description": "经验内容（Markdown 或纯文本，建议结构化描述）"
                    }
                },
                "required": ["domain", "category", "name", "keywords", "content"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let domain = params.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let category = params.get("category").and_then(|v| v.as_str()).unwrap_or("");
                let raw_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let keywords: Vec<String> = params.get("keywords")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let tags: Vec<String> = params.get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if domain.trim().is_empty() || category.trim().is_empty() || raw_name.trim().is_empty() {
                    return Ok(ToolResult::failure("domain / category / name 不能为空"));
                }
                if keywords.is_empty() {
                    return Ok(ToolResult::failure("keywords 不能为空"));
                }
                if content.trim().is_empty() {
                    return Ok(ToolResult::failure("content 不能为空"));
                }

                let name = sanitize_name(raw_name);

                let dir = experiences_dir().join(domain).join(category);
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    return Ok(ToolResult::failure(format!("创建目录失败: {}", e)));
                }

                let file_path = dir.join(format!("{}.json", name));
                let now = chrono::Utc::now().to_rfc3339();

                // 同名已存在 → 保留 created，更新 updated
                let created = if file_path.exists() {
                    std::fs::read_to_string(&file_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<ExperienceEntry>(&s).ok())
                        .map(|old| old.created)
                        .unwrap_or_else(|| now.clone())
                } else {
                    now.clone()
                };

                let entry = ExperienceEntry {
                    name: name.clone(),
                    domain: domain.to_string(),
                    category: category.to_string(),
                    keywords,
                    tags,
                    content: content.to_string(),
                    created,
                    updated: now,
                };

                let json = match serde_json::to_string_pretty(&entry) {
                    Ok(j) => j,
                    Err(e) => return Ok(ToolResult::failure(format!("JSON 序列化失败: {}", e))),
                };

                match std::fs::write(&file_path, &json) {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "经验已保存: plugin/experiences/{}/{}/{}.json",
                        domain, category, name
                    ))),
                    Err(e) => Ok(ToolResult::failure(format!("写入文件失败: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }

    // ── experience_query ──

    fn register_experience_query(&mut self) {
        self.register(ToolDef {
            name: "experience_query".to_string(),
            description: "检索经验库。按 domain + category（可选）+ 关键词匹配搜索经验文件。关键词在 keywords 和 content 字段中全文检索（AND 匹配，大小写不敏感）。返回 name/keywords/tags/content 摘要。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "domain": {
                        "type": "string",
                        "description": "经验领域过滤，如 desktop / browser / mobile（可选，不传则搜索所有领域）"
                    },
                    "category": {
                        "type": "string",
                        "description": "经验类别过滤（可选，不传则搜索该 domain 下所有类别）"
                    },
                    "query": {
                        "type": "string",
                        "description": "空格分词的关键词，AND 匹配（所有词都必须命中 keywords 或 content，大小写不敏感）"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 5,
                        "minimum": 1,
                        "maximum": 50,
                        "description": "最多返回条数"
                    }
                },
                "required": ["query"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let domain = params.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let category = params.get("category").and_then(|v| v.as_str()).unwrap_or("");
                let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

                if query.trim().is_empty() {
                    return Ok(ToolResult::failure("query 不能为空"));
                }

                // 空格分词，统一小写
                let query_words: Vec<String> = query
                    .split_whitespace()
                    .map(|w| w.to_lowercase())
                    .collect();

                let base_dir = if domain.trim().is_empty() {
                    experiences_dir()
                } else {
                    experiences_dir().join(domain)
                };

                // 收集需要扫描的子目录
                let mut scan_dirs: Vec<std::path::PathBuf> = Vec::new();
                if category.trim().is_empty() {
                    // 扫描所有子目录
                    if base_dir.exists() {
                        if let Ok(entries) = std::fs::read_dir(&base_dir) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_dir() {
                                    scan_dirs.push(p);
                                }
                            }
                        }
                    }
                    // 如果没有子目录（domain 可能是空的顶级），就把 base_dir 本身当作扫描目标
                    if scan_dirs.is_empty() && base_dir.exists() {
                        scan_dirs.push(base_dir.clone());
                    }
                } else {
                    let target = base_dir.join(category);
                    if target.exists() {
                        scan_dirs.push(target);
                    }
                }

                // 遍历目录，收集匹配条目
                let mut results: Vec<(ExperienceEntry, usize)> = Vec::new();

                for dir in &scan_dirs {
                    if !dir.exists() {
                        continue;
                    }
                    let rd = match std::fs::read_dir(dir) {
                        Ok(rd) => rd,
                        Err(_) => continue,
                    };
                    for entry in rd.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("json") {
                            continue;
                        }
                        let file_content = match std::fs::read_to_string(&path) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let exp: ExperienceEntry = match serde_json::from_str(&file_content) {
                            Ok(e) => e,
                            Err(_) => continue,
                        };

                        // AND 匹配：每个 query 词必须命中 keywords 或 content（大小写不敏感）
                        let mut matched_query_count = 0usize;
                        for qw in &query_words {
                            let hit_in_keywords = exp.keywords.iter().any(|kw| {
                                kw.to_lowercase().contains(qw.as_str())
                            });
                            let hit_in_content = exp.content.to_lowercase().contains(qw.as_str());
                            let hit_in_tags = exp.tags.iter().any(|t| {
                                t.to_lowercase().contains(qw.as_str())
                            });
                            if hit_in_keywords || hit_in_content || hit_in_tags {
                                matched_query_count += 1;
                            }
                        }

                        if matched_query_count == query_words.len() {
                            // 排序权重：keywords 命中数 × 3 + tags 命中数 × 2 + content 命中数
                            let kw_hits = exp.keywords.iter().filter(|kw| {
                                let kw_lower = kw.to_lowercase();
                                query_words.iter().any(|qw| kw_lower.contains(qw.as_str()))
                            }).count();
                            let tag_hits = exp.tags.iter().filter(|t| {
                                let t_lower = t.to_lowercase();
                                query_words.iter().any(|qw| t_lower.contains(qw.as_str()))
                            }).count();
                            let score = kw_hits * 3 + tag_hits * 2;
                            results.push((exp, score));
                        }
                    }
                }

                if results.is_empty() {
                    return Ok(ToolResult::success("无匹配经验"));
                }

                // 按匹配分降序
                results.sort_by_key(|a| std::cmp::Reverse(a.1));
                results.truncate(limit);

                // 构建输出
                let output: Vec<serde_json::Value> = results.into_iter().map(|(exp, _)| {
                    let char_count = exp.content.chars().count();
                    let summary: String = exp.content.chars().take(300).collect();
                    let display = if char_count > 300 {
                        format!("{}...", summary)
                    } else {
                        summary
                    };
                    serde_json::json!({
                        "name": exp.name,
                        "domain": exp.domain,
                        "category": exp.category,
                        "keywords": exp.keywords,
                        "tags": exp.tags,
                        "summary": display,
                    })
                }).collect();

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "[]".to_string())
                ))
            },
            depends_on: vec![],
        });
    }

    // ── experience_list ──

    fn register_experience_list(&mut self) {
        self.register(ToolDef {
            name: "experience_list".to_string(),
            description:
                "列举经验库条目。可按 domain/category/tag 过滤查看已存储的所有经验条目名称和摘要。"
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "domain": {
                        "type": "string",
                        "description": "领域过滤（可选）"
                    },
                    "category": {
                        "type": "string",
                        "description": "类别过滤，仅当指定 domain 时有效（可选）"
                    },
                    "tag": {
                        "type": "string",
                        "description": "按标签过滤（可选，大小写不敏感子串匹配）"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 20,
                        "minimum": 1,
                        "maximum": 100,
                        "description": "最多返回条数"
                    }
                }
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let domain = params.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let category = params
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tag_filter = params.get("tag").and_then(|v| v.as_str()).unwrap_or("");
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

                let base_dir = if domain.trim().is_empty() {
                    experiences_dir()
                } else {
                    experiences_dir().join(domain)
                };

                // 收集需要扫描的子目录
                let mut scan_dirs: Vec<std::path::PathBuf> = Vec::new();
                if category.trim().is_empty() {
                    if base_dir.exists() {
                        if let Ok(entries) = std::fs::read_dir(&base_dir) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p.is_dir() {
                                    scan_dirs.push(p);
                                }
                            }
                        }
                    }
                    if scan_dirs.is_empty() && base_dir.exists() {
                        scan_dirs.push(base_dir.clone());
                    }
                } else {
                    let target = base_dir.join(category);
                    if target.exists() {
                        scan_dirs.push(target);
                    }
                }

                let mut results: Vec<serde_json::Value> = Vec::new();

                for dir in &scan_dirs {
                    if !dir.exists() {
                        continue;
                    }
                    let rd = match std::fs::read_dir(dir) {
                        Ok(rd) => rd,
                        Err(_) => continue,
                    };
                    for entry in rd.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("json") {
                            continue;
                        }
                        let file_content = match std::fs::read_to_string(&path) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let exp: ExperienceEntry = match serde_json::from_str(&file_content) {
                            Ok(e) => e,
                            Err(_) => continue,
                        };

                        // Tag 过滤
                        if !tag_filter.trim().is_empty() {
                            let tf = tag_filter.to_lowercase();
                            let tag_match = exp.tags.iter().any(|t| t.to_lowercase().contains(&tf));
                            if !tag_match {
                                continue;
                            }
                        }

                        let content_preview: String = exp.content.chars().take(100).collect();
                        results.push(serde_json::json!({
                            "name": exp.name,
                            "domain": exp.domain,
                            "category": exp.category,
                            "keywords": exp.keywords,
                            "tags": exp.tags,
                            "preview": content_preview,
                            "updated": exp.updated,
                        }));

                        if results.len() >= limit {
                            break;
                        }
                    }
                    if results.len() >= limit {
                        break;
                    }
                }

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string()),
                ))
            },
            depends_on: vec![],
        });
    }

    // ── experience_delete ──

    fn register_experience_delete(&mut self) {
        self.register(ToolDef {
            name: "experience_delete".to_string(),
            description: "删除指定经验条目。需要 domain / category / name 精确定位。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "domain": {
                        "type": "string",
                        "description": "经验领域"
                    },
                    "category": {
                        "type": "string",
                        "description": "经验类别"
                    },
                    "name": {
                        "type": "string",
                        "description": "条目名称（不含 .json 扩展名）"
                    }
                },
                "required": ["domain", "category", "name"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let domain = params.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let category = params
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let raw_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");

                if domain.trim().is_empty()
                    || category.trim().is_empty()
                    || raw_name.trim().is_empty()
                {
                    return Ok(ToolResult::failure("domain / category / name 不能为空"));
                }

                let name = sanitize_name(raw_name);
                let file_path = experiences_dir()
                    .join(domain)
                    .join(category)
                    .join(format!("{}.json", name));

                if !file_path.exists() {
                    return Ok(ToolResult::failure(format!(
                        "经验条目未找到: plugin/experiences/{}/{}/{}.json",
                        domain, category, name
                    )));
                }

                match std::fs::remove_file(&file_path) {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "经验已删除: plugin/experiences/{}/{}/{}.json",
                        domain, category, name
                    ))),
                    Err(e) => Ok(ToolResult::failure(format!("删除失败: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }
}
