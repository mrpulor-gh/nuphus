//! 文件工具定义
//!
//! 包含所有文件操作相关的 ToolDef 注册方法：读写、编辑、删除、目录、搜索。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

/// 检测是否为不支持写操作的办公二进制格式
pub(crate) fn is_binary_office_format(path: &str) -> bool {
    let lower = path.to_lowercase();
    // .xlsx 有写支持（xlsx_write），允许通过
    lower.ends_with(".docx")
        || lower.ends_with(".pptx")
        || lower.ends_with(".xls")
        || lower.ends_with(".ods")
        || lower.ends_with(".odt")
        || lower.ends_with(".odp")
        || lower.ends_with(".pdf")
}

impl ToolRegistry {
    fn backup_file(path: &str) -> Result<String, String> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let backup_dir = std::path::Path::new(".nuphus/backup");
        std::fs::create_dir_all(backup_dir)
            .map_err(|e| format!("create backup dir failed: {}", e))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let backup_path = backup_dir.join(format!("{}_{}", file_name, now));

        std::fs::copy(path, &backup_path).map_err(|e| format!("backup failed: {}", e))?;

        Ok(backup_path.to_string_lossy().to_string())
    }
    /// 安全计算 Read 的行范围：防 i64::MIN panic、防越界
    /// 返回 (start, end)，均为 0-based，start <= end
    fn compute_read_range(offset_param: i64, limit: usize, total_lines: usize) -> (usize, usize) {
        let start = if offset_param < 0 {
            total_lines.saturating_sub(offset_param.unsigned_abs() as usize)
        } else {
            (offset_param as usize).saturating_sub(1)
        };
        let start = start.min(total_lines);
        let end = (start + limit).min(total_lines);
        // 防御：即使逻辑上 start <= end 恒成立，JSON 参数可能异常
        let end = end.max(start);
        (start, end)
    }
    pub(crate) fn register_read_file(&mut self) {
        self.register(ToolDef {
            name: "Read".to_string(),
            description: "Read file content with line numbers".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to read" },
                    "offset": { "type": "integer", "description": "Line number to start from (1-based). Negative = from end (-1 = last line)" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "Max lines to return" }
                },
                "required": ["path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.is_empty() {
                    return Ok(ToolResult::failure("path is required".to_string()));
                }
                let offset_param = params.get("offset").and_then(|v| v.as_i64()).unwrap_or(1);
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
                let limit = limit.min(2000);

                // .xlsx 文件走 calamine 解析 → Markdown 表格
                if path.to_lowercase().ends_with(".xlsx") {
                    let markdown = crate::utils::xlsx::read_xlsx_to_markdown(path)
                        .map_err(|e| format!("xlsx read failed: {} ({})", e, path))?;

                    // 对 Markdown 输出也应用行级别的 offset/limit（按换行分割）
                    let all_lines: Vec<&str> = markdown.lines().collect();
                    let total_lines = all_lines.len();

                    let (start, end) = Self::compute_read_range(offset_param, limit, total_lines);

                    if total_lines == 0 {
                        return Ok(ToolResult::success(format!("{}\n(empty xlsx, 0 rows)", path)));
                    }
                    if start >= total_lines {
                        return Ok(ToolResult::success(format!(
                            "{} (xlsx parsed, {} lines total)\n\
                             [WARNING] offset {} is beyond file end. \
                             Use offset <= {} to read content.",
                            path, total_lines, offset_param, total_lines
                        )));
                    }

                    let max_line_num_width = end.to_string().len();
                    let selected: Vec<String> = all_lines[start..end]
                        .iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let line_num = start + i + 1;
                            format!(
                                "{:>width$} | {}",
                                line_num,
                                line,
                                width = max_line_num_width
                            )
                        })
                        .collect();
                    let text = selected.join("\n");
                    let range_note = if end < total_lines {
                        format!(" (xlsx as markdown [TRUNCATED: lines {}-{} of {}])", start + 1, end, total_lines)
                    } else {
                        format!(" (xlsx as markdown, lines {}-{} of {})", start + 1, end, total_lines)
                    };
                    let result = format!("{}{}\n{}", path, range_note, text);
                    return Ok(ToolResult::success(result));
                }

                // ── 办公文档：docx/pptx/xls/ods/odt/odp/pdf ──
                if let Some(result) = crate::utils::office::read_office(path) {
                    let markdown = result?;
                    let all_lines: Vec<&str> = markdown.lines().collect();
                    let total_lines = all_lines.len();

                    let (start, end) = Self::compute_read_range(offset_param, limit, total_lines);

                    if total_lines == 0 {
                        return Ok(ToolResult::success(format!("{}\n(empty document)", path)));
                    }
                    if start >= total_lines {
                        return Ok(ToolResult::success(format!(
                            "{} ({} lines total)\n[WARNING] offset {} is beyond file end.",
                            path, total_lines, offset_param
                        )));
                    }

                    let max_w = end.to_string().len();
                    let selected: Vec<String> = all_lines[start..end]
                        .iter().enumerate()
                        .map(|(i, line)| format!("{:>width$} | {}", start + i + 1, line, width = max_w))
                        .collect();
                    let text = selected.join("\n");
                    let range_note = if end < total_lines {
                        format!(" (office doc [TRUNCATED: lines {}-{} of {}])", start + 1, end, total_lines)
                    } else {
                        format!(" (lines {}-{} of {})", start + 1, end, total_lines)
                    };
                    return Ok(ToolResult::success(format!("{}{}\n{}", path, range_note, text)));
                }

                let content = std::fs::read_to_string(path)
                    .map_err(|e| match e.kind() {
                        std::io::ErrorKind::PermissionDenied => {
                            format!("Permission denied: {}", path)
                        }
                        std::io::ErrorKind::NotFound => {
                            let cwd = std::env::current_dir()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| "unknown".to_string());
                            format!("File not found: {} (cwd: {})", path, cwd)
                        }
                        _ => format!("read failed: {} ({})", e, path),
                    })?;

                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();

                let (start, end) = Self::compute_read_range(offset_param, limit, total_lines);

                if total_lines == 0 {
                    return Ok(ToolResult::success(format!("{}\n(empty file, 0 lines)", path)));
                }

                // offset 超出文件范围 → 返回明确提示，避免空内容误导 LLM
                if start >= total_lines {
                    return Ok(ToolResult::success(format!(
                        "{} (file exists, {} lines total)\n\
                         [WARNING] offset {} is beyond file end. \
                         Use offset <= {} to read content.",
                        path, total_lines, offset_param, total_lines
                    )));
                }

                let max_line_num_width = end.to_string().len();
                let selected: Vec<String> = lines[start..end].iter().enumerate().map(|(i, line)| {
                    let line_num = start + i + 1;
                    format!("{:>width$} | {}", line_num, line, width = max_line_num_width)
                }).collect();

                let text = selected.join("\n");
                let range_note = if end < total_lines {
                    format!(
                        " [TRUNCATED: lines {}-{} of {} shown — use offset={} to continue]",
                        start + 1, end, total_lines, end + 1
                    )
                } else {
                    format!(" (lines {}-{} of {})", start + 1, end, total_lines)
                };
                let result = format!("{}{}\n{}", path, range_note, text);

                Ok(ToolResult::success(result))
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_write_file(&mut self) {
        // 注册 write_file（主名称）
        let write_def = ToolDef {
            name: "Write".to_string(),
            description: "Create or overwrite a file. Auto-creates parent directories, auto-backs up before overwrite.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write to. Must be non-empty. Examples: 'C:/Users/YourName/Desktop/report.md', './output.txt'" },
                    "content": { "type": "string", "description": "Text content to write to the file" }
                },
                "required": ["path", "content"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");

                // 仅阻止明显的路径穿越（`../` 等），不做沙箱限制
                // 用户使用场景需要写桌面、下载目录等任何位置
                if path.contains("..") {
                    return Ok(ToolResult::failure("path traversal not allowed"));
                }
                if path.trim().is_empty() {
                    return Ok(ToolResult::failure("empty path — you must provide a file path, e.g. 'C:/Users/YourName/Desktop/report.md'"));
                }

                let p = std::path::Path::new(path);
                // 确保父目录存在
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("create parent dirs failed: {}", e))?;
                    }
                }

                // Backup existing file before overwriting
                if p.exists() {
                    if let Err(e) = Self::backup_file(path) {
                        tracing::warn!("[file] backup failed for {}: {}", path, e);
                    }
                }

                // .xlsx → 结构化写出（CSV/Markdown 表格 → xlsx）
                if path.to_lowercase().ends_with(".xlsx") {
                    crate::utils::xlsx_write::write_text_to_xlsx(path, content, "Sheet1")
                        .map_err(|e| format!("xlsx write failed: {} ({})", e, path))?;
                    return Ok(ToolResult::success(format!(
                        "Wrote {} bytes to {} (xlsx, {} lines)",
                        content.len(),
                        path,
                        content.lines().count()
                    )));
                }

                // 办公二进制格式防护（docx/pptx/xls/ods/odt/odp/pdf 不支持写）
                if is_binary_office_format(path) {
                    return Ok(ToolResult::failure(format!(
                        "{} 是二进制办公格式，Write 不支持写入。请用 Read 读取内容后，另存为 .md 或 .txt 再编辑。",
                        path
                    )));
                }

                std::fs::write(p, content)
                    .map_err(|e| format!("write failed: {}", e))?;

                Ok(ToolResult::success(format!("Wrote {} bytes to {}", content.len(), path)))
            },
            depends_on: vec![],
        };
        self.register(write_def);
    }
    /// 匹配级别名称（用于结果回执，LLM 可据此判断误伤风险）
    fn match_level_name(level: u8) -> &'static str {
        match level {
            1 => "exact",
            2 => "行尾空白",
            3 => "缩进",
            _ => "首尾空白",
        }
    }

    /// Normalize text: strip BOM, normalize CRLF→LF, strip trailing \r
    fn normalize_newlines(text: &str) -> String {
        let s = text.strip_prefix('\u{feff}').unwrap_or(text);
        // 确保没有残余 \r：先把 \r\n→\n，再删孤立的 \r
        s.replace("\r\n", "\n").replace('\r', "\n")
    }

    pub(crate) fn register_edit_file(&mut self) {
        self.register(ToolDef {
            name: "Edit".to_string(),
            description: "Line-level search-and-replace in file. Auto-backs up. Single replace uses whitespace-tolerant matching. replace_all defaults to EXACT matches only (set fuzzy:true to include whitespace-tolerant ones); use expected_count to atomically validate hit count — mismatch aborts without writing.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit" },
                    "old_string": { "type": "string", "description": "Text to find (multi-line supported)" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: only first). Exact matches only unless fuzzy=true" },
                    "fuzzy": { "type": "boolean", "description": "With replace_all: also replace whitespace-tolerant matches (leading/trailing whitespace ignored). Default false — fuzzy candidates are reported but skipped" },
                    "expected_count": { "type": "integer", "description": "Expected number of replacements. If actual hits differ, the edit aborts atomically (nothing written). Grep first to get the count" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let old_str = Self::normalize_newlines(params.get("old_string").and_then(|v| v.as_str()).unwrap_or(""));
                let new_str = Self::normalize_newlines(params.get("new_string").and_then(|v| v.as_str()).unwrap_or(""));
                let replace_all = params.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
                // replace_all 默认只认精确命中；fuzzy=true 才纳入忽略首尾空白的模糊命中。
                // 单次替换始终允许模糊匹配（定位便利，仅命中第一处，风险可控）。
                let allow_fuzzy = params.get("fuzzy").and_then(|v| v.as_bool()).unwrap_or(false);
                // 命中数契约：不符则整体失败、零写入
                let expected_count = params.get("expected_count").and_then(|v| v.as_u64());

                if old_str.is_empty() {
                    return Ok(ToolResult::failure("old_string cannot be empty"));
                }

                // .xlsx → 走结构化编辑：读为 CSV → 文本替换 → 写回 xlsx
                if path.to_lowercase().ends_with(".xlsx") {
                    if let Err(e) = Self::backup_file(path) {
                        tracing::warn!("[file] backup failed for {}: {}", path, e);
                    }
                    crate::utils::xlsx_write::edit_xlsx(path, &old_str, &new_str, replace_all)
                        .map_err(|e| format!("xlsx edit failed: {} ({})", e, path))?;
                    return Ok(ToolResult::success(format!(
                        "Edited xlsx: {} ({} → {})",
                        path,
                        if replace_all { "all" } else { "first" },
                        "replacement applied"
                    )));
                }

                // 办公二进制格式防护（Edit 不支持直接修改二进制办公文件）
                if is_binary_office_format(path) {
                    return Ok(ToolResult::failure(format!(
                        "{} 是二进制办公格式，Edit 不支持直接修改。请用 Read 读取内容后，另存为 .md 或 .txt 再编辑。",
                        path
                    )));
                }

                // 编码检测：读原始字节，验证 UTF-8 合法性
                // PowerShell 重定向 / Set-Content 默认用 ANSI/GBK，会损坏非 ASCII 字符
                let raw_bytes = std::fs::read(path)
                    .map_err(|e| format!("无法读取文件: {} ({})", path, e))?;
                let is_valid_utf8 = std::str::from_utf8(&raw_bytes).is_ok();
                if !is_valid_utf8 {
                    // 尝试检测 BOM + UTF-16 LE（PowerShell 默认输出格式）
                    let looks_like_utf16le = raw_bytes.len() >= 2
                        && raw_bytes[0] == 0xFF && raw_bytes[1] == 0xFE;
                    let hint = if looks_like_utf16le {
                        "（检测到 UTF-16 LE BOM，文件可能被 PowerShell 管道/重定向损坏。请用 UTF-8 编码重新保存文件后重试。）"
                    } else {
                        "（文件包含非 UTF-8 字节，可能被 PowerShell Set-Content 或重定向损坏。请用 UTF-8 重新保存后重试。）"
                    };
                    return Ok(ToolResult::failure(format!(
                        "编码错误: 文件 {} 不是有效的 UTF-8 编码。{}", path, hint
                    )));
                }

                let raw_content = std::fs::read_to_string(path)
                    .map_err(|e| match e.kind() {
                        std::io::ErrorKind::PermissionDenied => {
                            format!("权限不足，无法读取文件: {}", path)
                        }
                        std::io::ErrorKind::NotFound => {
                            format!("文件不存在: {}", path)
                        }
                        _ => format!("read failed: {}", e),
                    })?;

                // 标准化：去 BOM、CRLF→LF，确保匹配不受 PowerShell 换行/编码影响
                let content = Self::normalize_newlines(&raw_content);
                let needs_normalize = content != raw_content;

                // Backup before editing
                let backup_file = match Self::backup_file(path) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("[file] backup failed for {}: {}", path, e);
                        String::new()
                    }
                };

                let content_lines: Vec<&str> = content.lines().collect();
                let old_lines: Vec<&str> = old_str.lines().collect();

                if old_lines.is_empty() {
                    return Ok(ToolResult::failure("old_string cannot be empty"));
                }

                // 3-pass line-based matching
                // match_positions：确认替换的位置；fuzzy_skipped：replace_all 且未开 fuzzy 时
                // 被跳过的模糊候选（只报告不动手，防止同构代码误伤）
                let mut match_positions: Vec<(usize, u8)> = Vec::new();
                let mut fuzzy_skipped: Vec<(usize, u8)> = Vec::new();

                if content_lines.len() < old_lines.len() {
                    return Ok(ToolResult::failure(format!(
                        "old_string not found in {}\n  文件仅 {} 行，old_string 有 {} 行",
                        path, content_lines.len(), old_lines.len()
                    )));
                }

                for start in 0..=content_lines.len() - old_lines.len() {
                    let mut matched = false;
                    let mut level: u8 = 0;

                    // Pass 1: exact match
                    if old_lines.iter().enumerate().all(|(i, ol)| content_lines[start + i] == *ol) {
                        matched = true;
                        level = 1;
                    }
                    // Pass 2: ignore trailing whitespace
                    else if old_lines.iter().enumerate().all(|(i, ol)| {
                        content_lines[start + i].trim_end() == ol.trim_end()
                    }) {
                        matched = true;
                        level = 2;
                    }
                    // Pass 3: ignore leading whitespace (indentation)
                    else if old_lines.iter().enumerate().all(|(i, ol)| {
                        content_lines[start + i].trim_start() == ol.trim_start()
                    }) {
                        matched = true;
                        level = 3;
                    }
                    // Pass 4: ignore both leading and trailing whitespace
                    else if old_lines.iter().enumerate().all(|(i, ol)| {
                        content_lines[start + i].trim() == ol.trim()
                    }) {
                        matched = true;
                        level = 4;
                    }

                    if matched {
                        if replace_all && !allow_fuzzy && level > 1 {
                            fuzzy_skipped.push((start, level));
                            continue;
                        }
                        match_positions.push((start, level));
                        if !replace_all {
                            break;
                        }
                    }
                }

                // 命中数原子契约：expected_count 不符 → 整体失败，零写入。
                // 备份已在上方完成但无副作用（仅多一个冗余备份文件）。
                if let Some(expected) = expected_count {
                    let actual = match_positions.len() as u64;
                    if actual != expected {
                        let skipped_note = if fuzzy_skipped.is_empty() {
                            String::new()
                        } else {
                            format!("，另有 {} 处模糊候选被跳过（fuzzy: true 可纳入）", fuzzy_skipped.len())
                        };
                        return Ok(ToolResult::failure(format!(
                            "expected_count={} 与实际命中 {} 不符，未做任何修改{}。请先用 Grep 核对 old_string 的命中数与位置",
                            expected, actual, skipped_note
                        )));
                    }
                }

                if match_positions.is_empty() {
                    // 精确未命中但存在模糊候选 → 明确告知位置与开启方式（LLM 可据此决策）
                    let fuzzy_hint = if fuzzy_skipped.is_empty() {
                        String::new()
                    } else {
                        let locs: Vec<String> = fuzzy_skipped.iter()
                            .map(|(s, l)| format!("L{}(差异:{})", s + 1, Self::match_level_name(*l)))
                            .collect();
                        format!(
                            "\n  发现 {} 处模糊匹配候选（仅空白差异）: {} —— 确认后可加 fuzzy: true 纳入替换",
                            fuzzy_skipped.len(), locs.join(", ")
                        )
                    };
                    let hint_lines: Vec<String> = content.lines().take(5).map(|l| l.to_string()).collect();
                    let hint = if hint_lines.is_empty() {
                        " (file is empty)".to_string()
                    } else {
                        format!("\n文件开头内容:\n{}", hint_lines.join("\n"))
                    };
                    return Ok(ToolResult::failure(format!(
                        "old_string not found in {}{}{}\n  提供的 old_string (前80字符): {}",
                        path, hint, fuzzy_hint, &old_str.chars().take(80).collect::<String>(),
                    )));
                }

                // Apply replacements in reverse order to preserve positions
                let new_lines: Vec<&str> = new_str.lines().collect();
                let mut result_lines: Vec<&str> = content_lines.clone();

                for &(start, _) in match_positions.iter().rev() {
                    let end = start + old_lines.len();
                    result_lines.splice(start..end, new_lines.iter().cloned());
                }

                let new_content = result_lines.join("\n");
                // 若原文件使用 \r\n，写回时保留原格式，避免整文件 diff 变动
                let write_content = if needs_normalize && raw_content.contains("\r\n") {
                    new_content.replace('\n', "\r\n")
                } else {
                    new_content
                };
                std::fs::write(path, &write_content)
                    .map_err(|e| format!("write failed: {}", e))?;

                // 写后验证：读回文件，检查编码完整性和替换结果
                let verify_result = match std::fs::read(path) {
                    Ok(bytes) => match std::str::from_utf8(&bytes) {
                        Ok(verified) => {
                            let check_token = new_str.lines().next().unwrap_or(&new_str).trim();
                            if !check_token.is_empty() && !verified.contains(check_token) {
                                Err(format!("写入后未找到替换内容 \"{}\"，文件可能损坏。", &check_token.chars().take(40).collect::<String>()))
                            } else {
                                Ok(())
                            }
                        }
                        Err(_) => Err("写入后文件编码损坏（非 UTF-8）。".to_string()),
                    },
                    Err(e) => Err(format!("写入后无法读取文件 ({})。", e)),
                };

                if let Err(reason) = verify_result {
                    if !backup_file.is_empty() && std::fs::copy(&backup_file, path).is_ok() {
                        return Ok(ToolResult::failure(format!(
                            "编辑验证失败: {}\n已自动从备份 {} 恢复原文件。", reason, backup_file
                        )));
                    }
                    return Ok(ToolResult::failure(format!(
                        "编辑验证失败: {}\n自动恢复也失败，请手动检查。备份: {}", reason, backup_file
                    )));
                }

                let match_count = match_positions.len();
                // 全量替换点回执：每处行号 + 匹配级别（非 exact 的标注差异类型）
                let positions: Vec<String> = match_positions.iter()
                    .map(|(s, l)| if *l > 1 {
                        format!("L{}({})", s + 1, Self::match_level_name(*l))
                    } else {
                        format!("L{}", s + 1)
                    })
                    .collect();
                // 被跳过的模糊候选：报告位置与开启方式，LLM 可据此决定是否追换
                let skipped_note = if fuzzy_skipped.is_empty() {
                    String::new()
                } else {
                    let locs: Vec<String> = fuzzy_skipped.iter()
                        .map(|(s, l)| format!("L{}(差异:{})", s + 1, Self::match_level_name(*l)))
                        .collect();
                    format!("\n跳过模糊候选 {} 处（未替换，fuzzy: true 可纳入）: {}", fuzzy_skipped.len(), locs.join(", "))
                };
                let normalized_note = if needs_normalize {
                    " [已标准化换行]"
                } else {
                    ""
                };

                // Return context around first change for verification
                let first_start = match_positions.first().map(|(s, _)| *s).unwrap_or(0);
                let context_start = first_start.saturating_sub(3);
                let context_end = (first_start + new_lines.len() + 3).min(result_lines.len());
                let max_width = context_end.to_string().len();
                let context: Vec<String> = result_lines[context_start..context_end]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| {
                        let line_num = context_start + i + 1;
                        format!("{:>width$} | {}", line_num, line, width = max_width)
                    })
                    .collect();

                Ok(ToolResult::success(format!(
                    "{} replacement(s) at {} in {}{}{}\n修改后上下文 (lines {}-{}):\n{}",
                    match_count, positions.join(", "), path, skipped_note, normalized_note,
                    context_start + 1, context_end,
                    context.join("\n")
                )))
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_delete(&mut self) {
        self.register(ToolDef {
            name: "Delete".to_string(),
            description: "Delete a file (not directories). Rejects path traversal attempts."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to delete" }
                },
                "required": ["path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.is_empty() {
                    return Ok(ToolResult::failure("path is required"));
                }
                match std::fs::remove_file(path) {
                    Ok(_) => Ok(ToolResult::success(format!("Deleted: {}", path))),
                    Err(e) => Ok(ToolResult::failure(format!("Delete failed: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_rename(&mut self) {
        self.register(ToolDef {
            name: "Rename".to_string(),
            description: "Rename or move a file/directory".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source path" },
                    "to": { "type": "string", "description": "Destination path" }
                },
                "required": ["from", "to"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let from = params.get("from").and_then(|v| v.as_str()).unwrap_or("");
                let to = params.get("to").and_then(|v| v.as_str()).unwrap_or("");
                if from.is_empty() || to.is_empty() {
                    return Ok(ToolResult::failure("both 'from' and 'to' are required"));
                }
                match std::fs::rename(from, to) {
                    Ok(_) => Ok(ToolResult::success(format!("Renamed: {} -> {}", from, to))),
                    Err(e) => Ok(ToolResult::failure(format!("Rename failed: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_copy(&mut self) {
        self.register(ToolDef {
            name: "Copy".to_string(),
            description: "Copy a file (not directories) to new path".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Source path" },
                    "to": { "type": "string", "description": "Destination path" }
                },
                "required": ["from", "to"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let from = params.get("from").and_then(|v| v.as_str()).unwrap_or("");
                let to = params.get("to").and_then(|v| v.as_str()).unwrap_or("");
                if from.is_empty() || to.is_empty() {
                    return Ok(ToolResult::failure("both 'from' and 'to' are required"));
                }
                match std::fs::copy(from, to) {
                    Ok(bytes) => Ok(ToolResult::success(format!(
                        "Copied: {} -> {} ({} bytes)",
                        from, to, bytes
                    ))),
                    Err(e) => Ok(ToolResult::failure(format!("Copy failed: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_create_dir(&mut self) {
        self.register(ToolDef {
            name: "CreateDir".to_string(),
            description: "Create a directory (auto-creates parent dirs if needed)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to create" }
                },
                "required": ["path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.is_empty() {
                    return Ok(ToolResult::failure("path is required"));
                }
                match std::fs::create_dir_all(path) {
                    Ok(_) => Ok(ToolResult::success(format!("Created directory: {}", path))),
                    Err(e) => Ok(ToolResult::failure(format!("Mkdir failed: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_remove_dir(&mut self) {
        self.register(ToolDef {
            name: "RemoveDir".to_string(),
            description: "Remove a directory (empty) or recursively".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to remove" },
                    "recursive": { "type": "boolean", "default": false, "description": "Recursively delete all contents. When false, only removes empty directories." }
                },
                "required": ["path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let recursive = params.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);

                if path.is_empty() {
                    return Ok(ToolResult::failure("path is required"));
                }

                let p = std::path::Path::new(path);
                if !p.exists() {
                    return Ok(ToolResult::failure(format!("directory not found: {}", path)));
                }
                if !p.is_dir() {
                    return Ok(ToolResult::failure(format!("not a directory: {}", path)));
                }

                if recursive {
                    match std::fs::remove_dir_all(p) {
                        Ok(_) => Ok(ToolResult::success(format!("Removed directory (recursive): {}", path))),
                        Err(e) => Ok(ToolResult::failure(format!("Remove dir failed: {}", e))),
                    }
                } else {
                    match std::fs::remove_dir(p) {
                        Ok(_) => Ok(ToolResult::success(format!("Removed directory: {}", path))),
                        Err(e) => Ok(ToolResult::failure(format!("Remove dir failed (dir not empty?): {}", e))),
                    }
                }
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_glob(&mut self) {
        self.register(ToolDef {
            name: "Glob".to_string(),
            description: "Find files by glob pattern".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "patterns": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Glob pattern(s) to match filenames or relative paths (e.g. [\"*.rs\", \"src/**/*.rs\"])"
                    },
                    "path": {
                        "type": "string",
                        "description": "Root directory to search (default: current directory)"
                    }
                },
                "required": ["patterns"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let patterns: Vec<String> = params.get("patterns")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let root = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

                if patterns.is_empty() {
                    return Ok(ToolResult::failure("No patterns provided".to_string()));
                }

                let globs: Vec<glob::Pattern> = patterns.iter()
                    .map(|p| glob::Pattern::new(p))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("invalid glob pattern: {}", e))?;

                let root_path = std::path::Path::new(root);
                let mut paths = Vec::new();
                let walk = ignore::WalkBuilder::new(root)
                    .max_depth(Some(10))
                    .build();

                for entry in walk.filter_map(|e| e.ok()) {
                    let file_name = entry.file_name().to_str().unwrap_or("");
                    let rel_path = entry.path().strip_prefix(root_path)
                        .unwrap_or(entry.path())
                        .to_str()
                        .unwrap_or("");
                    if globs.iter().any(|g| g.matches(file_name) || g.matches(rel_path)) {
                        paths.push(entry.path().display().to_string());
                    }
                }

                let count = paths.len();
                let result = if paths.is_empty() {
                    "No files found.".to_string()
                } else {
                    format!("{}\n({} file{})",
                        paths.join("\n"),
                        count,
                        if count == 1 { "" } else { "s" })
                };
                Ok(ToolResult::success(result))
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_grep(&mut self) {
        self.register(ToolDef {
            name: "Grep".to_string(),
            description: "Search file contents by regex".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": { "type": "string", "description": "Root directory to search (default: current directory)" },
                    "-n": { "type": "boolean", "description": "Show line numbers in results" },
                    "-i": { "type": "boolean", "description": "Case-insensitive search" },
                    "head_limit": { "type": "integer", "description": "Max matches to return (default: 50)" }
                },
                "required": ["pattern"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let pattern = params.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let case_insensitive = params.get("-i").and_then(|v| v.as_bool()).unwrap_or(false);
                let _show_line_numbers = params.get("-n").and_then(|v| v.as_bool()).unwrap_or(false);
                let limit = params.get("head_limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;

                let pattern = if case_insensitive {
                    format!("(?i){}", pattern)
                } else {
                    pattern.to_string()
                };

                let regex = regex::Regex::new(&pattern)
                    .map_err(|e| format!("invalid regex: {}", e))?;

                let mut matches = Vec::new();
                let walk = ignore::WalkBuilder::new(path)
                    .hidden(false)
                    .git_ignore(true)
                    .build();

                for entry in walk.filter_map(|e| e.ok()) {
                    if matches.len() >= limit {
                        break;
                    }

                    let ft = match entry.file_type() {
                        Some(ft) => ft,
                        None => continue,
                    };
                    if !ft.is_file() {
                        continue;
                    }

                    // 跳过二进制文件和大文件
                    let path = entry.path();
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    let skip_exts = ["exe", "dll", "so", "dylib", "bin", "o", "a", "lib", "pdb", "ico", "png", "jpg", "jpeg", "gif", "svg", "mp3", "mp4", "zip", "tar", "gz", "rar", "7z", "pdf", "doc"];
                    if skip_exts.contains(&ext) {
                        continue;
                    }

                    // 检查文件大小，跳过 > 1MB 的文件
                    let metadata = match std::fs::metadata(path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if metadata.len() > 1_000_000 {
                        continue;
                    }

                    // 读取文件内容
                    let content = match std::fs::read_to_string(path) {
                        Ok(c) => c,
                        Err(_) => continue, // 二进制文件会在这里失败
                    };

                    // 逐行匹配，但限制每文件最大匹配数
                    let max_per_file = 10;
                    let mut file_matches = 0;
                    for (line_num, line) in content.lines().enumerate() {
                        if regex.is_match(line) {
                            matches.push(serde_json::json!({
                                "path": path.display().to_string(),
                                "line": line_num + 1,
                                "text": line
                            }));
                            file_matches += 1;

                            if matches.len() >= limit || file_matches >= max_per_file {
                                break;
                            }
                        }
                    }
                }

                let count = matches.len();
                let result = if matches.is_empty() {
                    "No matches found.".to_string()
                } else {
                    let lines: Vec<String> = matches.iter().map(|m| {
                        format!("{}:{}: {}",
                            m.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                            m.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
                            m.get("text").and_then(|v| v.as_str()).unwrap_or(""))
                    }).collect();
                    format!("{}\n({} match{})",
                        lines.join("\n"),
                        count,
                        if count == 1 { "" } else { "es" })
                };
                Ok(ToolResult::success(result))
            },
            depends_on: vec![],
        });
    }
    pub(crate) fn register_diff(&mut self) {
        self.register(ToolDef {
            name: "Diff".to_string(),
            description: "Compare two files as unified diff".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "original_path": { "type": "string", "description": "Path to the original/source file" },
                    "modified_path": { "type": "string", "description": "Path to the modified file" },
                    "context_lines": { "type": "integer", "minimum": 0, "maximum": 10, "description": "Context lines around each change (default 3)" }
                },
                "required": ["original_path", "modified_path"]
            }),
            category: ToolCategory::FileAccess,
            executor: |params, _ctx| {
                let original = params.get("original_path").and_then(|v| v.as_str()).unwrap_or("");
                let modified = params.get("modified_path").and_then(|v| v.as_str()).unwrap_or("");
                let context = params.get("context_lines").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
                let context = context.min(10);

                match crate::tools::builtin::diff::file_diff(original, modified, context) {
                    Ok(output) => Ok(ToolResult::success(output)),
                    Err(e) => Ok(ToolResult::failure(e)),
                }
            },
            depends_on: vec![],
        });
    }
}

#[cfg(test)]
mod edit_contract_tests {
    use crate::tools::registry::ToolRegistry;

    /// 样本：2 处精确（8 空格缩进，L2/L6）+ 1 处模糊候选（4 空格缩进，L4）
    const SAMPLE: &str =
        "header\n        needle = 1;\nmid\n    needle = 1;\ntail\n        needle = 1;\nend\n";

    fn setup_file(name: &str, content: &str) -> String {
        let dir = std::env::temp_dir().join("nuphus_edit_contract_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path.to_string_lossy().to_string()
    }

    fn run_edit(params: serde_json::Value) -> crate::ToolResult {
        let registry = ToolRegistry::builtin();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(registry.execute("Edit", &params))
            .expect("execute should not Err")
    }

    #[test]
    fn replace_all_defaults_to_exact_only() {
        let path = setup_file("t1.txt", SAMPLE);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "        needle = 1;", "new_string": "        needle = 2;",
            "replace_all": true
        }));
        assert!(r.success, "edit failed: {:?}", r.error);
        let out = r.output.unwrap();
        assert!(
            out.contains("2 replacement(s)"),
            "expected 2 replacements: {out}"
        );
        assert!(
            out.contains("跳过模糊候选 1 处"),
            "should report skipped fuzzy candidate: {out}"
        );
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("    needle = 1;"),
            "fuzzy candidate must remain untouched"
        );
        assert_eq!(content.matches("needle = 2;").count(), 2);
    }

    #[test]
    fn replace_all_with_fuzzy_includes_whitespace_variants() {
        let path = setup_file("t2.txt", SAMPLE);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "        needle = 1;", "new_string": "        needle = 2;",
            "replace_all": true, "fuzzy": true
        }));
        assert!(r.success, "edit failed: {:?}", r.error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            !content.contains("needle = 1;"),
            "all occurrences should be replaced"
        );
    }

    #[test]
    fn expected_count_aborts_atomically_on_mismatch() {
        let path = setup_file("t3.txt", SAMPLE);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "        needle = 1;", "new_string": "        needle = 2;",
            "replace_all": true, "expected_count": 5
        }));
        assert!(!r.success, "mismatch must fail");
        assert!(r.error.unwrap().contains("expected_count=5"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            SAMPLE,
            "file must be untouched on abort"
        );
    }

    #[test]
    fn expected_count_passes_on_exact_match() {
        let path = setup_file("t4.txt", SAMPLE);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "        needle = 1;", "new_string": "        needle = 2;",
            "replace_all": true, "expected_count": 2
        }));
        assert!(r.success, "matching count must succeed: {:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(&path)
                .unwrap()
                .matches("needle = 2;")
                .count(),
            2
        );
    }

    #[test]
    fn single_replace_still_tolerates_whitespace_drift() {
        // 回归保护：单次替换保留模糊定位（old_string 与文件缩进不一致也能命中第一处）
        let path = setup_file("t5.txt", SAMPLE);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "needle = 1;", "new_string": "needle = 9;"
        }));
        assert!(
            r.success,
            "single replace fuzzy locate failed: {:?}",
            r.error
        );
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content.matches("needle = 9;").count(),
            1,
            "only first occurrence replaced"
        );
        assert_eq!(content.matches("needle = 1;").count(), 2);
    }

    #[test]
    fn result_reports_all_positions() {
        let path = setup_file("t6.txt", SAMPLE);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "        needle = 1;", "new_string": "        needle = 2;",
            "replace_all": true
        }));
        assert!(r.success);
        let out = r.output.unwrap();
        assert!(out.contains("L2"), "should list line 2: {out}");
        assert!(out.contains("L6"), "should list line 6: {out}");
    }
}
