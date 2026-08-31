//! 记忆条目 SQLite 存储（FTS5 全文检索）
//!
//! Nuphus 唯一记忆存储层。所有读写操作均经过此模块。

use crate::memory::entry::{AgentType, MemoryEntry, MemoryKind};
use crate::segmenter::segment_for_fts;
use instant_distance::{Builder, HnswMap, Point, Search};
use rusqlite::params;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// memory_entries 全列（row_to_entry 的列序契约，所有查询必须复用）
const ENTRY_COLS: &str = "id, session_id, turn_id, sequence, created_at, wall_clock_ms,
        agent_type, kind, task_chain_id, chain_step, goal_type, tags,
        intent, summary, user_message, assistant_message, tools_used,
        success, output, artifacts, is_marked, execution_steps,
        parent_id, children_ids, pattern, custom_agent_id";

/// 带表别名前缀的列列表（JOIN 查询用）
fn entry_cols(prefix: &str) -> String {
    ENTRY_COLS
        .split(',')
        .map(|c| format!("{}.{}", prefix, c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 将 MemoryEntry 写入 SQLite
///
/// 单事务写入：主表 INSERT + FTS 同步强一致（任一失败整体回滚）；
/// embedding 先算好再开事务（缩短写锁时间），upsert 失败仅降级告警不阻断。
pub fn insert_entry(entry: &MemoryEntry) -> crate::Result<()> {
    // ── Custom 记忆隔离：写入时从全局状态填充归属卡片 id（若当前是 Custom 会话）──
    let mut entry = entry.clone();
    if entry.custom_agent_id.is_none() {
        entry.custom_agent_id = crate::custom_agents::current_custom_agent_id();
    }
    let entry = &entry;

    // ── 先计算 embedding（模型推理可能耗时），再开事务 ──
    // 计算失败降级为无向量（不影响主表+FTS 一致性）
    let embedding = crate::embed::Embedder::get().and_then(|embedder| {
        let text = format!(
            "{} {} {}",
            entry.intent,
            entry.summary,
            entry.output.as_deref().unwrap_or("")
        );
        embedder.embed_passage(&text).ok()
    });

    let mut guard = crate::store::db::acquire()?;

    let tags_str = entry.tags.join(",");
    let tools_used_str = entry.tools_used.join(",");
    let artifacts_str = entry.artifacts.join(",");
    let children_ids_str = entry.children_ids.join(",");
    let exec_steps_str = serde_json::to_string(&entry.execution_steps).unwrap_or_default();

    // ── 项目归属登记（惰性）：session → 项目 tag，供记忆检索默认项目过滤。
    // 失败仅告警，不阻断主写入。
    if let Some(tag) = crate::utils::active_project_tag() {
        if let Err(e) = guard.execute(
            "INSERT OR IGNORE INTO session_meta (session_id, project_tag, created_at) VALUES (?1, ?2, ?3)",
            params![entry.session_id, tag, entry.created_at],
        ) {
            tracing::warn!("[memory] session_meta register failed: {e}");
        }
    }

    let tx = guard.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO memory_entries
         (id, session_id, turn_id, sequence, created_at, wall_clock_ms,
          agent_type, kind, task_chain_id, chain_step, goal_type, tags,
          intent, summary, user_message, assistant_message, tools_used,
          success, output, artifacts, is_marked, execution_steps,
          parent_id, children_ids, pattern, custom_agent_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                 ?24, ?25, ?26)",
        params![
            entry.id,
            entry.session_id,
            entry.turn_id,
            entry.sequence,
            entry.created_at,
            entry.wall_clock_ms,
            entry.agent_type.to_string(),
            entry.kind.as_str(),
            entry.task_chain_id,
            entry.chain_step,
            entry.goal_type,
            tags_str,
            entry.intent,
            entry.summary,
            entry.user_message,
            entry.assistant_message,
            tools_used_str,
            entry.success as i32,
            entry.output,
            artifacts_str,
            entry.is_marked as i32,
            exec_steps_str,
            entry.parent_id,
            children_ids_str,
            entry.pattern,
            entry.custom_agent_id,
        ],
    )?;

    // FTS 同步失败必须回滚（主表与索引不允许不一致）
    sync_entry_fts(
        &tx,
        &entry.id,
        entry.kind,
        &entry.intent,
        &entry.summary,
        &tags_str,
        entry.pattern.as_deref().unwrap_or(""),
        entry.output.as_deref().unwrap_or(""),
    )?;

    // embedding upsert 失败仅降级（主表+FTS 已强一致，向量可后补）
    let mut embedding_written = false;
    if let Some(ref vec) = embedding {
        match upsert_embedding(&tx, &entry.id, vec) {
            Ok(_) => embedding_written = true,
            Err(e) => {
                tracing::warn!(
                    "[MEMORY] embedding upsert failed for {} (degraded): {}",
                    entry.id,
                    e
                )
            }
        }
    }
    tx.commit()?;

    if embedding_written {
        invalidate_embedding_cache();
    }
    drop(guard);
    let _ = prune_entries();

    Ok(())
}

/// 将指定条目的文本同步到 FTS5 v4 索引（分词后插入，含 kind 列）
#[allow(clippy::too_many_arguments)]
fn sync_entry_fts(
    conn: &rusqlite::Connection,
    id: &str,
    kind: MemoryKind,
    intent: &str,
    summary: &str,
    tags: &str,
    pattern: &str,
    output: &str,
) -> crate::Result<()> {
    // 删除旧索引
    conn.execute("DELETE FROM memory_fts_v4 WHERE id = ?1", params![id])?;
    // 插入分词后的新索引（kind 为枚举 token，不分词）
    let fts_intent = segment_for_fts(intent);
    let fts_summary = segment_for_fts(summary);
    let fts_tags = segment_for_fts(tags);
    let fts_pattern = segment_for_fts(pattern);
    let fts_output = segment_for_fts(output);
    conn.execute(
        "INSERT INTO memory_fts_v4(id, kind, intent, summary, tags, pattern, output) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, kind.as_str(), fts_intent, fts_summary, fts_tags, fts_pattern, fts_output],
    )?;
    Ok(())
}

/// 一次性数据迁移：回填 conversation 条目的 intent/summary 索引字段。
/// 历史 bug：persist_leader_turn 写入时 intent/summary 留空，而 FTS 索引只覆盖
/// intent/summary/tags/pattern/output（不含 user_message/assistant_message），
/// 导致对话全文已存但任何内容词都检索不到（embedding 文本同样为空）。
/// 幂等：只处理 intent 和 summary 均为空的 conversation 条目，可重复调用。
/// 注：存量条目的 embedding 不回补（需模型推理，代价高），语义检索对旧对话仍弱，
/// FTS 回填后关键词检索即可恢复。
pub fn backfill_conversation_index_fields() -> crate::Result<usize> {
    let mut guard = crate::store::db::acquire()?;

    let rows: Vec<(String, String, String, String, Option<String>)> = {
        let mut stmt = guard.prepare(
            "SELECT id, user_message, assistant_message, tags, output
             FROM memory_entries
             WHERE kind = 'conversation' AND intent = '' AND summary = ''",
        )?;
        let collected = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        collected
    };

    if rows.is_empty() {
        return Ok(0);
    }

    let tx = guard.transaction()?;
    let mut migrated = 0usize;
    for (id, user_msg, asst_msg, tags, output) in &rows {
        let intent: String = user_msg.chars().take(100).collect();
        let summary: String = asst_msg.chars().take(300).collect();
        tx.execute(
            "UPDATE memory_entries SET intent = ?2, summary = ?3 WHERE id = ?1",
            params![id, intent, summary],
        )?;
        sync_entry_fts(
            &tx,
            id,
            MemoryKind::Conversation,
            &intent,
            &summary,
            tags,
            "",
            output.as_deref().unwrap_or(""),
        )?;
        migrated += 1;
    }
    tx.commit()?;

    tracing::info!(
        "[MEMORY] backfill_conversation_index_fields: migrated {} conversation entries",
        migrated
    );
    Ok(migrated)
}

/// 根据 ID 获取记忆条目
pub fn get_entry_by_id(id: &str) -> crate::Result<Option<MemoryEntry>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(&format!(
        "SELECT {} FROM memory_entries WHERE id = ?1",
        ENTRY_COLS
    ))?;

    let mut rows = stmt.query_map(params![id], row_to_entry)?;
    match rows.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        _ => Ok(None),
    }
}

