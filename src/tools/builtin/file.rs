//! file — 文件/目录浏览工具(衔接 search_grep / file_read / file_edit)
//!
//! 三件套设计目标:让"看目录 → 看文件属性 → 读 → 改"的 LLM 工具链衔接顺畅。
//!
//! 衔接原则:
//! - 所有路径字段输出 **绝对路径**,可直接喂给下一步工具
//! - 输出为 JSON,LLM 可按字段编程式继续推理
//! - 失败模式区分清晰(不存在 vs 权限 vs IO)

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

impl ToolRegistry {
    /// 列出目录内容(衔接链:list_dir → stat / read / grep)
    pub(crate) fn register_list_dir(&mut self) {
        self.register(ToolDef {
            name: "ListDir".to_string(),
            description: "List directory entries one level deep".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path" },
                    "include_hidden": { "type": "boolean", "default": false, "description": "Include hidden entries" },
                    "limit": { "type": "integer", "default": 200, "description": "Max entries to return" }
                },
                "required": ["path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let include_hidden = params.get("include_hidden").and_then(|v| v.as_bool()).unwrap_or(false);
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

                if path.trim().is_empty() {
                    return Ok(ToolResult::failure("path is required"));
                }
                if path.contains("..") {
                    return Ok(ToolResult::failure("path traversal not allowed"));
                }

                // 二进制办公格式不支持追加内容
                if crate::tools::definitions::file::is_binary_office_format(path) {
                    return Ok(ToolResult::failure(format!(
                        "{} 是二进制办公格式，Append 不支持追加内容。",
                        path
                    )));
                }

                let p = std::path::Path::new(path);
                if !p.exists() {
                    let cwd = std::env::current_dir()
                        .map(|c| c.display().to_string())
                        .unwrap_or_else(|_| "unknown".to_string());
                    return Ok(ToolResult::failure(format!(
                        "directory not found: {} (cwd: {})", path, cwd
                    )));
                }
                if !p.is_dir() {
                    return Ok(ToolResult::failure(format!("not a directory: {}", path)));
                }

                let abs_root = match std::fs::canonicalize(p) {
                    Ok(r) => r,
                    Err(_) => p.to_path_buf(),
                };

                let mut entries = Vec::new();
                let read_dir = match std::fs::read_dir(&abs_root) {
                    Ok(rd) => rd,
                    Err(e) => return Ok(ToolResult::failure(format!("read_dir failed: {}", e))),
                };

                let mut total_seen = 0usize;
                let mut total_skipped = 0usize;
                let mut truncated = false;
                for ent_result in read_dir {
                    let ent = match ent_result {
                        Ok(e) => e,
                        Err(_) => {
                            total_skipped += 1;
                            continue;
                        }
                    };

                    let name = ent.file_name().to_string_lossy().to_string();

                    // 隐藏文件过滤：Unix 风格 (.开头) + Windows 隐藏属性
                    let is_hidden = if !include_hidden {
                        if name.starts_with('.') {
                            true
                        } else {
                            #[cfg(windows)]
                            {
                                ent.metadata().ok()
                                    .map(|m| m.file_attributes() & 0x2 != 0) // FILE_ATTRIBUTE_HIDDEN
                                    .unwrap_or(false)
                            }
                            #[cfg(not(windows))]
                            {
                                false
                            }
                        }
                    } else {
                        false
                    };

                    total_seen += 1;

                    if is_hidden {
                        total_skipped += 1;
                        continue;
                    }

                    if entries.len() >= limit {
                        truncated = true;
                        continue;
                    }

                    let ent_path = ent.path();
                    let entry_type = if ent_path.is_dir() { "dir" } else { "file" };

                    let (size, mtime) = match ent.metadata() {
                        Ok(m) => {
                            let size = if entry_type == "file" { m.len() } else { 0 };
                            let mtime = m.modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| {
                                    chrono::DateTime::<chrono::Utc>::from_timestamp(d.as_secs() as i64, 0)
                                        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default();
                            (size, mtime)
                        }
                        Err(_) => (0, String::new()),
                    };

                    entries.push(serde_json::json!({
                        "name": name,
                        "path": ent_path.display().to_string(),
                        "type": entry_type,
                        "size": size,
                        "mtime": mtime,
                    }));
                }

                // 空目录时给出更明确的提示
                if entries.is_empty() && total_seen > 0 && total_skipped > 0 {
                    return Ok(ToolResult::success(format!(
                        "{{\"path\":\"{}\",\"count\":0,\"total\":{},\"skipped\":{},\"truncated\":false,\"entries\":[],\"note\":\"All entries are hidden. Set include_hidden=true to see them.\"}}",
                        abs_root.display(), total_seen, total_skipped
                    )));
                }

                let summary = serde_json::json!({
                    "path": abs_root.display().to_string(),
                    "count": entries.len(),
                    "total": total_seen,
                    "skipped": total_skipped,
                    "truncated": truncated,
                    "entries": entries,
                });

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&summary).unwrap_or_else(|_| "{}".to_string())
                ))
            },
            depends_on: vec![],
        });
    }

    /// 查询文件/目录属性(衔接链:stat → read 决定是否分页 / 判断 exists)
    pub(crate) fn register_files_info(&mut self) {
        self.register(ToolDef {
            name: "FilesInfo".to_string(),
            description: "Get file/directory metadata. Non-existent paths return {exists:false}"
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File or directory path" }
                },
                "required": ["path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.trim().is_empty() {
                    return Ok(ToolResult::failure("path is required"));
                }

                let p = std::path::Path::new(path);
                let absolute = std::fs::canonicalize(p)
                    .map(|a| a.display().to_string())
                    .unwrap_or_else(|_| p.display().to_string());
                let parent = p
                    .parent()
                    .map(|pp| pp.display().to_string())
                    .unwrap_or_default();

                if !p.exists() {
                    let result = serde_json::json!({
                        "exists": false,
                        "path": path,
                        "absolute": absolute,
                        "parent": parent,
                    });
                    return Ok(ToolResult::success(
                        serde_json::to_string_pretty(&result).unwrap_or_default(),
                    ));
                }

                let metadata = match std::fs::metadata(p) {
                    Ok(m) => m,
                    Err(e) => return Ok(ToolResult::failure(format!("stat failed: {}", e))),
                };

                let entry_type = if metadata.is_dir() { "dir" } else { "file" };
                let size = if entry_type == "file" {
                    metadata.len()
                } else {
                    0
                };
                let mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        chrono::DateTime::<chrono::Utc>::from_timestamp(d.as_secs() as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();

                let result = serde_json::json!({
                    "exists": true,
                    "type": entry_type,
                    "size": size,
                    "mtime": mtime,
                    "readonly": metadata.permissions().readonly(),
                    "path": path,
                    "absolute": absolute,
                    "parent": parent,
                });

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                ))
            },
            depends_on: vec![],
        });
    }

    /// 追加内容到文件末尾(避免 read+write 全量重写)
    pub(crate) fn register_append(&mut self) {
        self.register(ToolDef {
            name: "Append".to_string(),
            description: "Append text to end of file (creates file + parent dirs)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "content": { "type": "string", "description": "Text to append (no auto newline)" }
                },
                "required": ["path", "content"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if path.trim().is_empty() {
                    return Ok(ToolResult::failure("path is required"));
                }
                if path.contains("..") {
                    return Ok(ToolResult::failure("path traversal not allowed"));
                }

                // 二进制办公格式不支持追加内容
                if crate::tools::definitions::file::is_binary_office_format(path) {
                    return Ok(ToolResult::failure(format!(
                        "{} 是二进制办公格式，Append 不支持追加内容。",
                        path
                    )));
                }

                let p = std::path::Path::new(path);
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            return Ok(ToolResult::failure(format!("create parent dirs failed: {}", e)));
                        }
                    }
                }

                use std::io::Write;
                let mut file = match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                {
                    Ok(f) => f,
                    Err(e) => return Ok(ToolResult::failure(format!("open failed: {}", e))),
                };

                if let Err(e) = file.write_all(content.as_bytes()) {
                    return Ok(ToolResult::failure(format!("write failed: {}", e)));
                }

                let total = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                Ok(ToolResult::success(format!(
                    "Appended {} bytes to {} (total {} bytes)",
                    content.len(), path, total
                )))
            },
            depends_on: vec![],
        });
    }
}
