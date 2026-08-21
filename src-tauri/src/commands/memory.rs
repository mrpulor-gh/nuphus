//! memory — Tauri memory system commands

use crate::state::{AppState, MemoryStats, SessionDetailEntry, TimelineIndexStats};
use nuphus::store::session;
use serde::{Deserialize, Serialize};
use tauri::State;

#[tauri::command]
pub fn get_memory_stats(_state: State<'_, AppState>) -> Result<MemoryStats, String> {
    let stats = nuphus::store::memory::entry_stats().unwrap_or_default();
    let entries_count = stats.total_entries as usize;

    // Count entries with actual pattern content
    let patterns_count: i64 = match nuphus::store::db::acquire() {
        Ok(guard) => guard
            .query_row(
                "SELECT COUNT(*) FROM memory_entries WHERE pattern IS NOT NULL AND pattern != ''",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0),
        Err(_) => 0,
    };

    Ok(MemoryStats {
        total_entries: entries_count,
        patterns: patterns_count as usize,
        skills: 0,
        principles: 0,
        templates: 0,
        seeds: 0,
    })
}

/// Get entry statistics (replaces legacy TimelineIndex)
#[tauri::command]
pub fn get_timeline_index_stats(_state: State<'_, AppState>) -> Result<TimelineIndexStats, String> {
    use nuphus::store::memory::entry_stats;
    let stats = entry_stats().map_err(|e| e.to_string())?;

    // Calculate session count
    let total_sessions: i64 = {
        let guard = nuphus::store::db::acquire().map_err(|e| e.to_string())?;
        guard
            .query_row(
                "SELECT COUNT(DISTINCT session_id) FROM memory_entries",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?
    };

    Ok(TimelineIndexStats {
        total_entries: stats.total_entries as usize,
        total_sessions: total_sessions as usize,
        successful: stats.by_success as usize,
        failed: stats.by_failure as usize,
        by_intent: stats
            .by_kind
            .into_iter()
            .map(|(k, v)| (k, v as usize))
            .collect(),
    })
}

/// Get history session list (read from SQLite sessions table)
/// For sessions with empty message_count/summary, fill data from memory_entries.
#[tauri::command]
pub fn get_session_history() -> Result<Vec<nuphus::store::memory::SessionSummary>, String> {
    // 1) Read ALL rows from sessions table
    let rows =
        session::list_sessions(500, 0).map_err(|e| format!("Failed to load sessions: {}", e))?;

    let mut sessions: Vec<nuphus::store::memory::SessionSummary> = Vec::new();
    let mut empty_ids: Vec<String> = Vec::new();

    for r in rows {
        if r.message_count > 0 && !r.summary.is_empty() {
            let dt = chrono::DateTime::parse_from_rfc3339(&r.updated_at)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            sessions.push(nuphus::store::memory::SessionSummary {
                session_id: r.id.clone(),
                user_message: r.summary.clone(),
                intent: r.summary,
                last_assistant_message: String::new(),
                entry_count: r.message_count as u32,
                timestamp: dt,
                success: true,
                tags: Vec::new(),
                tool_call_count: 0,
            });
        } else {
            empty_ids.push(r.id);
        }
    }

    // 2) Fill empty sessions from memory_entries (only leader dialogue entries)
    if !empty_ids.is_empty() {
        let guard = nuphus::store::db::acquire().map_err(|e| e.to_string())?;
        let placeholders: Vec<String> = (1..=empty_ids.len()).map(|i| format!("?{}", i)).collect();
        // Exclude snapshot entries — their user_message
        // is memory.md content (not a user query), which pollutes the session title.
        let sql = format!(
            "SELECT session_id,
                    COALESCE(
                         (SELECT summary FROM memory_entries
                          WHERE session_id = me.session_id AND kind = 'distill' AND summary != ''
                          ORDER BY created_at DESC LIMIT 1),
                         (SELECT user_message FROM memory_entries
                          WHERE session_id = me.session_id
                            AND kind = 'conversation'
                            AND user_message != ''
                          ORDER BY created_at ASC LIMIT 1),
                          (SELECT intent FROM memory_entries
                           WHERE session_id = me.session_id
                             AND agent_type = 'leader'
                             AND intent != ''
                           ORDER BY created_at ASC LIMIT 1),
                          ''
                     ),
                     COALESCE((SELECT intent FROM memory_entries WHERE session_id = me.session_id AND agent_type = 'leader' AND intent != '' ORDER BY created_at DESC LIMIT 1), ''),
                     COALESCE((SELECT assistant_message FROM memory_entries WHERE session_id = me.session_id AND agent_type = 'leader' AND assistant_message != '' ORDER BY created_at DESC LIMIT 1), ''),
                     COUNT(*) FILTER (WHERE kind = 'conversation'),
                     MAX(created_at),
                     MAX(CASE WHEN success = 1 THEN 1 ELSE 0 END)
              FROM memory_entries me
              WHERE session_id IN ({})
              GROUP BY session_id
              ORDER BY MAX(created_at) DESC",
            placeholders.join(",")
        );

        let prepared = guard.prepare(&sql);
        if let Ok(mut stmt) = prepared {
            let params: Vec<&dyn rusqlite::types::ToSql> = empty_ids
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                let session_id: String = row.get(0)?;
                let user_message: String = row.get(1)?;
                let intent: String = row.get(2)?;
                let last_assistant_message: String = row.get(3)?;
                let entry_count: i64 = row.get(4)?;
                let last_ts: String = row.get(5)?;
                let has_success: i64 = row.get(6)?;
                Ok(nuphus::store::memory::SessionSummary {
                    session_id,
                    user_message,
                    intent,
                    last_assistant_message,
                    entry_count: entry_count as u32,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&last_ts)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    success: has_success > 0,
                    tags: Vec::new(),
                    tool_call_count: 0,
                })
            }) {
                for row in rows.flatten() {
                    sessions.push(row);
                }
            }
        }
    }

    // 3) Batch query memory_entries for kind breakdown (conversation turns + tool calls)
    if !sessions.is_empty() {
        let guard = nuphus::store::db::acquire().map_err(|e| e.to_string())?;
        let all_ids: Vec<String> = sessions.iter().map(|s| s.session_id.clone()).collect();
        let placeholders: Vec<String> = (1..=all_ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT session_id,
                    COUNT(*) FILTER (WHERE kind = 'conversation') as turns,
                    COUNT(*) FILTER (WHERE kind = 'task_trace') as tool_calls
             FROM memory_entries
             WHERE session_id IN ({})
             GROUP BY session_id",
            placeholders.join(",")
        );
        // Build lookup map: session_id -> (turns, tool_calls)
        let prepared = guard.prepare(&sql);
        if let Ok(mut stmt) = prepared {
            let params: Vec<&dyn rusqlite::types::ToSql> = all_ids
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?, // session_id
                    row.get::<_, i64>(1)?,    // turns (conversation count)
                    row.get::<_, i64>(2)?,    // tool_calls (task_trace count)
                ))
            }) {
                let mut count_map: std::collections::HashMap<String, (u32, u32)> =
                    std::collections::HashMap::new();
                for row in rows.flatten() {
                    count_map.insert(row.0, (row.1 as u32, row.2 as u32));
                }
                // Update all sessions
                for s in &mut sessions {
                    if let Some((turns, tool_calls)) = count_map.get(&s.session_id) {
                        s.entry_count = *turns;
                        s.tool_call_count = *tool_calls;
                    } else {
                        // Session has no memory_entries — keep entry_count if already set, default 0
                        s.tool_call_count = 0;
                    }
                }
            }
        }
    }

    Ok(sessions)
}
/// ensuring clean dialogue results are displayed to user.
#[tauri::command]
pub fn get_session_detail(session_id: String) -> Result<Vec<SessionDetailEntry>, String> {
    let entries = nuphus::store::memory::get_entries_by_session(&session_id)
        .map_err(|e| format!("Failed to load entries for session: {}", e))?;

    let raw_count = entries.len();
    let first_kind = entries
        .first()
        .map(|e| e.kind.as_str().to_string())
        .unwrap_or_default();
    let first_sid = entries
        .first()
        .map(|e| e.session_id.clone())
        .unwrap_or_default();
    tracing::info!(
        "[memory_panel] session={} raw_entries={} first_kind={} sid_match={}",
        &session_id[..8.min(session_id.len())],
        raw_count,
        first_kind,
        first_sid == session_id,
    );

    // kind 驱动：展示对话、提炼、任务轨迹、快照、模式
    let mut session_entries: Vec<SessionDetailEntry> = entries
        .into_iter()
        .filter(|e| {
            if e.session_id != session_id {
                return false;
            }
            match e.kind {
                nuphus::memory::entry::MemoryKind::Distill => !e.summary.is_empty(),
                nuphus::memory::entry::MemoryKind::Conversation
                | nuphus::memory::entry::MemoryKind::TaskTrace => {
                    !(e.assistant_message.starts_with("tool ")
                        && e.assistant_message.contains("failed")
                        && !e.success)
                }
                nuphus::memory::entry::MemoryKind::Pattern
                | nuphus::memory::entry::MemoryKind::Snapshot => {
                    !e.summary.is_empty() || !e.user_message.is_empty()
                }
            }
        })
        .map(|e| {
            let is_distill = e.kind == nuphus::memory::entry::MemoryKind::Distill;
            let is_snapshot = e.kind == nuphus::memory::entry::MemoryKind::Snapshot;
            let is_pattern = e.kind == nuphus::memory::entry::MemoryKind::Pattern;
            SessionDetailEntry {
                id: e.id.clone(),
                kind: e.kind.as_str().to_string(),
                user_message: if is_distill {
                    format!("[提炼] {}", e.summary.chars().take(200).collect::<String>())
                } else if is_snapshot || is_pattern {
                    if !e.summary.is_empty() {
                        e.summary.clone()
                    } else {
                        e.user_message.clone()
                    }
                } else if !e.user_message.is_empty() {
                    e.user_message.clone()
                } else {
                    e.intent.clone()
                },
                assistant_message: if is_distill || is_snapshot || is_pattern {
                    String::new()
                } else {
                    e.assistant_message.clone()
                },
                steps_summary: e.tools_used.clone(),
                goal_type: e.goal_type.clone(),
                timestamp: e.created_at.clone(),
                success: e.success,
            }
        })
        .collect();

    // 按 kind 优先级排序：conversation > task_trace > distill > pattern/snapshot
    // 同优先级按时间排序
    session_entries.sort_by(|a, b| {
        let kind_order = |k: &str| match k {
            "conversation" => 0,
            "task_trace" => 1,
            "distill" => 2,
            "snapshot" => 3,
            "pattern" => 3,
            _ => 4,
        };
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then(a.timestamp.cmp(&b.timestamp))
    });
    Ok(session_entries)
}