/// 构建 FTS5 MATCH 查询语句：分词后逐 token 加前缀匹配标记，空格连接保持隐式 AND
///
/// "网络问题" → segment → "网络 问题" → MATCH "\"网络\" * \"问题\" *"
/// 前缀语法 `"tok" *` 经真实 sqlite 单测实证可用（FTS5 文档语法）：
/// "单例" 可命中索引词 "单例化"，根治词边界误杀；多 token 隐式 AND 语义不变。
fn build_fts_query(query: &str) -> String {
    let tokens = crate::segmenter::segment(query);
    if tokens.is_empty() {
        return String::new();
    }
    tokens
        .iter()
        .map(|t| format!("\"{}\" *", t))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 逐词命中计数（严格 AND 零结果时的诊断数据）
///
/// 对每个分词 token 独立执行 FTS5 前缀 MATCH 计数，告知调用方哪个词拖垮了查询。
/// 只提供诊断数据，绝不降级返回部分匹配结果集。
pub fn token_hit_counts(query: &str) -> crate::Result<Vec<(String, usize)>> {
    let guard = crate::store::db::acquire()?;
    token_hit_counts_on(&guard, query)
}

/// token_hit_counts 的核心实现（注入连接，便于内存 DB 单测）
fn token_hit_counts_on(
    conn: &rusqlite::Connection,
    query: &str,
) -> crate::Result<Vec<(String, usize)>> {
    let tokens = crate::segmenter::segment(query);
    let mut stmt =
        conn.prepare("SELECT count(*) FROM memory_fts_v4 WHERE memory_fts_v4 MATCH ?1")?;
    let mut counts = Vec::with_capacity(tokens.len());
    for token in tokens {
        let fts = format!("\"{}\" *", token);
        let n: i64 = stmt.query_row(params![fts], |r| r.get(0))?;
        counts.push((token, n as usize));
    }
    Ok(counts)
}

/// FTS5 全文搜索记忆条目（带 BM25 评分），可选 kind 过滤
/// exclude_task_trace：kind 未指定时在 SQL 层排除 task_trace（执行轨迹噪声大），
/// 避免 Rust 后过滤时 fetch 窗口被噪声占据导致有效结果不足 limit。
pub fn search_entries_scored(
    query: &str,
    limit: usize,
    kind: Option<MemoryKind>,
    exclude_task_trace: bool,
    all_projects: bool,
) -> crate::Result<Vec<(MemoryEntry, f32)>> {
    let guard = crate::store::db::acquire()?;

    let fts_query = build_fts_query(query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    let custom_id = crate::custom_agents::current_custom_agent_id();
    // 匿名 ? 按文本顺序绑定：MATCH → kind → custom → LIMIT。
    // Custom 记忆分层（2026-08-31）：Custom 会话可检索「本卡片私有 + 项目公共（未打标）」
    // 记忆——上下文过渡靠记忆承载，不能隔离到空白；非 Custom（Leader/Workflow）仍排除
    // 所有 Custom 记忆（IS NULL），Custom 私有沉淀不污染公共检索。
    let mut sql = format!(
        "SELECT {}, bm25(memory_fts_v4) as score
         FROM memory_entries e
         JOIN memory_fts_v4 fts ON e.id = fts.id
         WHERE memory_fts_v4 MATCH ?",
        entry_cols("e")
    );
    if kind.is_some() {
        sql.push_str(" AND e.kind = ?");
    } else if exclude_task_trace {
        // 字面量常量，不参与参数绑定，不影响 ? 顺序
        sql.push_str(" AND e.kind != 'task_trace'");
    }
    match &custom_id {
        Some(_) => sql.push_str(" AND (e.custom_agent_id = ? OR e.custom_agent_id IS NULL)"),
        None => sql.push_str(" AND e.custom_agent_id IS NULL"),
    }
    // ── 项目记忆隔离：默认只看当前项目的 session（session_meta 登记）；
    // all_projects=true 或未配置项目目录时不过滤。? 顺序：fts → kind → custom → project → LIMIT。
    let project_tag = if all_projects {
        None
    } else {
        crate::utils::active_project_tag()
    };
    if project_tag.is_some() {
        sql.push_str(
            " AND e.session_id IN (SELECT session_id FROM session_meta WHERE project_tag = ?)",
        );
    }
    sql.push_str(" ORDER BY score LIMIT ?");

    let limit_i = limit as i64;
    let kind_str = kind.map(|k| k.as_str());
    let mut stmt = guard.prepare(&sql)?;
    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<(MemoryEntry, f32)> {
        let entry = row_to_entry(row)?;
        let score: f32 = row.get(26)?;
        Ok((entry, score))
    };
    // slice 顺序严格对应 SQL 文本中 ? 出现顺序
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(fts_query)];
    if let Some(k) = kind_str {
        param_values.push(Box::new(k.to_string()));
    }
    if let Some(ref cid) = custom_id {
        param_values.push(Box::new(cid.clone()));
    }
    if let Some(tag) = project_tag {
        param_values.push(Box::new(tag));
    }
    param_values.push(Box::new(limit_i));
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|b| b.as_ref()).collect();
    let entries = stmt
        .query_map(params_ref.as_slice(), map_row)?
        .filter_map(|r| match r {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("[memory] search_entries_scored: row parse failed — {}", e);
                None
            }
        })
        .collect();
    Ok(entries)
}

