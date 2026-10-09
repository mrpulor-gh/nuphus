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
    /// 覆写前备份。`path` 为**用户原始入参**（绝对或相对），基准统一走
    /// [`crate::utils::resolve_user_path`]，与写入落点保持一致。
    ///
    /// 备份目录同样基于工作根：原先用相对路径 `.nuphus/backup`，会随进程 cwd
    /// 散落到程序目录 —— 与产物落错位置是同一类缺陷。
    fn backup_file(path: &str) -> Result<String, String> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let source = crate::utils::resolve_user_path(path);
        let backup_dir = crate::utils::work_root().join(".nuphus").join("backup");
        std::fs::create_dir_all(&backup_dir)
            .map_err(|e| format!("create backup dir failed: {}", e))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let file_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let backup_path = backup_dir.join(format!("{}_{}", file_name, now));

        std::fs::copy(&source, &backup_path).map_err(|e| format!("backup failed: {}", e))?;

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
                    "limit": { "type": "integer", "minimum": 1, "maximum": 5000, "description": "Max lines to return (default 2000)" }
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
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
                let limit = limit.min(5000);

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
                        format!(" (xlsx as markdown [还有 {} 行未显示，已显示 {}-{}，共 {} 行])", total_lines - end, start + 1, end, total_lines)
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
                        format!(" (office doc [还有 {} 行未显示，已显示 {}-{}，共 {} 行])", total_lines - end, start + 1, end, total_lines)
                    } else {
                        format!(" (lines {}-{} of {})", start + 1, end, total_lines)
                    };
                    return Ok(ToolResult::success(format!("{}{}\n{}", path, range_note, text)));
                }

                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）。
                // Read 必须与 Write 同一基准：读走 cwd、写走项目目录，会出现
                // 「刚写进项目目录的文件读不到」这种自相矛盾。
                let resolved = crate::utils::resolve_user_path(path);
                let content = std::fs::read_to_string(&resolved)
                    .map_err(|e| match e.kind() {
                        std::io::ErrorKind::PermissionDenied => {
                            format!("Permission denied: {}", path)
                        }
                        std::io::ErrorKind::NotFound => {
                            let root = crate::utils::work_root();
                            format!("File not found: {} (基准目录: {})", path, root.display())
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
                        " [还有 {} 行未显示（已显示 {}-{}，共 {} 行）；需要续读请用 offset={}]",
                        total_lines - end,
                        start + 1,
                        end,
                        total_lines,
                        end + 1
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

                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）：
                // Write 是「对话产物落到项目目录」这条链路的落点，基准则落进程序目录。
                let p = crate::utils::resolve_user_path(path);
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
            4 => "逐行去缩进",
            _ => "首尾空白",
        }
    }

    /// Normalize text: strip BOM, normalize CRLF→LF, strip trailing \r
    fn normalize_newlines(text: &str) -> String {
        let s = text.strip_prefix('\u{feff}').unwrap_or(text);
        // 确保没有残余 \r：先把 \r\n→\n，再删孤立的 \r
        s.replace("\r\n", "\n").replace('\r', "\n")
    }

    /// 文件块（自 `start` 起）与 `old_string` 的缩进形状是否一致。
    fn shape_matches(content_lines: &[&str], start: usize, old_str: &str) -> bool {
        let block: String = (start..start + old_str.lines().count())
            .map(|i| content_lines[i])
            .collect::<Vec<_>>()
            .join("\n");
        Self::remove_indentation(&block) == Self::remove_indentation(old_str)
    }

    /// 对称去缩进：剥掉 text 里所有非空行**共有的最小行首空白**，只留形状。
    ///
    /// 关键约束：**只用于比较，绝不用于写盘。** 文件块与 old_string 各自归零后比形状，
    /// 命中的仍是文件原文切片 —— 这样匹配能容忍两边缩进基准不同（Tab vs 空格、
    /// 4 空格基准 vs 8 空格基准、模型 dedent 锚点），又不会像逐行 trim_start 那样把
    /// **块内相对层级一起抹平**（那样 `a {\n  b();\n}` 会误配 `a {\nb();\n}`）。
    ///
    /// 用 `strip_prefix` 而非 `&s[n..]` 下标切片：行首空白可能含多字节字符
    /// （U+00A0 / U+3000 等），按字符数下标切会 panic 在非字符边界上。
    fn remove_indentation(text: &str) -> String {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut common: Option<String> = None;
        for line in lines.iter().filter(|l| !l.trim().is_empty()) {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            common = Some(match common {
                None => indent,
                Some(prev) => prev
                    .chars()
                    .zip(indent.chars())
                    .take_while(|(a, b)| a == b)
                    .map(|(_, b)| b)
                    .collect(),
            });
        }
        let common = match common {
            Some(c) if !c.is_empty() => c,
            // 无非空行，或本来就贴左边：形状未变，原样返回
            _ => return text.to_string(),
        };
        lines
            .iter()
            .map(|l| {
                if l.trim().is_empty() {
                    *l
                } else {
                    l.strip_prefix(common.as_str())
                        .unwrap_or_else(|| l.trim_start())
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 写盘后收敛缩进：Rust 走 `cargo fmt`，前端走 `prettier --write`。
    ///
    /// 写盘侧刻意不做缩进重建（`new_string` 逐字写入），所以模型给出的缩进若与文件基准
    /// 不一致就会原样保留。opencode 的做法是写完立刻 `format.file()` 再读回 ——
    /// **缩进问题在工具内部闭环，而不是甩给下一次 `cargo fmt --check` / prettier。**
    /// 这正是「Edit 之后才报缩进错」这个症状的根因：信号被搬到了离根因最远的地方。
    ///
    /// 只在**模糊命中**时触发：精确命中说明模型照着 Read 的真实内容给的缩进，本来就对，
    /// 此时不该有多余副作用（更不该在别人的仓库里擅自跑 fmt）。模糊命中才是缩进可能
    /// 漂移的唯一入口。
    ///
    /// 工具缺失 / 执行失败 / 非零退出都**不阻断编辑** —— 格式化是兜底收敛，不是前置条件。
    /// 返回 Some 表示确实跑过（回执里要如实告知模型）。
    fn format_after_write(path: &str) -> Option<String> {
        let lower = path.to_ascii_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        // 第三方 vendor 目录不动：改坏 vendored 代码不是本工具该做的事，
        // 而且这些文件通常带机器生成的标记，重排只会制造无关 diff。
        if lower.replace('\\', "/").contains("/third_party/") {
            return None;
        }
        // Rust 走 rustfmt 而非 `cargo fmt`：`cargo fmt` 只认 cargo 的 target 列表，
        // 对单个被编辑文件会直接 "Failed to find targets" 退出。rustfmt 能只格式化
        // 这一个文件，且自动向上找 rustfmt.toml / Cargo.toml 的 edition 配置。
        let (program, args): (&str, Vec<&str>) = if ext == "rs" {
            ("rustfmt", vec!["--edition", "2021"])
        } else if matches!(
            ext,
            "ts" | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "json"
                | "css"
                | "scss"
                | "md"
                | "yaml"
                | "yml"
        ) {
            ("prettier", vec!["--write", "--log-level", "silent"])
        } else {
            return None;
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args).arg(path);
        let ok = cmd
            .current_dir(
                std::path::Path::new(path)
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| std::path::Path::new(".")),
            )
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok.then(|| format!("已执行 `{program}` 收敛 {path} 的缩进"))
    }

    pub(crate) fn register_edit_file(&mut self) {
        self.register(ToolDef {
            name: "Edit".to_string(),
            description: "Line-level search-and-replace in file. Auto-backs up. Single replace uses whitespace-tolerant matching. replace_all defaults to EXACT matches only (set fuzzy:true to include whitespace-tolerant ones); use expected_count to atomically validate hit count — mismatch aborts without writing.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit" },
                    "old_string": { "type": "string", "description": "Text to find (multi-line supported). Indentation is matched tolerantly, but the tool NEVER rewrites your new_string's indentation — it is written verbatim." },
                    "new_string": { "type": "string", "description": "Replacement text, written verbatim. Copy the real indentation from Read output; do not re-indent or dedent it. If your match was whitespace-tolerant (the receipt shows a level other than 'exact'), the returned context reflects the file AFTER formatting — trust the context, not your input." },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: only first). Exact matches only unless fuzzy=true" },
                    "fuzzy": { "type": "boolean", "description": "With replace_all: also replace whitespace-tolerant matches (leading/trailing whitespace ignored). Default false — fuzzy candidates are reported but skipped" },
                    "expected_count": { "type": "integer", "description": "Expected number of replacements. If actual hits differ, the edit aborts atomically (nothing written). Grep first to get the count" },
                    "start_line": { "type": "integer", "minimum": 1, "description": "Optional inclusive 1-based first line of the search range" },
                    "end_line": { "type": "integer", "minimum": 1, "description": "Optional inclusive 1-based last line of the search range" }
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
                let start_line = params.get("start_line").and_then(|v| v.as_u64());
                let end_line = params.get("end_line").and_then(|v| v.as_u64());

                if let (Some(start), Some(end)) = (start_line, end_line) {
                    if start == 0 || end == 0 || start > end {
                        return Ok(ToolResult::failure(format!(
                            "行号范围无效: start_line={}、end_line={}，要求为 1-based 且 start_line <= end_line",
                            start, end
                        )));
                    }
                } else if start_line == Some(0) || end_line == Some(0) {
                    return Ok(ToolResult::failure("行号范围无效: start_line/end_line 必须从 1 开始"));
                }

                if old_str.is_empty() {
                    return Ok(ToolResult::failure("old_string cannot be empty"));
                }

                // .xlsx → 走结构化编辑：读为 CSV → 文本替换 → 写回 xlsx
                if path.to_lowercase().ends_with(".xlsx") {
                    if start_line.is_some() || end_line.is_some() {
                        return Ok(ToolResult::failure(
                            "start_line/end_line 仅支持文本文件，.xlsx 编辑不支持行号范围",
                        ));
                    }
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

                // Edit 是「读—改—写」全链路，基准必须与 Read/Write 完全一致，
                // 否则会出现「读到 A 处文件、改到 B 处文件」。唯一入口见 utils::resolve_user_path。
                let target = crate::utils::resolve_user_path(path);

                // 编码检测：读原始字节，验证 UTF-8 合法性
                // PowerShell 重定向 / Set-Content 默认用 ANSI/GBK，会损坏非 ASCII 字符
                let raw_bytes = std::fs::read(&target)
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

                let raw_content = std::fs::read_to_string(&target)
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

                // 行号范围按候选块的首行裁剪；范围缺省时保持原有全文件扫描。
                // 超出文件尾部的范围安全裁剪，不扩大搜索范围。
                let range_start = start_line.map(|line| (line as usize).saturating_sub(1)).unwrap_or(0);
                let range_end = end_line
                    .map(|line| (line as usize).min(content_lines.len()))
                    .unwrap_or(content_lines.len());
                let last_start = content_lines.len() - old_lines.len();
                let first_start = range_start.min(content_lines.len());
                let last_start = last_start.min(range_end.saturating_sub(old_lines.len()));

                if first_start > last_start {
                    return Ok(ToolResult::failure(format!(
                        "old_string not found in {}\n  搜索范围为 lines {}-{}，无法容纳 {} 行 old_string",
                        path,
                        start_line.unwrap_or(1),
                        end_line.unwrap_or(content_lines.len() as u64),
                        old_lines.len()
                    )));
                }

                for start in first_start..=last_start {
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
                    // Pass 3: 对称去缩进 —— 文件块与 old_string 各自剥掉共有最小缩进后比形状。
                    // 缩进弹性到此为止：命中的仍是 content_lines 的原文切片，写盘逐字写入
                    // （见下方 replacement 构造），不把匹配端的容错泄漏到写盘端。
                    else if Self::shape_matches(&content_lines, start, &old_str) {
                        matched = true;
                        level = 3;
                    }
                    // Pass 4: 忽略行首空白（逐行 trim_start）。
                    // 比 Pass 3 宽松：抹平块内相对层级，仅作 Pass 3 落空时的兜底。
                    else if old_lines.iter().enumerate().all(|(i, ol)| {
                        content_lines[start + i].trim_start() == ol.trim_start()
                    }) {
                        matched = true;
                        level = 4;
                    }
                    // Pass 5: ignore both leading and trailing whitespace
                    else if old_lines.iter().enumerate().all(|(i, ol)| {
                        content_lines[start + i].trim() == ol.trim()
                    }) {
                        matched = true;
                        level = 5;
                    }

                    if matched {
                        if replace_all && !allow_fuzzy && level > 1 {
                            fuzzy_skipped.push((start, level));
                            continue;
                        }
                        match_positions.push((start, level));
                        // 保持缺省调用的历史“首个命中”兼容语义；指定范围时扫描完整范围，
                        // 以便拒绝重复候选而不是静默选择可能错误的位置。
                        if !replace_all && start_line.is_none() && end_line.is_none() {
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

                // 显式行号范围意味着调用方正在消歧；单次替换范围内仍有多个候选时拒绝写入。
                // expected_count 只验证命中数量，不改变 replace_all=false 的单次替换语义。
                if (start_line.is_some() || end_line.is_some())
                    && !replace_all
                    && match_positions.len() > 1
                {
                    let positions: Vec<String> = match_positions
                        .iter()
                        .map(|(start, level)| format!("L{}({})", start + 1, Self::match_level_name(*level)))
                        .collect();
                    return Ok(ToolResult::failure(format!(
                        "行号范围内发现 {} 个候选，拒绝选择以避免歧义（候选: {}）。请缩小范围或使用 replace_all=true 明确执行全量替换，未做任何修改",
                        match_positions.len(), positions.join(", ")
                    )));
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
                // result_lines 用 owned String：缩进对齐后的 synced_lines 是局部 String，
                // 必需移入而非借用（&str 借用生命周期无法跨 splice 存活）。
                let mut result_lines: Vec<String> =
                    content_lines.iter().map(|s| s.to_string()).collect();

                for &(start, _level) in match_positions.iter().rev() {
                    let end = start + old_lines.len();
                    // new_string **逐字写入**：写盘侧不做任何缩进重建。
                    //
                    // 历史实现会在 level>=3 时把每行重挂成「文件原缩进 + 模型相对前缀」，
                    // 但基准只取自 new_string 的**首行**。模型习惯用闭合括号 / dedent 行
                    // 当锚点（首行缩进 0），此时 `own_prefix.starts_with("")` 恒真，
                    // relative_prefix 退化成整段缩进 → 结果 = 原缩进 + 模型缩进（双重叠加）。
                    // 这类损坏完全静默，只能等 fmt / 编译阶段炸出来，锅落在格式化工具头上。
                    //
                    // 现在缩进弹性只存在于**定位侧**（remove_indentation pass），命中的是
                    // 文件原文切片，写盘不猜。模型若给出与文件不一致的缩进，会原样落盘并
                    // 出现在返回的上下文里，随后由 format_after_write 收敛 ——
                    // 可见的错误优于静默的错误。
                    let replacement: Vec<String> = new_lines.iter().map(|s| s.to_string()).collect();
                    result_lines.splice(start..end, replacement);
                }

                let mut new_content = result_lines.join("\n");
                // `str::lines()` 丢弃末尾换行（`"a\n".lines()` → `["a"]`），join 不会补回。
                // 不还原的话每次编辑都会静默吃掉文件原有的 EOF 换行，触发 fmt / prettier /
                // editorconfig 报错。EOF 换行跟随原文件：原来有则保留，原来无则不加。
                // （CRLF 文件在此补 `\n`，由下方还原逻辑统一转回 `\r\n`。）
                if content.ends_with('\n') {
                    new_content.push('\n');
                }
                // 若原文件使用 \r\n，写回时保留原格式，避免整文件 diff 变动
                let write_content = if needs_normalize && raw_content.contains("\r\n") {
                    new_content.replace('\n', "\r\n")
                } else {
                    new_content
                };
                std::fs::write(&target, &write_content)
                    .map_err(|e| format!("write failed: {}", e))?;

                // 模糊命中是缩进可能漂移的唯一入口（精确命中的缩进来自 Read 的真实内容）。
                // 在这里闭环，而不是等调用方下一次 cargo fmt / prettier --check 才发现。
                let format_note = if match_positions.iter().all(|(_, l)| *l == 1) {
                    None
                } else {
                    Self::format_after_write(&target.to_string_lossy())
                };

                // 格式化可能重排缩进，上下文必须回读落盘结果，不能用内存里的 result_lines。
                let result_lines: Vec<String> = match std::fs::read_to_string(&target) {
                    Ok(s) => Self::normalize_newlines(&s).lines().map(|l| l.to_string()).collect(),
                    Err(_) => result_lines,
                };

                // 写后验证：读回文件，检查编码完整性和替换结果
                let verify_result = match std::fs::read(&target) {
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
                // 模糊命中后跑了格式化，必须如实回执 —— 模型看到的应是落盘真相，
                // 而不是它以为写进去的样子。
                let format_str = match &format_note {
                    Some(note) => format!("\n{}", note),
                    None => String::new(),
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
                    "{} replacement(s) at {} in {}{}{}{}\n修改后上下文 (lines {}-{}):\n{}",
                    match_count, positions.join(", "), path, skipped_note, normalized_note,
                    format_str,
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
                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）
                let (src, dst) = (
                    crate::utils::resolve_user_path(from),
                    crate::utils::resolve_user_path(to),
                );
                match std::fs::rename(&src, &dst) {
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
                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）
                let (src, dst) = (
                    crate::utils::resolve_user_path(from),
                    crate::utils::resolve_user_path(to),
                );
                match std::fs::copy(&src, &dst) {
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
                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）
                let p = crate::utils::resolve_user_path(path);
                match std::fs::create_dir_all(&p) {
                    Ok(_) => Ok(ToolResult::success(format!(
                        "Created directory: {}",
                        p.display()
                    ))),
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

                let p = crate::utils::resolve_user_path(path);
                if !p.exists() {
                    return Ok(ToolResult::failure(format!("directory not found: {}", path)));
                }
                if !p.is_dir() {
                    return Ok(ToolResult::failure(format!("not a directory: {}", path)));
                }

                if recursive {
                    match std::fs::remove_dir_all(&p) {
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
                        "description": "Root directory to search (default: current work root; absolute paths unaffected)"
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

                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）：
                // 与 Write/Read 同基准，否则「写进项目目录、在 cwd 里搜」会搜不到。
                let root_path = crate::utils::resolve_user_path(root);
                let mut paths = Vec::new();
                let walk = ignore::WalkBuilder::new(&root_path)
                    .max_depth(Some(10))
                    .build();

                for entry in walk.filter_map(|e| e.ok()) {
                    let file_name = entry.file_name().to_str().unwrap_or("");
                    let rel_path = entry.path().strip_prefix(&root_path)
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
                    "path": { "type": "string", "description": "Root directory to search (default: current work root; absolute paths unaffected)" },
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

                // 相对路径以当前工作根为基准（唯一入口，见 utils::resolve_user_path）：
                // 与 Write/Read 同基准，否则「写进项目目录、在 cwd 里搜」会搜不到。
                let root_path = crate::utils::resolve_user_path(path);
                let mut matches = Vec::new();
                let walk = ignore::WalkBuilder::new(&root_path)
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

    /// 相对路径必须落在**工作根**（此处用隔离 HOME 配好项目目录），而非进程 cwd。
    ///
    /// 守护的是产品语义（issue：产物落错位置）：Write 落项目目录、Read/Edit 却按 cwd
    /// 解析时，会出现「刚写进去的文件读不到、改不到」。本用例把三个动作串起来验证同一基准。
    #[test]
    fn relative_paths_resolve_against_work_root() {
        let _guard = crate::utils::path_base_tests::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // 隔离 HOME：把项目目录指向一个干净临时目录
        let home = std::env::temp_dir().join(format!("nuphus-edit-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nuphus")).unwrap();
        let project = home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            home.join(".nuphus").join("preferences.json"),
            format!(
                "{{\"language\":\"zh-CN\",\"project_dir\":{:?}}}",
                project.to_string_lossy()
            ),
        )
        .unwrap();

        let previous_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &home);

        // 用相对路径写入 → 应落在项目目录
        let rel_name = "rel_probe.txt";
        let write = {
            let registry = ToolRegistry::builtin();
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(registry.execute(
                "Write",
                &serde_json::json!({ "path": rel_name, "content": "line-a\nline-b\n" }),
            ))
            .expect("execute should not Err")
        };
        assert!(write.success, "write failed: {:?}", write.error);
        assert!(
            project.join(rel_name).exists(),
            "相对路径写入必须落在项目目录：{}",
            project.join(rel_name).display()
        );

        // 同一相对路径的 Edit 必须命中同一文件（读—改—写同一基准）
        let edit = run_edit(serde_json::json!({
            "path": rel_name, "old_string": "line-a", "new_string": "line-A"
        }));
        assert!(edit.success, "edit failed: {:?}", edit.error);
        let after = std::fs::read_to_string(project.join(rel_name)).unwrap();
        assert!(after.contains("line-A"), "Edit 应改到项目目录下的同一文件");

        match previous_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// issue #53 补齐：目录创建 / 复制 / 改名 / 搜索 / 对比 的入口必须与 Write/Read/Edit
    /// 同一基准（当前工作根）。此前这些入口各自按进程 cwd 解析，出现「写进项目目录、
    /// 却在 cwd 里搜 / 移动 / 对比」的断链——本用例把六个入口串起来守护同一基准。
    #[test]
    fn directory_copy_rename_search_and_diff_entrypoints_share_the_write_base() {
        let _guard = crate::utils::path_base_tests::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        // 隔离 HOME：把项目目录指向一个干净临时目录（同 relative_paths_resolve_against_work_root）
        let home = std::env::temp_dir().join(format!("nuphus-entry-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".nuphus")).unwrap();
        let project = home.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            home.join(".nuphus").join("preferences.json"),
            format!(
                "{{\"language\":\"zh-CN\",\"project_dir\":{:?}}}",
                project.to_string_lossy()
            ),
        )
        .unwrap();
        let previous_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &home);

        let registry = ToolRegistry::builtin();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let run = |name: &str, params: serde_json::Value| {
            let result = rt
                .block_on(registry.execute(name, &params))
                .expect("execute should not Err");
            assert!(result.success, "{} failed: {:?}", name, result.error);
            result.output.unwrap_or_default()
        };

        // 六个入口全部使用相对路径，落点应与 Write 完全一致
        run(
            "Write",
            serde_json::json!({ "path": "probe/entry.txt", "content": "alpha line\nbeta line\n" }),
        );
        run("CreateDir", serde_json::json!({ "path": "made_by_tool" }));
        run(
            "Copy",
            serde_json::json!({ "from": "probe/entry.txt", "to": "probe/copy.txt" }),
        );
        run(
            "Rename",
            serde_json::json!({ "from": "probe/copy.txt", "to": "probe/renamed.txt" }),
        );
        run(
            "Write",
            serde_json::json!({ "path": "probe/renamed.txt", "content": "gamma line\n" }),
        );

        // 落点必须在项目目录，进程 cwd 下不能留下任何痕迹
        let cwd = std::env::current_dir().unwrap();
        assert!(
            project.join("made_by_tool").is_dir(),
            "CreateDir 必须落在项目目录"
        );
        assert!(
            !cwd.join("made_by_tool").exists(),
            "CreateDir 不得落在进程 cwd"
        );
        assert!(
            project.join("probe/renamed.txt").exists(),
            "Rename 必须落在项目目录"
        );
        assert!(!cwd.join("probe").exists(), "相对路径不得在 cwd 建出目录");

        // 搜索与对比默认 root 即工作根：项目目录内文件可被命中
        let glob = run("Glob", serde_json::json!({ "patterns": ["renamed.txt"] }));
        assert!(
            glob.contains("renamed.txt"),
            "Glob 应命中项目目录内文件：{glob}"
        );
        let grep = run("Grep", serde_json::json!({ "pattern": "gamma" }));
        assert!(
            grep.contains("renamed.txt"),
            "Grep 应命中项目目录内文件：{grep}"
        );
        let diff = run(
            "Diff",
            serde_json::json!({
                "original_path": "probe/entry.txt",
                "modified_path": "probe/renamed.txt"
            }),
        );
        assert!(
            diff.contains("gamma"),
            "Diff 应按工作根读取并产出差异：{diff}"
        );

        match previous_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&home);
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

    #[test]
    fn fuzzy_match_locates_by_shape_and_writes_verbatim() {
        // 回归保护（2026-09-08 反复踩坑）：old_string 无缩进、文件行带缩进时，
        // **定位**必须成功 —— 这是缩进弹性的唯一承诺。
        //
        // 旧用例断言「替换后必须保留文件原缩进」，那是写盘端重建缩进的副作用，
        // 而重建正是双重叠加 bug 的来源。契约已反转：定位端容错，写盘端逐字。
        // 缩进保持原样由调用方负责（照 Read 的真实内容给 new_string）。
        let path = setup_file(
            "t7-indent.txt",
            "{\n  \"version\": \"0.2.7\",\n  \"build\": {}\n}\n",
        );
        // old_string dedent（定位需要容错），new_string 带真实缩进（照 Read 的内容给）。
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "\"version\": \"0.2.7\",",
            "new_string": "  \"version\": \"0.2.8\","
        }));
        assert!(r.success, "fuzzy edit failed: {:?}", r.error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content, "{\n  \"version\": \"0.2.8\",\n  \"build\": {}\n}\n",
            "模糊定位必须命中，且 new_string 逐字写入"
        );
    }

    /// 缩进弹性的契约测试：old_string 与文件缩进不一致时仍能定位（哪怕 new_string dedent）。
    ///
    /// 守护的是「定位端容错」这条承诺本身，不是任何缩进重建行为。
    #[test]
    fn dedented_old_string_still_locates_indented_block() {
        let path = setup_file(
            "t13-dedent-locate.yaml",
            "root:\n    item: old\n    other: 1\n",
        );
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "item: old",
            "new_string": "item: new"
        }));
        assert!(r.success, "dedent locate failed: {:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "root:\nitem: new\n    other: 1\n",
            "new_string 逐字写入；定位不重建缩进"
        );
    }

    #[test]
    fn trailing_whitespace_match_does_not_rewrite_explicit_replacement_indent() {
        // level 2 仅忽略行尾空白；new_string 的显式行首缩进必须原样保留。
        let path = setup_file("t9-trailing-whitespace.txt", "  key: old;  \n");
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "  key: old;",
            "new_string": "    key: new;"
        }));
        assert!(r.success, "trailing-whitespace edit failed: {:?}", r.error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "    key: new;\n");
    }

    #[test]
    fn indentation_fuzzy_match_locates_block_but_writes_new_string_verbatim() {
        // level 3（对称去缩进）只在**定位**侧生效。写盘侧不重建缩进。
        //
        // 旧断言是 `root:\n\titem: new\n\t  child: value\n` —— 那是「文件原缩进 \t +
        // 模型缩进」的叠加结果，属于被误当成契约的损坏行为。模型给的就是
        // `item: new` / `  child: value`，工具就该原样写这两行。
        let path = setup_file("t10-tab-multiline.txt", "root:\n\titem: old\nend\n");
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "item: old",
            "new_string": "item: new\n  child: value"
        }));
        assert!(r.success, "tab multiline edit failed: {:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "root:\nitem: new\n  child: value\nend\n",
            "new_string 必须逐字写入：定位端容错不得泄漏到写盘端"
        );
    }

    #[test]
    fn fuzzy_match_writes_new_string_verbatim_without_rehanging_indent() {
        // 多行替换：new_string 逐字写入，缩进不被重挂。
        //
        // 旧断言 `a:\n  b: 10\n    c: 20\n    d: 30\n` 里那 4 空格 =
        // 文件原缩进 2 + 模型缩进 2 的**双重叠加**，正是 2026-09 反复踩坑的那个 bug；
        // 它被写进了断言，于是错误变成了契约。现在契约是：原样写入。
        let path = setup_file("t8-multiline.txt", "a:\n  b: 1\n  c: 2\nd: 3\n");
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "b: 1\n  c: 2",
            "new_string": "b: 10\n  c: 20\n  d: 30"
        }));
        assert!(r.success, "multiline fuzzy edit failed: {:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a:\nb: 10\n  c: 20\n  d: 30\nd: 3\n",
            "new_string 必须逐字写入"
        );
    }

    /// 核心回归守卫：模型用 dedent 锚点（旧实现的真实触发条件）时，绝不能双重叠加。
    ///
    /// 旧实现的缩进基准取自 new_string **首行**。首行缩进为 0（顶层 struct / 贴左的
    /// 闭合括号）时 `own_prefix.starts_with("")` 恒真，relative_prefix 退化成整段缩进，
    /// 结果 = 文件原缩进 + 模型缩进：8/8/4 落成 12/12/8。静默损坏，只能等 fmt 阶段炸。
    #[test]
    fn dedented_first_line_does_not_get_double_indented() {
        // 用 .txt 而非 .rs：否则 format_after_write 会跑 rustfmt 把结果重排，
        // 断言的就不是「写盘是否逐字」而是「rustfmt 输出什么」，测不到目标行为。
        let path = setup_file(
            "t11-dedent-first-line.txt",
            "    let old = 1;\n    let keep = 2;\n",
        );
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "let old = 1;",
            "new_string": "struct Added {\n        a: u8,\n        b: u8,\n    }"
        }));
        assert!(r.success, "dedent edit failed: {:?}", r.error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content, "struct Added {\n        a: u8,\n        b: u8,\n    }\n    let keep = 2;\n",
            "首行 dedent 时必须逐字写入，不能把 8 空格缩进叠加到原 4 空格基准上"
        );
        assert!(
            !content.contains("            a: u8"),
            "首行 dedent 触发了缩进双重叠加：\n{}",
            content
        );
    }

    /// 模型照 Read 的真实缩进给 new_string 时，必须逐字往返（无任何改写）。
    #[test]
    fn new_string_with_real_indent_round_trips_verbatim() {
        let path = setup_file("t12-verbatim.txt", "fn f() {\n    let x = 1;\n}\n");
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "    let x = 1;",
            "new_string": "    let x = 2;\n    let y = 3;"
        }));
        assert!(r.success, "verbatim edit failed: {:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn f() {\n    let x = 2;\n    let y = 3;\n}\n"
        );
    }

    /// 对称去缩进必须保留**块内相对层级**。
    ///
    /// 这是它优于逐行 trim_start 的地方：`a {\n  b();\n}` 与 `a {\nb();\n}` 形状不同，
    /// 不该互相匹配。反过来，两边缩进基准不同但形状相同必须能匹配（Tab vs 空格）。
    #[test]
    fn remove_indentation_is_symmetric_and_keeps_inner_relative_shape() {
        // 基准不同、形状相同 → 归一化后相等
        assert_eq!(
            ToolRegistry::remove_indentation("        foo();\n            bar();"),
            ToolRegistry::remove_indentation("    foo();\n        bar();")
        );
        // 注意：Tab 与空格**不会**互相归一 —— 只剥「共有的最小缩进」，不替换空白字符。
        // 混合缩进文件靠 Pass 4（逐行 trim_start）兜底，不靠本函数跨字符集匹配。
        assert_ne!(
            ToolRegistry::remove_indentation("\t\tfoo();\n\t\t\tbar();"),
            ToolRegistry::remove_indentation("    foo();\n        bar();")
        );
        // 形状不同（内层相对层级不一致）→ 必须不等
        assert_ne!(
            ToolRegistry::remove_indentation("a {\n  b();\n}"),
            ToolRegistry::remove_indentation("a {\nb();\n}")
        );
        // 空白行不参与基准计算，也不被 strip_prefix 破坏
        assert_eq!(
            ToolRegistry::remove_indentation("  a:\n\n  b: 1\n"),
            "a:\n\nb: 1\n"
        );
        // 全空白行：形状未变
        assert_eq!(ToolRegistry::remove_indentation("\n  \n"), "\n  \n");
    }

    #[test]
    fn line_range_targets_repeated_candidate_and_reports_position() {
        let path = setup_file("t10-range.txt", "a\nneedle\nb\nneedle\nc\n");
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "needle", "new_string": "changed",
            "start_line": 4, "end_line": 4
        }));
        assert!(r.success, "range edit failed: {:?}", r.error);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "a\nneedle\nb\nchanged\nc\n");
        assert!(r.output.unwrap().contains("L4"));
    }

    #[test]
    fn line_range_rejects_ambiguous_candidates_without_writing() {
        let original = "a\nneedle\nneedle\nc\n";
        let path = setup_file("t11-range-ambiguous.txt", original);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "needle", "new_string": "changed",
            "start_line": 2, "end_line": 3
        }));
        assert!(!r.success, "ambiguous range must fail");
        assert!(r.error.unwrap().contains("L2"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn line_range_expected_count_does_not_promote_single_replace_to_replace_all() {
        let original = "a\nneedle\nneedle\nc\n";
        let path = setup_file("t13-range-expected-count.txt", original);
        let r = run_edit(serde_json::json!({
            "path": path,
            "old_string": "needle",
            "new_string": "changed",
            "start_line": 2,
            "end_line": 3,
            "expected_count": 2
        }));
        assert!(!r.success, "single replace must reject multiple candidates");
        assert!(r.error.unwrap().contains("replace_all"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn line_range_out_of_bounds_does_not_expand_search() {
        let original = "a\nneedle\nb\n";
        let path = setup_file("t12-range-boundary.txt", original);
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "needle", "new_string": "changed",
            "start_line": 4, "end_line": 99
        }));
        assert!(!r.success, "out-of-bounds range must not find earlier line");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn edit_preserves_trailing_newline() {
        // `str::lines()` 丢弃末尾换行：原文件有 EOF 换行时，编辑后必须保留，
        // 否则每次编辑都静默吃掉末尾换行（触发 fmt / prettier / editorconfig 报错）。
        let path = setup_file("t14-eof-newline.txt", "a\nb\nc\n");
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "b", "new_string": "B"
        }));
        assert!(r.success, "edit failed: {:?}", r.error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nB\nc\n");
    }

    #[test]
    fn edit_does_not_add_trailing_newline_when_absent() {
        // 反向契约：原文件无 EOF 换行时不得凭空添加（EOF 换行跟随原文件）。
        let path = setup_file("t15-no-eof-newline.txt", "a\nb\nc");
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "b", "new_string": "B"
        }));
        assert!(r.success, "edit failed: {:?}", r.error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nB\nc");
    }

    #[test]
    fn edit_preserves_trailing_newline_for_crlf_file() {
        // CRLF 文件：补回的 `\n` 由写回时的 CRLF 还原逻辑统一转回 `\r\n`。
        let path = setup_file("t16-crlf-eof.txt", "a\r\nb\r\nc\r\n");
        let r = run_edit(serde_json::json!({
            "path": path, "old_string": "b", "new_string": "B"
        }));
        assert!(r.success, "edit failed: {:?}", r.error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nB\r\nc\r\n");
    }
}