#[tauri::command]
pub fn get_knowledge_items(category: String) -> Result<serde_json::Value, String> {
    let _ = category;
    Ok(serde_json::json!([]))
}

// ── Memory Overview（概览 tab 聚合数据）──

#[derive(Debug, Clone, Serialize)]
pub struct MemoryOverview {
    pub total_entries: u64,
    pub success_rate: f64,
    pub db_size_bytes: u64,
    pub embedded_count: i64,
    /// 最早记忆条目时间戳（毫秒）
    pub oldest_ms: u64,
    /// 最新记忆条目时间戳（毫秒）
    pub newest_ms: u64,
    /// 提炼条目数（distill）
    pub distill_count: u64,
    /// 用户标记/模式条目数（pattern）
    pub pattern_count: u64,
}

/// 记忆概览：统计 + 时间跨度 + 经验沉淀 + 向量覆盖（概览 tab 数据源）
#[tauri::command]
pub fn get_memory_overview(_state: State<'_, AppState>) -> Result<MemoryOverview, String> {
    let stats = nuphus::store::memory::entry_stats().map_err(|e| e.to_string())?;
    let success_rate = if stats.total_entries > 0 {
        (stats.by_success as f64 / stats.total_entries as f64 * 1000.0).round() / 10.0
    } else {
        0.0
    };

    let distill_count = stats.by_kind.get("distill").copied().unwrap_or(0);
    let pattern_count = stats.by_kind.get("pattern").copied().unwrap_or(0);

    let embedded_count: i64 = nuphus::store::db::acquire()
        .and_then(|g| {
            g.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |r| r.get(0))
                .map_err(Into::into)
        })
        .unwrap_or(0);

    Ok(MemoryOverview {
        total_entries: stats.total_entries,
        success_rate,
        db_size_bytes: nuphus::store::db::db_size(),
        embedded_count,
        oldest_ms: stats.oldest_ms,
        newest_ms: stats.newest_ms,
        distill_count,
        pattern_count,
    })
}