/// 按条件搜索记忆条目（结构化 + 全文混合），可选 kind 过滤
#[allow(clippy::too_many_arguments)]
pub fn search_entries_filtered(
    query: Option<&str>,
    session_id: Option<&str>,
    goal_type: Option<&str>,
    agent_type: Option<&str>,
    kind: Option<MemoryKind>,
    tags: Option<&[String]>,
    success: Option<bool>,
    time_window_ms: Option<u64>,
    is_marked: Option<bool>,
    exclude_goal_types: Option<&[String]>,
    search_text: Option<&str>,
    limit: usize,
    all_projects: bool,
) -> crate::Result<Vec<MemoryEntry>> {
    let guard = crate::store::db::acquire()?;

    let mut conditions: Vec<String> = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    // ── FTS5 查询：单独处理，JOIN 时用 BM25 排序 ──
    let fts_query: Option<String> = if let Some(q) = query {
        let fts = build_fts_query(q);
        if fts.is_empty() {
            None
        } else {
            Some(fts)
        }
    } else {
        None
    };

    // ── Custom 记忆分层（最高优先级过滤，2026-08-31）──
    // Custom 会话 → 本卡片私有（custom_agent_id = cid）+ 项目公共（IS NULL）皆可检索，
    // 上下文过渡靠记忆承载；非 Custom（Leader/Workflow）→ 排除所有 Custom 记忆（IS NULL），
    // Custom 私有沉淀不污染公共检索。
    // 全局共享层（Soul/Tenet/Knowledge）走 prompt 注入，不经此检索，天然不受影响。
    match crate::custom_agents::current_custom_agent_id() {
        Some(cid) => {
            conditions.push("(e.custom_agent_id = ? OR e.custom_agent_id IS NULL)".to_string());
            param_values.push(Box::new(cid));
        }
        None => {
            conditions.push("e.custom_agent_id IS NULL".to_string());
        }
    }

    if let Some(gt) = goal_type {
        conditions.push("e.goal_type = ?".to_string());
        param_values.push(Box::new(gt.to_string()));
    }
    // ── 项目记忆隔离：默认只看当前项目的 session（session_meta 登记）。
    // 豁免：all_projects=true（显式全局）、显式传 session_id（精确定位键跨项目合法——
    // 与 memory_session_context 语义对齐，md 索引引导的跨项目 sid 精查不被卡死）、
    // 未配置项目目录。无 meta 的历史 session 不命中 IN 子查询 = 默认不可见（查全局才可见）。
    if !all_projects && session_id.is_none() {
        if let Some(tag) = crate::utils::active_project_tag() {
            conditions.push(
                "e.session_id IN (SELECT session_id FROM session_meta WHERE project_tag = ?)"
                    .to_string(),
            );
            param_values.push(Box::new(tag));
        }
    }
    if let Some(sid) = session_id {
        conditions.push("e.session_id = ?".to_string());
        param_values.push(Box::new(sid.to_string()));
    }
    if let Some(at) = agent_type {
        conditions.push("e.agent_type = ?".to_string());
        param_values.push(Box::new(at.to_string()));
    }
    if let Some(k) = kind {
        conditions.push("e.kind = ?".to_string());
        param_values.push(Box::new(k.as_str().to_string()));
    }
    if let Some(s) = success {
        conditions.push("e.success = ?".to_string());
        param_values.push(Box::new(s as i32));
    }
    if let Some(tags_filter) = tags {
        for tag in tags_filter {
            // 用 LIKE 匹配逗号分隔的 tags 字段
            // 注意：必须使用匿名 ?（非 ?N），否则与 fts_query 的 MATCH ? 索引冲突
            conditions.push("',' || e.tags || ',' LIKE ?".to_string());
            param_values.push(Box::new(format!("%,{}%", tag)));
        }
    }
    if let Some(tw) = time_window_ms {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let cutoff = now.saturating_sub(tw);
        conditions.push("e.wall_clock_ms >= ?".to_string());
        param_values.push(Box::new(cutoff as i64));
    }

    // ── is_marked: 标记条目或提炼（distill 视为用户可感知的高价值内容）──
    if let Some(marked) = is_marked {
        if marked {
            conditions.push("(e.is_marked = 1 OR e.kind = 'distill')".to_string());
        }
    }

    // ── exclude_goal_types: 排除指定 goal_type ──
    if let Some(types) = exclude_goal_types {
        if !types.is_empty() {
            let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
            conditions.push(format!("e.goal_type NOT IN ({})", placeholders.join(", ")));
            for gt in types {
                param_values.push(Box::new(gt.clone()));
            }
        }
    }

    // ── search_text: LIKE 模糊搜索 ──
    if let Some(text) = search_text {
        let keyword = text.to_lowercase();
        conditions.push(
            "(LOWER(e.intent) LIKE ? OR LOWER(e.summary) LIKE ? OR LOWER(e.tags) LIKE ?)"
                .to_string(),
        );
        let like_val = format!("%{}%", keyword);
        param_values.push(Box::new(like_val.clone()));
        param_values.push(Box::new(like_val.clone()));
        param_values.push(Box::new(like_val));
    }

    // ── 构造 SQL：有 FTS 查询时 JOIN FTS 表用 BM25 排序，snapshot 优先 ──
    let conditions_str = if conditions.is_empty() {
        String::new()
    } else {
        conditions.join(" AND ")
    };

    let select_cols = entry_cols("e");

    let sql = if fts_query.is_some() {
        // FTS5 JOIN + BM25 相关性排序，snapshot（工作记忆快照）加权优先
        let extra_where = if conditions_str.is_empty() {
            String::new()
        } else {
            format!(" AND {}", conditions_str)
        };
        format!(
            "SELECT {}\n         FROM memory_entries e\n         JOIN memory_fts_v4 fts ON e.id = fts.id\n         WHERE memory_fts_v4 MATCH ?{}\n         ORDER BY CASE e.kind WHEN 'snapshot' THEN 0 WHEN 'distill' THEN 1 ELSE 2 END, e.wall_clock_ms DESC, bm25(memory_fts_v4) ASC\n         LIMIT ?",
            select_cols, extra_where
        )
    } else {
        // 无 FTS 查询：时间排序
        let where_clause = if conditions_str.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions_str)
        };
        format!(
            "SELECT {}\n         FROM memory_entries e\n         {}\n         ORDER BY e.wall_clock_ms DESC\n         LIMIT ?",
            select_cols, where_clause
        )
    };

    let mut stmt = guard.prepare(&sql)?;

    // ── 构建完整参数列表，统一通过 query_map 传参 ──
    // raw_bind_parameter 会在 query_map 调用时被 sqlite3_reset 清空，
    // 所以必须把所有参数打包传给 query_map 一次性绑定。
    let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref fts_q) = fts_query {
        all_params.push(Box::new(fts_q.clone()));
    }
    all_params.append(&mut param_values);
    all_params.push(Box::new(limit as i64));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        all_params.iter().map(|p| p.as_ref()).collect();

    let entries = stmt
        .query_map(param_refs.as_slice(), row_to_entry)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

