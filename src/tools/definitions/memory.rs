//! 记忆/时间线工具定义
//!
//! 包含记忆查询、时间线搜索、会话历史等 ToolDef 注册方法。

use crate::memory::entry::{MemoryEntry, MemoryKind};
use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

/// 判断查询是否像标签词（纯 ASCII 无空格，含下划线分隔）
/// 如 "check_pattern", "session_refine", "high_quality"
fn is_tag_like(q: &str) -> bool {
    !q.contains(' ')
        && q.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 严格 AND 零结果时的诊断文本：逐词命中数 + 行动引导
///
/// 只解释零结果原因并给出退路，绝不降级返回部分匹配结果集。
fn format_zero_result_diagnostic(counts: &[(String, usize)]) -> String {
    let per_token = if counts.is_empty() {
        "（无有效关键词）".to_string()
    } else {
        counts
            .iter()
            .map(|(t, c)| format!("{}={}", t, c))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "未找到同时覆盖全部关键词的记忆（多关键词为严格 AND，不返回部分匹配）。\n逐词命中数: {}\n建议: 无单一记忆覆盖全部关键词；可减少关键词重试，或用 semantic=true 语义搜索。",
        per_token
    )
}

/// 解析工具参数中的 kind 字符串（非法值返回带可选值列表的错误）
fn parse_kind_param(params: &serde_json::Value) -> Result<Option<MemoryKind>, ToolResult> {
    match params.get("kind").and_then(|v| v.as_str()) {
        None => Ok(None),
        Some(s) => match s.parse::<MemoryKind>() {
            Ok(k) => Ok(Some(k)),
            Err(_) => Err(ToolResult::failure(format!(
                "非法 kind: '{}'，可选值: conversation / task_trace / distill / pattern / snapshot",
                s
            ))),
        },
    }
}

fn format_entry(e: &MemoryEntry, score: Option<f32>) -> String {
    let sid = e.session_id.clone();
    let ts = e
        .created_at
        .parse::<chrono::DateTime<chrono::Utc>>()
        .ok()
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let status = if e.success { "✓" } else { "✗" };
    // Best snippet: summary > pattern > output > intent
    let snippet = if !e.summary.is_empty() {
        &e.summary
    } else {
        e.pattern
            .as_deref()
            .or(e.output.as_deref())
            .unwrap_or(&e.intent)
    };
    let snippet = snippet
        .chars()
        .take(500)
        .collect::<String>()
        .replace('\n', " ");
    let score_str = match score {
        Some(s) => format!(" {:.2}", s),
        None => String::new(),
    };
    let tag_str = if e.tags.is_empty() {
        String::new()
    } else {
        format!(
            " | {}",
            e.tags
                .iter()
                .take(5)
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    format!(
        "[{}]#{} {} {} [{}/{}]{} {} |{}",
        sid,
        e.turn_id,
        ts,
        status,
        e.agent_type.as_str(),
        e.kind.as_str(),
        score_str,
        snippet,
        tag_str
    )
}

impl ToolRegistry {
    pub(crate) fn register_search_timeline(&mut self) {
        self.register(ToolDef {
            name: "memory_search".to_string(),
            description: "搜索记忆条目。kind 四类：conversation（用户对话对，找\"当时说了什么\"）、task_trace（任务执行轨迹，含工具调用摘要，排查执行问题）、distill（会话提炼，LLM 压缩的语义摘要）、pattern（实战模式，经评分验证的可复用经验）。最佳实践：找经验用 kind=pattern/distill，找对话用 conversation，排查执行用 task_trace。不指定 kind 时默认排除 task_trace（执行轨迹噪声大），需要排查执行过程时显式传 kind=task_trace。默认只检索当前项目的记忆（session 归属由 session_meta 自动登记）；跨项目检索先 read 记忆索引中其它项目的文件获取线索，再传 all_projects=true 看全局。关键词必须优先从用户当前消息中提取 1-2 个核心原词（用户原词比自行推测命中率高），避免多词严格 AND 互相绞杀。默认关键词 FTS5 检索（多关键词=严格 AND（虚词/单字自动过滤，自然句式可直接查询），不返回部分匹配；已支持前缀匹配，如\"单例\"可命中\"单例化\"；零结果时返回逐词命中数诊断——可减少关键词重试，或用 semantic=true 语义搜索）；semantic=true 用向量语义搜索；标签式查询（如 check_pattern）走标签匹配。可选 session_id/goal_type/kind 过滤（session_id 精查不受项目过滤限制），include_annotations 附加标注结果。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "搜索关键词"},
                    "limit": {"type": "integer", "default": 10, "description": "Max results"},"session_id": {"type": "string", "description": "按会话 ID 过滤，追踪因果链"},
                    "goal_type": {"type": "string", "description": "按操作类型过滤"},
                    "kind": {"type": "string", "enum": ["conversation", "task_trace", "distill", "pattern"], "description": "按记忆类别过滤：conversation=对话对 / task_trace=执行轨迹 / distill=会话提炼 / pattern=实战模式"},
                    "semantic": {"type": "boolean", "default": false, "description": "Use semantic (vector) search instead of keyword FTS5"},
                    "include_annotations": {"type": "boolean", "default": false, "description": "Also search annotations by the same keyword and append results"},
                    "all_projects": {"type": "boolean", "default": false, "description": "true = 不限当前项目，检索所有项目的记忆（跨项目场景先 read 其它项目记忆文件获取线索）"}
                },
                "required": ["query"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                use crate::store::memory;
                let query = params.get("query").and_then(|v| v.as_str());
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let session_id = params.get("session_id").and_then(|v| v.as_str());
                let goal_type = params.get("goal_type").and_then(|v| v.as_str());
                let kind = match parse_kind_param(params) {
                    Ok(k) => k,
                    Err(tr) => return Ok(tr),
                };
                let semantic = params.get("semantic").and_then(|v| v.as_bool()).unwrap_or(false);
                let include_annotations = params.get("include_annotations").and_then(|v| v.as_bool()).unwrap_or(false);
                // 项目隔离：默认只查当前项目（session_meta 归属），all_projects=true 看全局
                let all_projects = params.get("all_projects").and_then(|v| v.as_bool()).unwrap_or(false);
                // 默认排除 TaskTrace（执行轨迹噪声大），显式传 kind 时不过滤
                let exclude_task_trace = kind.is_none();
                // semantic / filtered 路径为 Rust 后过滤，需 3x 取回兜底；
                // FTS scored 路径已在 SQL 层排除 task_trace，取 limit 即可
                let fetch_limit = if exclude_task_trace { limit * 3 } else { limit };

                let query_ref = query.as_ref();
                let mut zero_result_diagnostic: Option<String> = None;
                #[allow(clippy::unnecessary_unwrap)]
                let entries: Vec<String> = if semantic && query_ref.is_some() {
                    let mut results = memory::search_entries_semantic(query_ref.unwrap(), fetch_limit, kind, all_projects)
                        .map_err(|e| format!("semantic search failed: {}", e))?;
                    if let Some(sid) = session_id { results.retain(|(e,_)| e.session_id == sid); }
                    if let Some(gt) = goal_type { results.retain(|(e,_)| e.goal_type.as_deref() == Some(gt)); }
                    results.into_iter()
                        .filter(|(e,_)| !exclude_task_trace || !matches!(e.kind, crate::memory::entry::MemoryKind::TaskTrace))
                        .take(limit)
                        .map(|(e, score)| format_entry(&e, Some(score))).collect()
                } else if session_id.is_some() || goal_type.is_some() {
                    let results = memory::search_entries_filtered(query, session_id, goal_type, None, kind, None, None, None, None, None, None, fetch_limit, all_projects)
                        .map_err(|e| format!("search failed: {}", e))?;
                    results.iter()
                        .filter(|e| !exclude_task_trace || !matches!(e.kind, crate::memory::entry::MemoryKind::TaskTrace))
                        .take(limit)
                        .map(|e| format_entry(e, None)).collect()
                } else if query.map(is_tag_like).unwrap_or(false) {
                    // 标签式查询：走 LIKE 匹配 tags/summary，比 FTS5 BM25 更精准
                    let results = memory::search_entries_filtered(None, None, None, None, kind, None, None, None, None, None, query, fetch_limit, all_projects)
                        .map_err(|e| format!("search failed: {}", e))?;
                    results.iter()
                        .filter(|e| !exclude_task_trace || !matches!(e.kind, crate::memory::entry::MemoryKind::TaskTrace))
                        .take(limit)
                        .map(|e| format_entry(e, None)).collect()
                } else {
                    // ── FTS5 关键词搜索路径（含 BM25 分数）──
                    // task_trace 排除已下推 SQL 层，直接取 limit，无需 Rust 后过滤
                    let q = query.unwrap_or("");
                    let results = memory::search_entries_scored(q, limit, kind, exclude_task_trace, all_projects)
                        .map_err(|e| format!("search failed: {}", e))?;
                    if results.is_empty() {
                        // AND 零结果：逐词命中数诊断 + 行动引导，不降级返回结果集
                        let counts = memory::token_hit_counts(q).unwrap_or_default();
                        zero_result_diagnostic = Some(format!(
                            "{}\n提示：默认仅检索当前项目的记忆（session_meta 归属）；若预期命中其它项目，先 read 记忆索引中该项目文件获取线索，再传 all_projects=true。",
                            format_zero_result_diagnostic(&counts)
                        ));
                    }
                    results
                        .into_iter()
                        .map(|(e, score)| format_entry(&e, Some(score)))
                        .collect()
                };

                let count = entries.len();
                let mut result = if entries.is_empty() {
                    zero_result_diagnostic.unwrap_or_else(|| "No entries found.".to_string())
                } else {
                    format!("{}\n({} entr{})",
                        entries.join("\n"),
                        count,
                        if count == 1 { "y" } else { "ies" })
                };

                // ── 统一检索：附加 annotation 结果 ──
                if include_annotations {
                    if let Some(q) = query {
                        let anns = crate::annotation::store::AnnotationStore::search(q);
                        if !anns.is_empty() {
                            result.push_str(&format!("\n\n## Annotations matching '{}' ({} result{})\n", q, anns.len(), if anns.len() == 1 { "" } else { "s" }));
                            for a in &anns {
                                result.push_str(&format!("- **{}**: {}\n", a.keyword, a.description));
                                if let Some(meid) = &a.memory_entry_id {
                                    result.push_str(&format!("  ↳ memory: {}\n", meid));
                                }
                            }
                        }
                    }
                }
                Ok(ToolResult::success(result))
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_recent_timeline(&mut self) {
        self.register(ToolDef {
            name: "memory_recent".to_string(),
            description: "查看最近的跨会话记忆记录（时间/状态/kind/摘要/标签）。默认只看当前项目的记忆（all_projects=true 看全局）。kind 未指定时默认排除 task_trace（执行轨迹噪声大，只看高价值记录：对话/提炼/模式）；需要排查执行过程时显式指定 kind=task_trace。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "default": 10, "description": "返回多少条最近记录"},
                    "kind": {"type": "string", "enum": ["conversation", "task_trace", "distill", "pattern"], "description": "按记忆类别过滤；不指定时默认排除 task_trace"},
                    "all_projects": {"type": "boolean", "default": false, "description": "true = 不限当前项目，查看所有项目的最近记录"}
                }
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                use crate::store::memory;
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let kind = match parse_kind_param(params) {
                    Ok(k) => k,
                    Err(tr) => return Ok(tr),
                };
                let all_projects = params.get("all_projects").and_then(|v| v.as_bool()).unwrap_or(false);
                // kind 未指定时排除 task_trace（噪声最大），指定时精确过滤
                let exclude: &[MemoryKind] = if kind.is_none() {
                    &[MemoryKind::TaskTrace]
                } else {
                    &[]
                };
                let results = memory::recent_entries(limit, kind, exclude, all_projects).map_err(|e| format!("recent failed: {}", e))?;
                let count = results.len().min(limit);
                let entries: Vec<String> = results.into_iter().take(limit).map(|e| format_entry(&e, None)).collect();
                let result = if entries.is_empty() {
                    "No recent entries.".to_string()
                } else {
                    format!("{}\n({} entries)", entries.join("\n"), count)
                };
                Ok(ToolResult::success(result))
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_timeline_stats(&mut self) {
        self.register(ToolDef {
            name: "memory_stats".to_string(),
            description: "查看记忆系统统计：总条目数、成功率、agent 分布、kind 分布（conversation 对话 / task_trace 执行轨迹 / distill 提炼 / pattern 实战模式 / snapshot 工作快照）。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            category: ToolCategory::Core,
            executor: |_params, _ctx| {
                use crate::store::memory;
                let stats = memory::entry_stats().map_err(|e| format!("stats failed: {}", e))?;
                let success_rate = if stats.total_entries > 0 {
                    (stats.by_success as f64 / stats.total_entries as f64 * 100.0).round() / 100.0
                } else {
                    0.0
                };
                let by_kind: serde_json::Map<String, serde_json::Value> = stats.by_kind
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                    .collect();
                let result = serde_json::json!({
                    "total_entries": stats.total_entries,
                    "leader_count": stats.by_agent_leader,
                    "exec_count": stats.by_agent_exec,
                    "successful": stats.by_success,
                    "failed": stats.by_failure,
                    "success_rate": success_rate,
                    "by_kind": by_kind,
                });
                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
                ))
            },
            depends_on: vec![],
        });
    }

    /// Leader 主动记忆更新工具 —— 只允许 Leader 写入，ExecAgent 不可访问
    ///
    /// 写入 ~/.nuphus/memory.md（用户数据目录，见 utils::nuphus_data_dir）。
    /// 这不是随手笔记，而是重点阶段/文件/操作的索引指南，
    /// 会被注入到 Leader 每轮 prompt 中用于快速决策。
    /// 自动截断到 2000 字符，保留尾部最新内容。
    pub(crate) fn register_leader_memory_update(&mut self) {
        self.register(ToolDef {
            name: "leader::memory_update".to_string(),
            description: "Append a dated summary entry to the project memory journal (.nuphus/memory/{tag}.md) — the LLM working memory whose newest tail is auto-injected into new sessions of the same project. Write a SEARCHABLE digest: key decisions, file names, error signatures, next steps — not full details. Deep history stays in SQLite (conversation / task_trace / distill); when the digest is not enough, locate details via memory_search / memory_session_context using the session id shown in each signature line. Session id is auto-appended by the system — do not repeat it. Write INCREMENTAL changes only (new phase, new decision, resolved blocker) — do not restate unchanged items. Max 2000 chars per entry. Suggested structure: ##Phase / ##File / ##Blocker / ##Decision.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "记忆内容（建议结构化格式：## phase / ## files / ## blockers / ## decisions）"}
                },
                "required": ["content"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let max_len = 2000usize;
                let trimmed = if content.len() > max_len {
                    let start = content.len() - max_len;
                    // Round down to a valid UTF-8 char boundary — raw byte slicing
                    // can land in the middle of a multi-byte character and panic.
                    let start = crate::utils::floor_char_boundary(content, start);
                    &content[start..]
                } else {
                    content
                };
                // ── 追加式项目记忆日志（memory/{tag}.md）──
                // 每次调用追加一条带署名条目；不覆盖他人/他轮内容。
                let sid_full = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("leader-current");
                let sid8: String = sid_full.chars().take(8).collect();
                let ts = chrono::Local::now();
                let sig = format!("[{} · {}]", ts.format("%m-%d %H:%M"), sid8);

                let path = crate::utils::active_memory_md_path();
                if let Some(parent) = path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return Ok(ToolResult::failure(format!("Failed to create dir: {}", e)));
                    }
                }
                // 追加：读旧 → 拼（署名行 + 锚点 frontmatter + 内容）→ 32KB 整条目容量
                // 裁剪（丢最旧）→ 写回。失败必须可见，不允许静默假成功。
                let existing = std::fs::read_to_string(&path).unwrap_or_default();
                // ── Project Map: 锚点 frontmatter 置于条目头部，内容只写一份 ──
                // （旧实现 entry 与 frontmatter 各拼一次全文 → 条目双份，tail 有效容量减半）
                let anchors = extract_memory_anchors(trimmed);
                let journal_entry =
                    format!("{}{}{trimmed}\n\n", sig, memory_anchors_frontmatter(&anchors));
                let capped = crate::utils::trim_memory_journal_to_cap(
                    &format!("{existing}{journal_entry}"),
                    crate::utils::MEMORY_JOURNAL_CAP_BYTES,
                );
                match std::fs::write(&path, &capped) {
                    Ok(_) => {
                        Ok(ToolResult::success(format!(
                            "Memory appended ({} chars)",
                            trimmed.len()
                        )))
                    }
                    Err(e) => Ok(ToolResult::failure(format!("Failed to write memory.md: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }

    /// WorkflowAgent 主动记忆写入工具
    ///
    /// 写入 ~/.nuphus/workflow-memory.md（用户数据目录，见 utils::nuphus_data_dir）。
    /// 会被注入到后续 Workflow 会话 prompt 中作为跨会话参考。
    /// 机制与 Leader memory journal 对齐：append 追加式（读旧→拼条目→cap 裁剪→写回），
    /// 条目含署名行（时间 + session 前 8 位）与摘要锚点 frontmatter；session id 由系统自动追加。
    /// 自动截断条目到 2000 字符。
    pub(crate) fn register_workflow_memory_update(&mut self) {
        self.register(ToolDef {
            name: "workflow_memory_update".to_string(),
            description: "Append a dated summary entry to the workflow working memory (.nuphus/workflow-memory.md), injected into new Workflow Agent sessions. Write a SEARCHABLE digest: phase progress, key findings, parameter experiments, blocking items — not full details; deep history stays in SQLite, reachable via memory_search / memory_session_context using the session id shown in each signature line. Session id is auto-appended by the system — do not repeat it. Write INCREMENTAL changes only (new phase, new decision, resolved blocker) — do not restate unchanged items. Max 2000 chars per entry. Suggested structure: ##Phase / ##File / ##Blocker / ##Decision.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "记忆内容（建议结构化格式：## phase / ## files / ## blockers / ## decisions）"},
                },
                "required": ["content"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let content = params.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let max_len = 2000usize;
                let trimmed = if content.len() > max_len {
                    let start = content.len() - max_len;
                    let start = crate::utils::floor_char_boundary(content, start);
                    &content[start..]
                } else {
                    content
                };
                // ── 对齐 Leader：append 追加式日志 + 署名行（时间 + session8）+ 摘要锚点 + cap ──
                let sid_full = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("workflow-current");
                let sid8: String = sid_full.chars().take(8).collect();
                let ts = chrono::Local::now();
                let sig = format!("[{} · {}]", ts.format("%m-%d %H:%M"), sid8);

                let nuphus_dir = crate::utils::nuphus_data_dir();
                let path = nuphus_dir.join("workflow-memory.md");
                if let Err(e) = std::fs::create_dir_all(&nuphus_dir) {
                    return Ok(ToolResult::failure(format!("Failed to create dir: {}", e)));
                }
                let existing = std::fs::read_to_string(&path).unwrap_or_default();
                let anchors = extract_memory_anchors(trimmed);
                let journal_entry =
                    format!("{}{}{trimmed}\n\n", sig, memory_anchors_frontmatter(&anchors));
                let capped = crate::utils::trim_memory_journal_to_cap(
                    &format!("{existing}{journal_entry}"),
                    crate::utils::MEMORY_JOURNAL_CAP_BYTES,
                );
                match std::fs::write(&path, &capped) {
                    Ok(_) => {
                        Ok(ToolResult::success(format!(
                            "Workflow 记忆已追加 ({} chars)",
                            trimmed.len()
                        )))
                    }
                    Err(e) => Ok(ToolResult::failure(format!("写入失败: {}", e))),
                }
            },
            depends_on: vec![],
        });
    }

    /// 按 session_id 查看会话因果链上下文（最多返回 limit 条）
    pub(crate) fn register_session_context(&mut self) {
        self.register(ToolDef {
            name: "memory_session_context".to_string(),
            description: "查看会话因果链。session_id：该会话最近 N 条；session_id+turn_id：该轮全部条目（Leader 意图与对话、Exec/Workflow 任务轨迹的紧凑步骤：工具 + 参数摘要 + 结果摘要 + 成败）。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": {"type": "string", "description": "会话 ID（从 memory_search 结果中获取的完整 session_id）"},
                    "turn_id": {"type": "string", "description": "可选。精确到某轮次，查看该轮次的所有条目（Leader 意图、Exec 结果、StateChecker 模式等）"},
                    "limit": {"type": "integer", "default": 20, "description": "不加 turn_id 时最多返回多少条"}
                },
                "required": ["session_id"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                use crate::store::memory;
                let session_id = match params.get("session_id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return Ok(ToolResult::failure("missing 'session_id' parameter")),
                };
                let turn_id = params.get("turn_id").and_then(|v| v.as_str());
                let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

                let results = if let Some(tid) = turn_id {
                    // ── 精确到轮次：查该 session + turn 的所有条目（ASC 时间顺序） ──
                    let mut all = memory::search_entries_filtered(
                        None, Some(session_id), None, None, None, None, None, None, None, None, None, 500, true,
                    ).map_err(|e| format!("session query failed: {}", e))?;
                    all.retain(|e| e.turn_id == tid);
                    all
                } else {
                    memory::search_entries_filtered(
                        None, Some(session_id), None, None, None, None, None, None, None, None, None, limit, true,
                    ).map_err(|e| format!("session query failed: {}", e))?
                };

                if results.is_empty() {
                    let suffix = turn_id.map(|t| format!(" turn={}", t)).unwrap_or_default();
                    return Ok(ToolResult::success(format!("No entries found for session {}{}", session_id, suffix)));
                }
                // ── 展平 leader 条目中的 user_message（包含完整对话），按 turn 分组展示 ──
                //    用 BTreeMap 保持 turn_id 排序
                let mut by_turn: std::collections::BTreeMap<String, Vec<&MemoryEntry>> = std::collections::BTreeMap::new();
                for e in &results {
                    by_turn.entry(e.turn_id.clone()).or_default().push(e);
                }
                let mut blocks: Vec<String> = Vec::new();
                for (tid, entries) in &by_turn {
                    blocks.push(format!("=== Turn {} ({} entries) ===", tid, entries.len()));
                    for e in entries {
                        let ts = e.created_at.parse::<chrono::DateTime<chrono::Utc>>().ok()
                            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default();
                        let status = if e.success { "✓" } else { "✗" };
                        let tag_str = if e.tags.is_empty() { String::new() } else {
                            format!(" [#{}]", e.tags.join(" #"))
                        };
                        let goal = e.goal_type.as_deref().map(|g| format!(" goal={}", g)).unwrap_or_default();
                        blocks.push(format!("--- [{}] {}/{} ({}){}{} ---", ts, e.agent_type.as_str(), e.kind.as_str(), status, goal, tag_str));
                        if !e.user_message.is_empty() {
                            blocks.push(format!("USER:\n{}", e.user_message));
                        }
                        if !e.assistant_message.is_empty() {
                            blocks.push(format!("ASSISTANT:\n{}", e.assistant_message));
                        }
                        if let Some(ref out) = e.output {
                            if !out.is_empty() {
                                blocks.push(format!("OUTPUT:\n{}", out));
                            }
                        }
                        if let Some(ref pat) = e.pattern {
                            if !pat.is_empty() {
                                blocks.push(format!("PATTERN: {}", pat));
                            }
                        }
                        // ── 精确到轮次时展示紧凑步骤轨迹（tool + params摘要 → result摘要 + 成败）──
                        if turn_id.is_some() && !e.execution_steps.is_empty() {
                            blocks.push(format!("STEPS ({} 步):", e.execution_steps.len()));
                            for (i, s) in e.execution_steps.iter().enumerate() {
                                let step_status = if s.success { "✓" } else { "✗" };
                                let mut line = format!("  {}. {} {}", i + 1, s.tool, step_status);
                                if !s.params_summary.is_empty() {
                                    line.push_str(&format!(" | params: {}", s.params_summary));
                                }
                                if !s.result_summary.is_empty() {
                                    line.push_str(&format!(" → {}", s.result_summary));
                                }
                                blocks.push(line);
                            }
                        }
                    }
                    blocks.push(String::new());
                }
                let turn_count = by_turn.len();
                Ok(ToolResult::success(format!("Session {} ({} turns, {} entr{}):\n{}",
                    session_id,
                    turn_count,
                    results.len(),
                    if results.len() == 1 { "y" } else { "ies" },
                    blocks.join("\n")
                )))
            },
            depends_on: vec![],
        });
    }
}

// ── Project Map: memory.md frontmatter helpers ──

/// 从 memory.md 正文中提取文件路径锚点
fn extract_memory_anchors(content: &str) -> Vec<String> {
    let mut anchors = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // 跳过已有 frontmatter
        if trimmed == "---" || trimmed.starts_with("anchors:") {
            continue;
        }
        // 匹配 `- path/to/file.rs` 或 `- "file://..."` 格式
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let path = rest.trim().trim_matches('"');
            if path.contains('.') && !path.contains(' ') {
                // 合理的文件路径：有扩展名、无空格
                let normalized = path.replace('\\', "/");
                let anchor = if normalized.starts_with("file:") {
                    normalized
                } else {
                    format!("file:{}", normalized)
                };
                if !anchors.contains(&anchor) {
                    anchors.push(anchor);
                }
            }
        }
    }
    anchors
}

/// 锚点 YAML frontmatter 块（无锚点时返回空串）。插在条目头部，内容不重复。
fn memory_anchors_frontmatter(anchors: &[String]) -> String {
    if anchors.is_empty() {
        return String::new();
    }
    let mut fm = String::from("---\nanchors:\n");
    for a in anchors {
        fm.push_str(&format!("  - \"{}\"\n", a));
    }
    fm.push_str("---\n\n");
    fm
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 诊断输出必须包含逐词命中数与两条行动引导，且不返回任何结果条目
    #[test]
    fn test_zero_result_diagnostic_format() {
        let counts = vec![
            ("cookievault".to_string(), 2usize),
            ("语音".to_string(), 0usize),
        ];
        let msg = format_zero_result_diagnostic(&counts);
        assert!(msg.contains("cookievault=2"), "msg: {}", msg);
        assert!(msg.contains("语音=0"), "msg: {}", msg);
        assert!(msg.contains("严格 AND"), "msg: {}", msg);
        assert!(msg.contains("semantic=true"), "msg: {}", msg);
        assert!(msg.contains("减少关键词"), "msg: {}", msg);
    }

    /// 空关键词（分词为空）时诊断不应 panic，且仍给出引导
    #[test]
    fn test_zero_result_diagnostic_empty_counts() {
        let msg = format_zero_result_diagnostic(&[]);
        assert!(msg.contains("semantic=true"), "msg: {}", msg);
    }
}