/// Delete knowledge entry (soft delete by setting active=false)
#[tauri::command]
pub fn delete_knowledge_item(category: String, id: String) -> Result<bool, String> {
    let _ = (category, id);
    Err("不支持的分类".to_string())
}

/// User submits execution rating (frontend review popup → memory system)
///
/// Rating logic:
/// - 4-5⭐ → "Save as strategy": tag strategy/high_quality, extract patterns from steps
/// - 3⭐   → "Save review": neutral archive, enter memory normally
/// - 1-2⭐ → Low quality: tag error_record, keep as error record but don't trigger evolution
#[tauri::command]
pub fn submit_execution_rating(
    goal: String,
    rating: u8,
    comment: String,
    tools_summary: String,
    steps_json: String,
    session_id: String,
) -> Result<String, String> {
    use nuphus::memory::entry::{
        normalize_tags, AgentType, MemoryEntry, MemoryKind, PersistedStep,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("时间错误: {}", e))?;
    let wall_clock_ms = now.as_millis() as u64;
    let turn_id = if session_id.is_empty() {
        format!("user-rating-{}", wall_clock_ms)
    } else {
        session_id.clone()
    };

    let is_high_quality = rating >= 4;
    let is_low_quality = rating <= 2;

    // ID 含 wall_clock_ms：同 session 多次评分不再互相覆盖
    let entry_session_id = if session_id.is_empty() {
        turn_id.clone()
    } else {
        session_id
    };
    let mut entry = MemoryEntry::new(
        format!("rating-{}-{}", entry_session_id, wall_clock_ms),
        entry_session_id,
        turn_id,
        AgentType::Leader,
        MemoryKind::Pattern,
    );
    entry.sequence = 0;
    entry.wall_clock_ms = wall_clock_ms;
    entry.success = rating >= 3;

    // ── Set different tags based on rating level ──
    if is_high_quality {
        // 4-5⭐: High quality strategy, force into memory for retrieval reuse
        entry.intent = format!(
            "[策略] {}⭐ · {}",
            rating,
            goal.chars().take(60).collect::<String>()
        );
        entry.summary = format!(
            "用户评分 {}⭐ | 可复用策略: {}",
            rating,
            comment.chars().take(200).collect::<String>()
        );
        entry.user_message = goal.clone();
        entry.assistant_message = comment.clone();
        entry.tools_used = tools_summary
            .split(", ")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let has_steps = !entry.tools_used.is_empty();
        let mut base_tags = vec![
            "user_rating".to_string(),
            format!("rating_{}", rating),
            "positive".to_string(),
        ];
        if has_steps {
            base_tags.extend_from_slice(&[
                "strategy".to_string(),
                "high_quality".to_string(),
                "check_pattern".to_string(),
            ]);
        }
        entry.tags = normalize_tags(&base_tags);

        // Parse execution steps from steps_json（紧凑格式：构造时即截断摘要）
        if !steps_json.trim().is_empty() {
            if let Ok(steps_array) = serde_json::from_str::<Vec<serde_json::Value>>(&steps_json) {
                let mut exec_steps: Vec<PersistedStep> = steps_array
                    .iter()
                    .filter_map(|s| {
                        let tool = s
                            .get("tool")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if tool.is_empty() {
                            return None;
                        }
                        let params = s.get("params").cloned().unwrap_or(serde_json::json!({}));
                        let success = s.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                        let result = s
                            .get("result")
                            .and_then(|v| v.as_str())
                            .or_else(|| s.get("error").and_then(|v| v.as_str()));
                        let duration = s.get("durationMs").and_then(|v| v.as_u64());
                        Some(PersistedStep::new(tool, &params, result, success, duration))
                    })
                    .collect();
                // 紧凑轨迹最多保留最后 20 步
                if exec_steps.len() > 20 {
                    exec_steps = exec_steps.split_off(exec_steps.len() - 20);
                }
                entry.execution_steps = exec_steps;
            }
        }

        // Build practical pattern — compact signal format, human-validated
        let tools = entry.tools_used.clone();
        if !tools.is_empty() {
            let goal_short = goal.chars().take(100).collect::<String>();
            let chain = tools.join(" → ");
            if comment.trim().is_empty() {
                entry.pattern = Some(format!("{}\n{}", goal_short, chain));
            } else {
                let comment_short = comment.chars().take(200).collect::<String>();
                entry.pattern = Some(format!("{}\n→ {}\n{}", goal_short, comment_short, chain));
            }
        }
    } else if is_low_quality {
        // 1-2⭐: Low quality, record as error but don't trigger evolution funnel
        let feedback = comment; // move comment once
        entry.intent = format!(
            "[低质] {}⭐ · {}",
            rating,
            goal.chars().take(60).collect::<String>()
        );
        entry.summary = format!(
            "用户评分 {}⭐ | 需改进: {}",
            rating,
            feedback.chars().take(200).collect::<String>()
        );
        entry.user_message = goal;
        entry.assistant_message = feedback;
        entry.tools_used = tools_summary
            .split(", ")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        entry.tags = normalize_tags(&[
            "user_rating".to_string(),
            format!("rating_{}", rating),
            "negative".to_string(),
            "error_record".to_string(),
        ]);
    } else {
        // 3⭐: Neutral, normal archive
        entry.intent = format!(
            "用户评分: {}⭐ · {}",
            rating,
            goal.chars().take(60).collect::<String>()
        );
        entry.summary = format!(
            "用户评分 {}⭐ | {}",
            rating,
            comment.chars().take(200).collect::<String>()
        );
        entry.user_message = goal;
        entry.assistant_message = comment;
        entry.tools_used = tools_summary
            .split(", ")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        entry.tags = normalize_tags(&[
            "user_rating".to_string(),
            format!("rating_{}", rating),
            "neutral".to_string(),
        ]);
    }

    nuphus::store::memory::insert_entry(&entry).map_err(|e| format!("保存评分失败: {}", e))?;

    let label = if is_high_quality {
        "策略已保存"
    } else if is_low_quality {
        "已记录反馈"
    } else {
        "点评已保存"
    };
    Ok(format!("{} ({}⭐)", label, rating))
}