/// 更新条目
pub fn update_entry<F>(id: &str, modifier: F) -> crate::Result<Option<MemoryEntry>>
where
    F: FnOnce(&mut MemoryEntry),
{
    let existing = get_entry_by_id(id)?;
    match existing {
        Some(mut entry) => {
            modifier(&mut entry);
            insert_entry(&entry)?;
            Ok(Some(entry))
        }
        None => Ok(None),
    }
}

/// 获取最近的条目。可选 kind 过滤（包含）；exclude_kinds 用于排除高噪声类别
///（如 memory_recent 默认排除 task_trace）。
pub fn recent_entries(
    limit: usize,
    kind: Option<MemoryKind>,
    exclude_kinds: &[MemoryKind],
    all_projects: bool,
) -> crate::Result<Vec<MemoryEntry>> {
    let guard = crate::store::db::acquire()?;

    let mut where_parts: Vec<String> = Vec::new();
    // 编号占位符必须连续（?1 = LIMIT，其后按序递增）。留空洞（如 kind 缺席时 ?2 缺失）
    // 会让 rusqlite 按最大编号计数要求更多绑定参数 → "Got N, needed N+1" 运行时报错。
    let mut next_idx = 2usize;
    if kind.is_some() {
        where_parts.push(format!("kind = ?{next_idx}"));
        next_idx += 1;
    }
    for _ in exclude_kinds.iter() {
        where_parts.push(format!("kind != ?{next_idx}"));
        next_idx += 1;
    }
    // ── 项目记忆隔离：默认只看当前项目的 session；无 meta 的历史 session 不命中，查全局才可见。
    let project_tag = if all_projects {
        None
    } else {
        crate::utils::active_project_tag()
    };
    if project_tag.is_some() {
        where_parts.push(format!(
            "session_id IN (SELECT session_id FROM session_meta WHERE project_tag = ?{next_idx})"
        ));
    }
    let where_clause = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };
    let sql = format!(
        "SELECT {} FROM memory_entries {} ORDER BY wall_clock_ms DESC LIMIT ?1",
        ENTRY_COLS, where_clause
    );

    let limit_i = limit as i64;
    let kind_str = kind.map(|k| k.as_str().to_string());
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(limit_i)];
    if let Some(ref k) = kind_str {
        param_values.push(Box::new(k.clone()));
    }
    for k in exclude_kinds {
        param_values.push(Box::new(k.as_str().to_string()));
    }
    if let Some(tag) = project_tag {
        param_values.push(Box::new(tag));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    let mut stmt = guard.prepare(&sql)?;
    let entries = stmt
        .query_map(param_refs.as_slice(), row_to_entry)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

/// 按 session_id 查询所有条目（不再受全局 5000 条限制）
pub fn get_entries_by_session(session_id: &str) -> crate::Result<Vec<MemoryEntry>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(&format!(
        "SELECT {} FROM memory_entries WHERE session_id = ?1 ORDER BY wall_clock_ms ASC",
        ENTRY_COLS
    ))?;

    let entries = stmt
        .query_map(params![session_id], row_to_entry)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

/// 记忆条目统计信息（等价于旧 IndexStats）
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct EntryStats {
    pub total_entries: u64,
    pub by_agent_leader: u64,
    pub by_agent_exec: u64,
    pub by_success: u64,
    pub by_failure: u64,
    /// 一维 kind 分布（conversation/task_trace/distill/pattern/snapshot）
    pub by_kind: std::collections::HashMap<String, u64>,
    pub by_task_chain: u64,
    pub oldest_ms: u64,
    pub newest_ms: u64,
}

/// 最大记忆条目数，超出后自动裁剪
const MAX_ENTRIES: usize = 10_000;

/// 根据 ID 删除记忆条目（物理删除）
pub fn delete_entry(id: &str) -> crate::Result<()> {
    let guard = crate::store::db::acquire()?;

    guard.execute("DELETE FROM memory_entries WHERE id = ?1", params![id])?;
    guard.execute("DELETE FROM memory_fts_v4 WHERE id = ?1", params![id])?;
    Ok(())
}

/// 自动裁剪：超出 MAX_ENTRIES 时删除最老的低价值条目
/// 候选仅限 task_trace（conversation/distill/pattern/snapshot 永久保留），
/// is_marked=1 豁免；最老优先删除。
pub fn prune_entries() -> crate::Result<usize> {
    let guard = crate::store::db::acquire()?;

    let count: i64 = guard.query_row("SELECT COUNT(*) FROM memory_entries", [], |r| r.get(0))?;
    if count as usize <= MAX_ENTRIES {
        return Ok(0);
    }
    let to_remove = count as usize - MAX_ENTRIES;

    // 选出待删除的 ID：最老的未标记 task_trace
    let mut stmt = guard.prepare(
        "SELECT id FROM memory_entries
         WHERE kind = 'task_trace' AND is_marked = 0
         ORDER BY wall_clock_ms ASC
         LIMIT ?1",
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![to_remove as i64], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    if ids.is_empty() {
        return Ok(0);
    }

    let removed = ids.len();
    for id in &ids {
        guard.execute("DELETE FROM memory_fts_v4 WHERE id = ?1", params![id])?;
    }
    // 用 IN 批量删除（SQLite 最多支持 999 个占位符）
    for chunk in ids.chunks(900) {
        let placeholders: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "DELETE FROM memory_entries WHERE id IN ({})",
            placeholders.join(",")
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = chunk
            .iter()
            .map(|id| id as &dyn rusqlite::types::ToSql)
            .collect();
        guard.execute(&sql, params.as_slice())?;

        // 同步删除 embedding 向量
        let emb_sql = format!(
            "DELETE FROM memory_embeddings WHERE id IN ({})",
            placeholders.join(",")
        );
        guard.execute(&emb_sql, params.as_slice())?;
    }

    // 失效 embedding 缓存（向量已删除）
    invalidate_embedding_cache();

    tracing::info!(
        "prune_entries: removed {} entries (total was {}, max {})",
        removed,
        count,
        MAX_ENTRIES
    );
    Ok(removed)
}

/// 聚合查询记忆统计
pub fn entry_stats() -> crate::Result<EntryStats> {
    let guard = crate::store::db::acquire()?;

    let total_entries: i64 =
        guard.query_row("SELECT COUNT(*) FROM memory_entries", [], |r| r.get(0))?;
    let by_agent_leader: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memory_entries WHERE agent_type = 'leader'",
        [],
        |r| r.get(0),
    )?;
    let by_agent_exec: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memory_entries WHERE agent_type = 'exec'",
        [],
        |r| r.get(0),
    )?;
    let by_success: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memory_entries WHERE success = 1",
        [],
        |r| r.get(0),
    )?;
    let by_failure: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memory_entries WHERE success = 0",
        [],
        |r| r.get(0),
    )?;
    let by_task_chain: i64 = guard.query_row(
        "SELECT COUNT(*) FROM memory_entries WHERE task_chain_id IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let oldest_ms: i64 = guard.query_row(
        "SELECT COALESCE(MIN(wall_clock_ms), 0) FROM memory_entries",
        [],
        |r| r.get(0),
    )?;
    let newest_ms: i64 = guard.query_row(
        "SELECT COALESCE(MAX(wall_clock_ms), 0) FROM memory_entries",
        [],
        |r| r.get(0),
    )?;

    // 按 kind 分组统计（一维分类分布）
    let mut stmt =
        guard.prepare("SELECT kind, COUNT(*) as cnt FROM memory_entries GROUP BY kind")?;
    let by_kind: std::collections::HashMap<String, u64> = stmt
        .query_map([], |row| {
            let k: String = row.get(0)?;
            let cnt: i64 = row.get(1)?;
            Ok((k, cnt as u64))
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(EntryStats {
        total_entries: total_entries as u64,
        by_agent_leader: by_agent_leader as u64,
        by_agent_exec: by_agent_exec as u64,
        by_success: by_success as u64,
        by_failure: by_failure as u64,
        by_kind,
        by_task_chain: by_task_chain as u64,
        oldest_ms: oldest_ms as u64,
        newest_ms: newest_ms as u64,
    })
}

/// Session summary (for history list display)
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub user_message: String,
    pub intent: String,
    pub last_assistant_message: String,
    pub entry_count: u32,
    pub tool_call_count: u32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub success: bool,
    pub tags: Vec<String>,
}

