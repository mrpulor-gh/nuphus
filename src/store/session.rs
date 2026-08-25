//! Session 表存储层
//!
//! 提供 sessions 表的读写操作，支持 upsert / get / list / snapshot。
//! 在 new_chat_session 时写入新记录，get_chat_history 从内存/SQLite 恢复。
//!
//! 快照（snapshot）：完整 Session 序列化 JSON（含 ToolUse/ToolResult/执行过程），
//! 方案A 起由 Shelf 层直接持久化到本表 snapshot 列，替代旧磁盘镜像文件。
//! `upsert_session` 使用真 UPSERT（ON CONFLICT DO UPDATE），绝不触碰 snapshot/mode
//! 列——rename/元数据刷新不会清掉已持久化的完整快照。

use rusqlite::params;
use std::collections::HashMap;

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

/// 插入或更新一条 session 记录（真 UPSERT）。
///
/// 冲突时仅更新元数据列，保留 created_at，且**不触碰 mode / snapshot 列**——
/// 调用方（rename、退出钩子、workflow 轮次回填）刷新元数据不会清空已持久化快照。
pub fn upsert_session(session: &SessionRow) -> crate::Result<()> {
    let guard = crate::store::db::acquire()?;

    guard.execute(
        "INSERT INTO sessions
         (id, parent_id, depth, created_at, updated_at, message_count, token_count, summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            parent_id     = excluded.parent_id,
            depth         = excluded.depth,
            updated_at    = excluded.updated_at,
            message_count = excluded.message_count,
            token_count   = excluded.token_count,
            summary       = excluded.summary",
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

// ══════════════════════════════════════════════════════════════════════════
// 快照（完整 Session 持久化）
// ══════════════════════════════════════════════════════════════════════════

/// 写入/更新完整快照（真 UPSERT）。
///
/// 冲突时仅更新 mode / snapshot / updated_at，保留 created_at 与全部用户可见
/// 元数据（summary / message_count / token_count / parent_id / depth）——
/// 快照写入幂等，且不覆盖列表页展示的元数据。
/// 行不存在时以默认元数据（depth=0, message_count=0, summary=''）创建，
/// 后续 `upsert_session` 会补齐真实元数据。
pub fn upsert_snapshot(id: &str, mode: &str, snapshot: &str) -> crate::Result<()> {
    let guard = crate::store::db::acquire()?;
    let now = chrono::Utc::now().to_rfc3339();

    guard.execute(
        "INSERT INTO sessions
         (id, parent_id, depth, created_at, updated_at, message_count, token_count, summary, mode, snapshot)
         VALUES (?1, NULL, 0, ?2, ?2, 0, 0, '', ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            mode      = excluded.mode,
            snapshot  = excluded.snapshot,
            updated_at = excluded.updated_at",
        params![id, now, mode, snapshot],
    )?;

    Ok(())
}

/// 按 ID 读取快照，返回 (mode, snapshot_json)。无快照（列 NULL 或行不存在）返回 None。
pub fn get_snapshot(id: &str) -> crate::Result<Option<(String, String)>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(
        "SELECT mode, snapshot FROM sessions
         WHERE id = ?1 AND snapshot IS NOT NULL",
    )?;

    let mut rows = stmt.query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    match rows.next() {
        Some(Ok(v)) => Ok(Some(v)),
        _ => Ok(None),
    }
}

/// 列出有快照的会话（按 updated_at 降序，最新在前）。返回 (id, mode, updated_at)。
pub fn list_snapshots(limit: usize) -> crate::Result<Vec<(String, String, String)>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(
        "SELECT id, mode, updated_at FROM sessions
         WHERE snapshot IS NOT NULL
         ORDER BY updated_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt
        .query_map(params![limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
}

/// 获取最新的快照（按 updated_at 降序取 1 条），返回 (mode, snapshot_json)。
pub fn latest_snapshot() -> crate::Result<Option<(String, String)>> {
    let guard = crate::store::db::acquire()?;

    let mut stmt = guard.prepare(
        "SELECT mode, snapshot FROM sessions
         WHERE snapshot IS NOT NULL
         ORDER BY updated_at DESC
         LIMIT 1",
    )?;

    let mut rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    match rows.next() {
        Some(Ok(v)) => Ok(Some(v)),
        _ => Ok(None),
    }
}

/// 删除快照（仅清 snapshot 列，保留元数据行——列表页历史不被移除）。
pub fn delete_snapshot(id: &str) -> crate::Result<()> {
    let guard = crate::store::db::acquire()?;
    guard.execute(
        "UPDATE sessions SET snapshot = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// 删除整行 session 记录（含元数据与快照）。用于测试清理或显式删除会话。
pub fn delete_session(id: &str) -> crate::Result<()> {
    let guard = crate::store::db::acquire()?;
    guard.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
    Ok(())
}

/// 批量查询会话创建时间（id → RFC3339 created_at）。用于 Shelf 稳定排序：
/// 切换/激活不改变创建时间，列表位置恒定，只动态变化颜色/效果。
pub fn list_created_at(ids: &[String]) -> crate::Result<HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let guard = crate::store::db::acquire()?;
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT id, created_at FROM sessions WHERE id IN ({placeholders})");
    let mut stmt = guard.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for r in rows.flatten() {
        map.insert(r.0, r.1);
    }
    Ok(map)
}

/// 保留策略：仅保留最近 `keep` 个有快照的会话（按 updated_at 倒序），超出部分
/// 清空 snapshot 列（元数据行保留——记忆页列表历史仍在，仅不可切换恢复）。
///
/// 对齐 Shelf「轻量切换 10 个」的产品语义：rail 可见会话（内存 LRU + active）总有
/// 完整 Session 可切换；SQLite 快照只保障崩溃/重启后恢复这 10 个，防止无界增长
/// （被动熵增）。返回被清理的快照数量。
pub fn prune_snapshots(keep: usize) -> crate::Result<usize> {
    let guard = crate::store::db::acquire()?;
    let n = guard.execute(
        "UPDATE sessions SET snapshot = NULL
         WHERE snapshot IS NOT NULL
           AND id NOT IN (
               SELECT id FROM sessions
               WHERE snapshot IS NOT NULL
               ORDER BY updated_at DESC
               LIMIT ?1
           )",
        params![keep as i64],
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn random_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn row_with(id: &str, summary: &str) -> SessionRow {
        let now = chrono::Utc::now().to_rfc3339();
        SessionRow {
            id: id.to_string(),
            parent_id: None,
            depth: 0,
            created_at: now.clone(),
            updated_at: now,
            message_count: 0,
            token_count: 0,
            summary: summary.to_string(),
        }
    }

    /// 真 UPSERT：冲突更新元数据后，已持久化的快照必须原样保留（rename 路径）
    #[serial]
    #[test]
    fn upsert_session_does_not_clear_snapshot() {
        let id = random_id();
        upsert_snapshot(&id, "leader", r#"{"id":"x","messages":[]}"#).unwrap();

        upsert_session(&row_with(&id, "重命名后的标题")).unwrap();

        let snap = get_snapshot(&id)
            .unwrap()
            .expect("快照不应被 upsert_session 清空");
        assert_eq!(snap.0, "leader");
        assert_eq!(snap.1, r#"{"id":"x","messages":[]}"#);

        delete_session(&id).unwrap();
        assert!(get_session(&id).unwrap().is_none());
    }

    /// 快照写入不得覆盖用户可见元数据（summary / created_at / message_count）
    #[serial]
    #[test]
    fn upsert_snapshot_preserves_visible_metadata() {
        let id = random_id();
        let now = chrono::Utc::now().to_rfc3339();
        let mut row = row_with(&id, "原始摘要");
        row.created_at = now.clone();
        row.message_count = 3;
        row.token_count = 100;
        upsert_session(&row).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(2));
        upsert_snapshot(&id, "workflow", r#"{"snap":1}"#).unwrap();

        let stored = get_session(&id).unwrap().expect("行应存在");
        assert_eq!(stored.summary, "原始摘要", "summary 不应被快照覆盖");
        assert_eq!(stored.created_at, now, "created_at 不应被快照覆盖");
        assert_eq!(stored.message_count, 3, "message_count 不应被快照覆盖");
        assert_eq!(stored.token_count, 100, "token_count 不应被快照覆盖");

        let snap = get_snapshot(&id).unwrap().unwrap();
        assert_eq!(snap.0, "workflow");
        assert_eq!(snap.1, r#"{"snap":1}"#);

        delete_session(&id).unwrap();
    }

    /// roundtrip + 列表按 updated_at 倒序 + latest + delete 后 None
    #[serial]
    #[test]
    fn snapshot_crud_roundtrip_and_order() {
        let a = random_id();
        let b = random_id();
        upsert_snapshot(&a, "leader", r#"{"a":1}"#).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        upsert_snapshot(&b, "workflow", r#"{"b":2}"#).unwrap();

        let list = list_snapshots(10).unwrap();
        let pos_a = list.iter().position(|(id, _, _)| id == &a).unwrap();
        let pos_b = list.iter().position(|(id, _, _)| id == &b).unwrap();
        assert!(pos_b < pos_a, "更新的快照应排在前面（倒序）");

        let (mode, _) = latest_snapshot().unwrap().expect("latest 应有值");
        assert_eq!(mode, "workflow");

        delete_snapshot(&a).unwrap();
        assert!(
            get_snapshot(&a).unwrap().is_none(),
            "delete 后 read 应为 None"
        );
        // 元数据行仍保留（快照删除不删列表项）
        assert!(get_session(&a).unwrap().is_some());

        delete_session(&a).unwrap();
        delete_session(&b).unwrap();
    }

    /// 保留策略：prune 后全局快照数收敛到 keep 以内，二次调用幂等（无更多清理）。
    /// 断言不依赖真实库全局状态（真实库可能已有快照），仅验证上界与收敛。
    #[serial]
    #[test]
    fn prune_snapshots_converges_and_bounds() {
        let mut ids = Vec::new();
        for _ in 0..12 {
            let id = random_id();
            // 递增 updated_at，保证排序稳定（不依赖真实库时间）
            std::thread::sleep(std::time::Duration::from_millis(2));
            upsert_snapshot(&id, "leader", r#"{"t":1}"#).unwrap();
            ids.push(id);
        }

        let cleaned = prune_snapshots(10).unwrap();
        let remaining = list_snapshots(100).unwrap().len();
        assert!(
            remaining <= 10,
            "prune 后快照数应 ≤ keep(10)，实际 {remaining}"
        );
        let second = prune_snapshots(10).unwrap();
        assert_eq!(
            second, 0,
            "二次 prune 应幂等收敛（无更多清理），实际 {second}"
        );
        let _ = cleaned;

        for id in &ids {
            delete_session(id).unwrap();
        }
    }
}