// ── Below is merged content from memory_v2.rs (memory entry CRUD / search) ──

// ── UserMemory DTO (frontend/backend aligned) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMemory {
    pub id: String,
    pub session_id: String,
    /// 一维分类：conversation / task_trace / distill / pattern / snapshot
    pub kind: String,
    pub agent_type: String,
    pub goal_type: Option<String>,
    pub title: String,
    pub intent: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub quality_score: f64,
    pub user_rating: Option<u8>,
    pub is_marked: bool,
    pub block_injection: bool,
    pub pattern: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFilter {
    pub tag: Option<String>,
    pub kind: Option<String>,
    pub marked_only: Option<bool>,
    pub min_quality: Option<f64>,
    pub search: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdates {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub tags: Option<Vec<String>>,
    pub user_rating: Option<Option<u8>>,
    pub is_marked: Option<bool>,
    pub block_injection: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryListResult {
    pub memories: Vec<UserMemory>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

// ── Conversion: MemoryEntry → UserMemory ──

fn entry_to_user_memory(entry: &nuphus::memory::MemoryEntry) -> UserMemory {
    UserMemory {
        id: entry.id.clone(),
        session_id: entry.session_id.clone(),
        kind: entry.kind.as_str().to_string(),
        agent_type: entry.agent_type.to_string(),
        goal_type: entry.goal_type.clone(),
        title: entry.intent.clone(),
        intent: entry.intent.clone(),
        summary: entry.summary.clone(),
        tags: entry.tags.clone(),
        quality_score: if entry.success { 0.8 } else { 0.3 },
        user_rating: None,
        is_marked: entry.is_marked,
        block_injection: false,
        pattern: entry.pattern.clone(),
        created_at: entry.created_at.clone(),
        updated_at: entry.created_at.clone(),
    }
}

// ── Tauri commands ──

#[tauri::command]
pub async fn list_memories(
    filter: Option<MemoryFilter>,
    _state: State<'_, AppState>,
) -> Result<MemoryListResult, String> {
    let limit = filter.as_ref().and_then(|f| f.limit).unwrap_or(50);
    let offset = filter.as_ref().and_then(|f| f.offset).unwrap_or(0);

    let min_quality = filter.as_ref().and_then(|f| f.min_quality);
    let tag_filter = filter.as_ref().and_then(|f| f.tag.clone());
    let search = filter.as_ref().and_then(|f| f.search.clone());
    // kind 过滤（非法值静默忽略为不过滤，UI 层传枚举值）
    let kind_filter = filter
        .as_ref()
        .and_then(|f| f.kind.as_deref())
        .and_then(|k| k.parse::<nuphus::memory::entry::MemoryKind>().ok());

    let entries = nuphus::store::memory::search_entries_filtered(
        None,                                                       // query (no FTS)
        None,                                                       // session_id
        None,                                                       // goal_type
        None,                                                       // agent_type
        kind_filter,                                                // kind
        tag_filter.as_ref().map(std::slice::from_ref),              // tags
        None,                                                       // success
        None,                                                       // time_window_ms
        filter.as_ref().and_then(|f| f.marked_only).filter(|&m| m), // is_marked
        None,                                                       // exclude_goal_types
        filter.as_ref().and_then(|f| f.search.as_deref()),          // search_text
        5000, // limit (still high for min_quality fallback)
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(
        "[list_memories] entries from search_entries_filtered: {} rows",
        entries.len()
    );

    let mut memories: Vec<UserMemory> = entries
        .iter()
        .filter(|e| {
            // marked_only mode: only return user-marked entries
            if filter.as_ref().and_then(|f| f.marked_only).unwrap_or(false) && !e.is_marked {
                return false;
            }

            if let Some(min_q) = min_quality {
                let score = if e.success { 0.8 } else { 0.3 };
                if score < min_q {
                    return false;
                }
            }
            if let Some(ref tag) = tag_filter {
                if !e.tags.contains(tag) {
                    return false;
                }
            }
            true
        })
        .map(entry_to_user_memory)
        .collect();

    if let Some(ref s) = search {
        let s_lower = s.to_lowercase();
        memories.retain(|m| {
            m.title.to_lowercase().contains(&s_lower)
                || m.summary.to_lowercase().contains(&s_lower)
                || m.tags.iter().any(|t| t.to_lowercase().contains(&s_lower))
        });
    }

    memories.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let total = memories.len();

    let paginated: Vec<UserMemory> = memories.into_iter().skip(offset).take(limit).collect();

    Ok(MemoryListResult {
        memories: paginated,
        total,
        offset,
        limit,
    })
}

#[tauri::command]
pub async fn update_memory(
    id: String,
    updates: MemoryUpdates,
    _state: State<'_, AppState>,
) -> Result<UserMemory, String> {
    let result = nuphus::store::memory::update_entry(&id, |entry| {
        if let Some(ref title) = updates.title {
            entry.intent = title.clone();
        }
        if let Some(ref summary) = updates.summary {
            entry.summary = summary.clone();
        }
        if let Some(ref tags) = updates.tags {
            entry.tags = tags.clone();
        }
        if let Some(Some(r)) = updates.user_rating {
            // user_rating 不在 MemoryEntry 上，存到 tags 里
            entry.tags.push(format!("user_rating_{}", r));
        }
        if let Some(marked) = updates.is_marked {
            entry.is_marked = marked;
        }
        if let Some(block) = updates.block_injection {
            if block {
                entry.tags.push("block_injection".to_string());
            } else {
                entry.tags.retain(|t| t != "block_injection");
            }
        }
    })
    .map_err(|e| e.to_string())?;

    result
        .as_ref()
        .map(entry_to_user_memory)
        .ok_or_else(|| format!("未找到记忆条目: {}", id))
}

#[tauri::command]
pub async fn delete_memory(id: String, _state: State<'_, AppState>) -> Result<(), String> {
    nuphus::store::memory::delete_entry(&id).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_mark_memory(
    id: String,
    _state: State<'_, AppState>,
) -> Result<UserMemory, String> {
    // 先读取当前状态
    let current = nuphus::store::memory::get_entry_by_id(&id).map_err(|e| e.to_string())?;
    let entry = current
        .as_ref()
        .ok_or_else(|| format!("未找到记忆条目: {}", id))?;
    let new_marked = !entry.is_marked;

    let result = nuphus::store::memory::update_entry(&id, |entry| {
        entry.is_marked = new_marked;
    })
    .map_err(|e| e.to_string())?;

    result
        .as_ref()
        .map(entry_to_user_memory)
        .ok_or_else(|| format!("未找到记忆条目: {}", id))
}