// 将 SQLite 行映射为 MemoryEntry

// ══════════════════════════════════════════════════════════════════════════
// 向量 Embedding 存储与语义搜索
// ══════════════════════════════════════════════════════════════════════════

/// 写入或更新向量 Embedding（覆盖已有）
fn upsert_embedding(conn: &rusqlite::Connection, id: &str, embedding: &[f32]) -> crate::Result<()> {
    let blob: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
    conn.execute(
        "INSERT OR REPLACE INTO memory_embeddings(id, embedding) VALUES (?1, ?2)",
        params![id, blob],
    )?;
    Ok(())
}

/// 语义搜索：用 HNSW 近似最近邻检索替代全量扫描，返回 Top-N，可选 kind 过滤
pub fn search_entries_semantic(
    query: &str,
    limit: usize,
    kind: Option<MemoryKind>,
    all_projects: bool,
) -> crate::Result<Vec<(MemoryEntry, f32)>> {
    let embedder = crate::embed::Embedder::get()
        .ok_or_else(|| crate::NuphusError::store("Embedder 未初始化".to_string()))?;

    let query_vec = embedder
        .embed(query)
        .map_err(|e| crate::NuphusError::store(format!("Query embedding failed: {}", e)))?;
    let query_point = EmbeddingPoint(query_vec);

    // 确保 HNSW 索引已加载
    ensure_hnsw_loaded()
        .ok_or_else(|| crate::NuphusError::store("Embedding 索引为空".to_string()))?;

    // 读锁下搜索
    let cache = embedding_cache()
        .read()
        .map_err(|e| crate::NuphusError::store(format!("embedding cache read lock: {}", e)))?;
    let map = cache
        .as_ref()
        .ok_or_else(|| crate::NuphusError::store("Embedding 索引未构建".to_string()))?;

    let mut search = Search::default();
    // 向量均为 L2 归一化，L2 距离可转换为余弦相似度: cos = 1 - L2²/2
    let scored: Vec<(String, f32)> = map
        .search(&query_point, &mut search)
        .map(|item| {
            let cos = 1.0 - item.distance * item.distance / 2.0;
            (item.value.clone(), cos)
        })
        .collect();
    let _ = map;
    drop(cache);

    if scored.is_empty() {
        return Ok(vec![]);
    }

    // 取 top-N * 2 候选，获取完整 MemoryEntry，snapshot 加权 1.2x 优先
    let mut results = Vec::with_capacity(scored.len().min(limit * 2));
    for (id, score) in scored.into_iter().take(limit * 2) {
        if let Ok(Some(entry)) = get_entry_by_id(&id) {
            results.push((entry, score));
        }
    }
    // kind 过滤（HNSW 无列过滤能力，取回后过滤）
    if let Some(k) = kind {
        results.retain(|(e, _)| e.kind == k);
    }
    // Custom 双向隔离（取回后过滤）：None==None 保留非 Custom 记忆，
    // Some(id)==Some(id) 保留同卡片，其余组合排除——一行覆盖双向。
    let custom_id = crate::custom_agents::current_custom_agent_id();
    results.retain(|(e, _)| e.custom_agent_id == custom_id);
    // 项目记忆隔离（取回后过滤）：默认只保留当前项目的 session（session_meta 登记）
    if !all_projects {
        if let Some(tag) = crate::utils::active_project_tag() {
            let project_sids: std::collections::HashSet<String> = {
                let guard = crate::store::db::acquire()?;
                let mut stmt =
                    guard.prepare("SELECT session_id FROM session_meta WHERE project_tag = ?1")?;
                // 中间变量绑定：块尾链式临时会与块内 guard/stmt 的 drop 顺序冲突（E0597）
                let rows: std::collections::HashSet<String> = stmt
                    .query_map(params![&tag], |r| r.get::<_, String>(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                rows
            };
            results.retain(|(e, _)| project_sids.contains(&e.session_id));
        }
    }
    // 按相似度降序
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);

    Ok(results)
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<MemoryEntry> {
    // 列序契约见 ENTRY_COLS（0-25）
    let tags_str: String = row.get(11)?;
    let tags: Vec<String> = if tags_str.is_empty() {
        vec![]
    } else {
        tags_str.split(',').map(|s| s.to_string()).collect()
    };

    let tools_used_str: String = row.get(16).unwrap_or_default();
    let tools_used: Vec<String> = if tools_used_str.is_empty() {
        vec![]
    } else {
        tools_used_str.split(',').map(|s| s.to_string()).collect()
    };

    let artifacts_str: String = row.get(19).unwrap_or_default();
    let artifacts: Vec<String> = if artifacts_str.is_empty() {
        vec![]
    } else {
        artifacts_str.split(',').map(|s| s.to_string()).collect()
    };

    let children_ids_str: String = row.get(23).unwrap_or_default();
    let children_ids: Vec<String> = if children_ids_str.is_empty() {
        vec![]
    } else {
        children_ids_str.split(',').map(|s| s.to_string()).collect()
    };

    let exec_steps_str: String = row.get(21).unwrap_or_default();
    let execution_steps: Vec<crate::memory::entry::PersistedStep> =
        serde_json::from_str(&exec_steps_str).unwrap_or_default();

    // AgentType / MemoryKind 从字符串解析（非法值回退默认值，不阻断查询）
    let agent_type_str: String = row.get(6)?;
    let agent_type = agent_type_str
        .parse::<AgentType>()
        .unwrap_or(AgentType::Leader);
    let kind_str: String = row.get(7)?;
    let kind = kind_str
        .parse::<MemoryKind>()
        .unwrap_or(MemoryKind::Conversation);

    Ok(MemoryEntry {
        id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        sequence: row.get(3)?,
        created_at: row.get(4)?,
        wall_clock_ms: row.get(5)?,
        agent_type,
        kind,
        task_chain_id: row.get(8)?,
        chain_step: row.get(9)?,
        goal_type: row.get(10)?,
        tags,
        intent: row.get(12)?,
        summary: row.get(13)?,
        user_message: row.get(14)?,
        assistant_message: row.get(15)?,
        tools_used,
        success: row.get::<_, i32>(17)? != 0,
        output: row.get(18)?,
        artifacts,
        is_marked: row.get::<_, i32>(20)? != 0,
        execution_steps,
        parent_id: row.get(22)?,
        children_ids,
        pattern: row.get(24)?,
        custom_agent_id: row.get(25).unwrap_or(None),
    })
}

// ── HNSW Embedding 索引 ──

/// HNSW 索引所需 Point 实现：包装 L2 归一化向量
#[derive(Clone)]
struct EmbeddingPoint(Vec<f32>);

impl Point for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        self.0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
}

static EMBEDDING_CACHE: OnceLock<RwLock<Option<HnswMap<EmbeddingPoint, String>>>> = OnceLock::new();

fn embedding_cache() -> &'static RwLock<Option<HnswMap<EmbeddingPoint, String>>> {
    EMBEDDING_CACHE.get_or_init(|| RwLock::new(None))
}

