//! Session 表存储层
//!
//! 提供 sessions 表的读写操作，支持 upsert / get / list。
//! 在 new_chat_session 时写入新记录，get_chat_history 从内存/SQLite 恢复。

use rusqlite::params;

/// 会话行记录
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub depth: i32,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: i32,
    pub token_count: i32,
    pub summary: String,
}

/// 插入或替换一条 session 记录
pub fn upsert_session(session: &SessionRow) -> crate::Result<()> {
    let guard = crate::store::db::acquire()?;

    guard.execute(
        "INSERT OR REPLACE INTO sessions
         (id, parent_id, depth, created_at, updated_at, message_count, token_count, summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            session.id,
            session.parent_id,
            session.depth,
            session.created_at,
            session.updated_at,
            session.message_count,
            session.token_count,
            session.summary,
        ],
    )?;

    Ok(())
}

/// 按 ID 读取一条 session 记录
pub fn get_session(session_id: &str) -> crate::Result<Option<SessionRow>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(
        "SELECT id, parent_id, depth, created_at, updated_at,
                message_count, token_count, summary
         FROM sessions WHERE id = ?1",
    )?;

    let mut rows = stmt.query_map(params![session_id], |row| {
        Ok(SessionRow {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            depth: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            message_count: row.get(5)?,
            token_count: row.get(6)?,
            summary: row.get(7)?,
        })
    })?;

    match rows.next() {
        Some(Ok(row)) => Ok(Some(row)),
        _ => Ok(None),
    }
}

/// 分页查询 session 列表（按 updated_at 降序）
pub fn list_sessions(limit: usize, offset: usize) -> crate::Result<Vec<SessionRow>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(
        "SELECT id, parent_id, depth, created_at, updated_at,
                message_count, token_count, summary
         FROM sessions
         ORDER BY updated_at DESC
         LIMIT ?1 OFFSET ?2",
    )?;

    let rows = stmt
        .query_map(params![limit as i64, offset as i64], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                depth: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                message_count: row.get(5)?,
                token_count: row.get(6)?,
                summary: row.get(7)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
}

/// 获取最新一条 session 记录
pub fn latest_session() -> crate::Result<Option<SessionRow>> {
    let mut rows = list_sessions(1, 0)?;
    Ok(rows.pop())
}