fn invalidate_embedding_cache() {
    if let Ok(mut cache) = embedding_cache().write() {
        *cache = None;
    }
}

/// 从 DB 加载所有 embedding 向量并构建 HNSW 索引，写入缓存。
/// 在写锁内构建（确保并发安全），构建完成后释放写锁。
fn build_hnsw_from_db() -> Option<HnswMap<EmbeddingPoint, String>> {
    let guard = crate::store::db::acquire().ok()?;
    let mut stmt = guard
        .prepare("SELECT id, embedding FROM memory_embeddings")
        .ok()?;
    let rows: Vec<(String, Vec<f32>)> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            let vec: Vec<f32> = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            Ok((id, vec))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    drop(guard);

    if rows.is_empty() {
        return None;
    }

    let (points, values): (Vec<EmbeddingPoint>, Vec<String>) = rows
        .into_iter()
        .map(|(id, vec)| (EmbeddingPoint(vec), id))
        .unzip();

    Some(Builder::default().build(points, values))
}

/// 确保 HNSW 索引已加载。首次调用时从 DB 构建，后续直接命中缓存。
fn ensure_hnsw_loaded() -> Option<()> {
    // 快速路径：已有索引
    if embedding_cache().read().ok()?.is_some() {
        return Some(());
    }
    // 慢路径：获取写锁，构建索引
    let mut cache = embedding_cache().write().ok()?;
    if cache.is_none() {
        *cache = build_hnsw_from_db();
    }
    cache.as_ref().map(|_| ())
}

impl From<rusqlite::Error> for crate::NuphusError {
    fn from(e: rusqlite::Error) -> Self {
        crate::NuphusError::store(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与生产表结构一致的内存 FTS5 表（db.rs memory_fts_v4 定义）
    fn setup_fts_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE memory_fts_v4 USING fts5(
                id UNINDEXED, kind, intent, summary, tags, pattern, output,
                tokenize='unicode61'
            );",
        )
        .unwrap();
        conn
    }

    /// 与生产写入路径一致：内容经 segment_for_fts 分词后入索引（sync_entry_fts）
    fn insert_fts(conn: &rusqlite::Connection, id: &str, intent: &str) {
        let seg = crate::segmenter::segment_for_fts(intent);
        conn.execute(
            "INSERT INTO memory_fts_v4(id, kind, intent, summary, tags, pattern, output)
             VALUES (?1, 'pattern', ?2, '', '', '', '')",
            params![id, seg],
        )
        .unwrap();
    }

    /// 走生产 build_fts_query 构造 MATCH 查询并计数
    fn match_count(conn: &rusqlite::Connection, query: &str) -> i64 {
        let fts = build_fts_query(query);
        assert!(!fts.is_empty(), "build_fts_query 不应为空: {}", query);
        conn.query_row(
            "SELECT count(*) FROM memory_fts_v4 WHERE memory_fts_v4 MATCH ?1",
            params![fts],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// 前缀匹配：查询词 "单例" 必须命中索引词 "单例化"（词边界误杀根治）
    #[test]
    fn test_prefix_match_hits_longer_token() {
        let conn = setup_fts_conn();
        insert_fts(&conn, "e1", "shared_client 单例化模式");
        assert_eq!(match_count(&conn, "单例"), 1);
        // 前缀是单向的：更长查询词不应命中更短索引词
        assert_eq!(match_count(&conn, "单例化"), 1);
    }

    /// AND 语义保持：任一关键词不共现即零结果，绝不返回部分匹配
    #[test]
    fn test_and_semantics_no_partial_match() {
        let conn = setup_fts_conn();
        insert_fts(&conn, "e1", "CookieVault 凭据保管");
        // "CookieVault" 命中但 "语音" 不共现 → 整体零结果
        assert_eq!(match_count(&conn, "CookieVault 语音"), 0);
        // 单词查询正常命中，证明零结果来自 AND 而非索引缺失
        assert_eq!(match_count(&conn, "CookieVault"), 1);
    }

    /// token_hit_counts 逐词计数正确（含零命中词）
    #[test]
    fn test_token_hit_counts() {
        let conn = setup_fts_conn();
        insert_fts(&conn, "e1", "CookieVault 凭据保管");
        insert_fts(&conn, "e2", "CookieVault 二次确认");
        let counts = token_hit_counts_on(&conn, "CookieVault 语音").unwrap();
        assert_eq!(
            counts,
            vec![
                ("cookievault".to_string(), 2usize),
                ("语音".to_string(), 0usize),
            ]
        );
    }

    /// P0 回归：自然语言句式查询经虚词过滤后，严格 AND 能命中核心词文档
    ///（修复前「白名单是怎么实现的」5 token AND → 0 命中，生产库实证）
    #[test]
    fn test_natural_sentence_query_hits() {
        let conn = setup_fts_conn();
        insert_fts(&conn, "e1", "工具白名单三处过滤实现");
        insert_fts(&conn, "e2", "移动端通道回退修复");
        assert_eq!(match_count(&conn, "白名单是怎么实现的"), 1);
        assert_eq!(match_count(&conn, "移动端的回退修复了吗"), 1);
    }
}
