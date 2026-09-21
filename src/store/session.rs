//! Session 表存储层
//!
//! 提供 sessions 表的读写操作，支持 upsert / get / list / snapshot。
//! 在 new_chat_session 时写入新记录，get_chat_history 从内存/SQLite 恢复。
//!
//! 快照（snapshot）：完整 Session 序列化 JSON（含 ToolUse/ToolResult/执行过程），
//! 方案A 起由 Shelf 层直接持久化到本表 snapshot 列，替代旧磁盘镜像文件。
//! `upsert_session` 使用真 UPSERT（ON CONFLICT DO UPDATE），绝不触碰 snapshot/mode
//! 列——rename/元数据刷新不会清掉已持久化的完整快照。

use rusqlite::{params, params_from_iter};
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
    let placeholders = std::iter::repeat_n("?", ids.len())
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

/// 快照保留策略（白名单制）：仅清理 `protected` 名单之外会话的 snapshot 列
/// （元数据行保留——记忆页列表历史仍在，仅不可切换恢复）。返回被清理的快照数量。
///
/// 与 Shelf「轻量切换 10 个」的产品语义对齐，但判定依据从 updated_at 时间序
/// 截断改为显式 id 白名单：active 会话的 updated_at 被每轮执行反复刷新、
/// 长期驻留成员的时间戳冻结，按时间截断曾系统性误杀驻留成员的快照（表现为
/// 重启后 shelf 只剩寥寥数个可恢复会话）。保护名单由调用方收集：
/// runtime active 会话 ∪ shelf 全量驻留成员 ∪ session_backup 中转会话。
///
/// 空 `protected` 直接返回 Ok(0) 防御性短路，不执行 UPDATE——调用方拿不到
/// 状态时宁可跳过裁剪，也不允许意外清空全库快照。
pub fn prune_snapshots(protected: &[String]) -> crate::Result<usize> {
    if protected.is_empty() {
        return Ok(0);
    }
    let guard = crate::store::db::acquire()?;
    let placeholders: Vec<String> = (1..=protected.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE sessions SET snapshot = NULL
         WHERE snapshot IS NOT NULL
           AND id NOT IN ({})",
        placeholders.join(", ")
    );
    let n = guard.execute(&sql, params_from_iter(protected.iter()))?;
    Ok(n)
}

// ══════════════════════════════════════════════════════════════════════════
// 项目归属（session_meta）：会话 → 项目文件夹
// ══════════════════════════════════════════════════════════════════════════
//
// 语义：归属在**会话诞生时快照**——首次登记即定稿，此后切换工作目录 / 恢复会话 /
// 切换会话一律不改写（INSERT OR IGNORE 保证「只记首次」）。历史会话（无 session_meta
// 行，或旧惰性登记留下的无路径行）保持无归属，查询返回 None：不猜测、不按当前目录
// 回填，由前端归入「未分组」。
// project_tag 仍是记忆检索的项目过滤依据（语义未变）；project_path 为展示用原始路径
// （tag 含路径哈希，不可逆，无法从 tag 还原目录）。

/// 唯一写入语句：登记会话归属（幂等，已登记不覆盖）。
///
/// `created_at` 仅作审计留存，不参与任何查询判定。
fn register_session_meta(
    conn: &rusqlite::Connection,
    session_id: &str,
    project_tag: &str,
    project_path: &str,
    created_at: &str,
) -> rusqlite::Result<bool> {
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO session_meta (session_id, project_tag, project_path, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![session_id, project_tag, project_path, created_at],
    )?;
    Ok(inserted > 0)
}

/// 会话诞生点归属登记（**唯一公开入口**）。返回是否本次写入
/// （false = 已登记过，或当前未配置项目目录）。
///
/// 未配置项目目录（`preferences.project_dir` 为空）→ 不登记：宁可缺失，不可错记。
pub fn register_session_project(session_id: &str) -> crate::Result<bool> {
    let guard = crate::store::db::acquire()?;
    register_session_project_with_conn(&guard, session_id)
}

/// 同上，复用调用方**已持有**的池连接（记忆写入路径已持一连接，再 acquire 会额外
/// 占用连接池，池满时互相等待 30s 超时）。
pub fn register_session_project_with_conn(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> crate::Result<bool> {
    let Some((tag, dir)) = crate::utils::active_project() else {
        return Ok(false);
    };
    let created_at = chrono::Utc::now().to_rfc3339();
    Ok(register_session_meta(
        conn,
        session_id,
        &tag,
        &dir,
        &created_at,
    )?)
}

/// 读取单个会话的归属路径：无归属行、或旧惰性登记留下的无路径行 → None。
pub fn session_project_path(session_id: &str) -> crate::Result<Option<String>> {
    let guard = crate::store::db::acquire()?;
    let mut stmt = guard.prepare(
        "SELECT project_path FROM session_meta
         WHERE session_id = ?1 AND project_path IS NOT NULL AND project_path != ''",
    )?;
    let mut rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
    match rows.next() {
        Some(Ok(path)) => Ok(Some(path)),
        _ => Ok(None),
    }
}

/// 批量读取归属路径（session_id → 路径）。无归属 / 无路径的会话不出现在结果中，
/// 调用方据此把它们归入「未分组」。
pub fn session_project_paths(ids: &[String]) -> crate::Result<HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let guard = crate::store::db::acquire()?;
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT session_id, project_path FROM session_meta
         WHERE project_path IS NOT NULL AND project_path != ''
           AND session_id IN ({placeholders})"
    );
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

// ══════════════════════════════════════════════════════════════════════════
// 历史会话归属回填（一次性、幂等）
// ══════════════════════════════════════════════════════════════════════════
//
// 背景：`project_path` 是后来才加的展示列，旧库里的历史会话只有 `project_tag`
// （记忆过滤仍以 tag 为准，语义不变）。标签含路径哈希、**不可逆**，因此回填只做
// 「tag ↔ 已知候选目录」的精确匹配：候选目录全部来自用户已确认过的数据（书签 /
// 当前项目目录 / 已登记过的归属路径），匹配不上就保持无归属（前端归「未分组」）。
//
// 禁止事项（与既有硬约束一致）：不得按当前目录 / 会话内容 / 时间推断归属，
// 不得改写 `project_tag`，不得覆盖已有 `project_path`。

/// 待回填清单：`project_path` 为 NULL/空的归属行 → `(session_id, project_tag)`。
pub fn sessions_missing_project_path(
    conn: &rusqlite::Connection,
) -> crate::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, project_tag FROM session_meta
         WHERE project_path IS NULL OR project_path = ''",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 回填候选目录（按可靠度排序，全部来自用户已确认过的数据）：
/// ① 项目书签路径（含归档——归档是展示维度，归属是数据）
/// ② 当前项目目录
/// ③ 已登记过 `project_path` 的会话去重路径集合（自愈：同一 tag 一旦解析成功即可复用）。
pub fn backfill_candidate_dirs(
    conn: &rusqlite::Connection,
    bookmarks: &[crate::config::ProjectBookmark],
    project_dir: &str,
) -> crate::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let push = |dir: &str, out: &mut Vec<String>| {
        let dir = dir.trim();
        if !dir.is_empty() && !out.iter().any(|d| d == dir) {
            out.push(dir.to_string());
        }
    };
    for b in bookmarks {
        push(&b.path, &mut out);
    }
    push(project_dir, &mut out);

    let mut stmt = conn.prepare(
        "SELECT DISTINCT project_path FROM session_meta
         WHERE project_path IS NOT NULL AND project_path != ''",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    for path in rows {
        push(&path?, &mut out);
    }
    Ok(out)
}

/// 回填主体：对 `session_meta` 中无 `project_path` 的行，用其 `project_tag` 与候选目录
/// 精确匹配，命中才写（写的是命中候选目录的原始写法，保证 tag ↔ path 同源同值）。
///
/// 幂等：命中行写完即不再出现在待填清单里，且 UPDATE 自带空值守卫（见下），
/// 因此重复调用第二次零变更。返回本次写入的行数。
/// 既有 `project_path` 的行既不在待填清单内、也不会被 UPDATE 命中 —— 不可能被覆盖。
pub fn backfill_session_project_paths_with_conn(
    conn: &rusqlite::Connection,
    candidates: &[String],
) -> crate::Result<usize> {
    let pending = sessions_missing_project_path(conn)?;
    if pending.is_empty() || candidates.is_empty() {
        return Ok(0);
    }
    // 同一 tag 只解析一次（自愈复用：一次命中即可喂给同 tag 的其余行）
    let mut resolved: HashMap<String, String> = HashMap::new();
    let mut filled = 0usize;
    for (session_id, tag) in pending {
        let path = match resolved.get(&tag) {
            Some(p) => Some(p.clone()),
            None => match crate::utils::dir_for_project_tag(&tag, candidates) {
                Some(p) => {
                    resolved.insert(tag.clone(), p.clone());
                    Some(p)
                }
                None => None,
            },
        };
        // 匹配不上 → 保持无归属（不猜测）；命中的 tag 已算过，不再重复尝试
        let Some(path) = path else { continue };
        let updated = conn.execute(
            "UPDATE session_meta SET project_path = ?1
             WHERE session_id = ?2 AND (project_path IS NULL OR project_path = '')",
            params![path, session_id],
        )?;
        filled += updated;
    }
    Ok(filled)
}

/// 生产入口：启动时调用一次（幂等）。候选目录 = 书签（含归档）+ 当前项目目录 +
/// 已登记过的归属路径。失败由调用方降级（warn，不阻断启动）。
pub fn backfill_session_project_paths() -> crate::Result<usize> {
    let guard = crate::store::db::acquire()?;
    let prefs = crate::config::UserPreferences::load();
    let candidates = backfill_candidate_dirs(&guard, &prefs.project_bookmarks, &prefs.project_dir)?;
    backfill_session_project_paths_with_conn(&guard, &candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn random_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// 与生产表结构一致的内存 session_meta（db.rs DDL）——归属登记语义测试
    /// 不必污染真实 DB。
    fn setup_meta_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_meta (
                session_id      TEXT PRIMARY KEY,
                project_tag     TEXT NOT NULL,
                project_path    TEXT,
                created_at      TEXT NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    /// 归属 = 创建时快照：同一会话二次登记（不同目录）不得覆盖首次归属。
    #[test]
    fn register_session_meta_keeps_first_attribution() {
        let conn = setup_meta_conn();
        let id = random_id();

        assert!(
            register_session_meta(&conn, &id, "A-1a2b3c4d", "E:\\work\\A", "t1").unwrap(),
            "首次登记应写入"
        );
        assert!(
            !register_session_meta(&conn, &id, "B-5e6f7a8b", "E:\\work\\B", "t2").unwrap(),
            "二次登记（切目录后）应被忽略并返回 false"
        );

        let (tag, path): (String, String) = conn
            .query_row(
                "SELECT project_tag, project_path FROM session_meta WHERE session_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(tag, "A-1a2b3c4d", "既有归属 tag 不得被改写");
        assert_eq!(path, "E:\\work\\A", "既有归属路径不得被改写");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_meta WHERE session_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "同一会话只允许一行归属");
    }

    /// 无归属会话（历史会话）→ None / 空：不猜测、不伪造。
    #[serial]
    #[test]
    fn session_project_path_none_for_unregistered_session() {
        let id = random_id();
        assert_eq!(
            session_project_path(&id).unwrap(),
            None,
            "未登记会话必须返回 None（前端归入未分组）"
        );
        assert!(
            session_project_paths(std::slice::from_ref(&id))
                .unwrap()
                .is_empty(),
            "未登记会话不得出现在批量归属结果中"
        );
        assert!(
            session_project_paths(&[]).unwrap().is_empty(),
            "空名单必须短路返回空"
        );
    }

    /// 旧惰性登记行（无 project_path）同样不得被当作归属：
    /// tag 不可逆，无法还原目录 → 与「无归属」同等处理。
    #[serial]
    #[test]
    fn legacy_meta_row_without_path_is_not_attribution() {
        let id = random_id();
        {
            let conn = crate::store::db::acquire().unwrap();
            conn.execute(
                "INSERT INTO session_meta (session_id, project_tag, project_path, created_at)
                 VALUES (?1, 'legacy-1a2b3c4d', NULL, 't')",
                params![id],
            )
            .unwrap();
        }

        assert_eq!(session_project_path(&id).unwrap(), None);
        assert!(session_project_paths(std::slice::from_ref(&id))
            .unwrap()
            .is_empty());

        let conn = crate::store::db::acquire().unwrap();
        conn.execute(
            "DELETE FROM session_meta WHERE session_id = ?1",
            params![id],
        )
        .unwrap();
    }

    /// 池连接入口与记忆检索同源：登记的 (tag, path) 必须等于当前 active_project()；
    /// 未配置项目目录 → 不登记（宁可缺失，不可错记）。
    #[serial]
    #[test]
    fn register_session_project_matches_active_project() {
        let id = random_id();
        let registered = register_session_project(&id).unwrap();

        match crate::utils::active_project() {
            Some((tag, dir)) => {
                assert!(registered, "配置了项目目录时应写入归属");
                let (stored_tag, stored_path): (String, String) = {
                    let conn = crate::store::db::acquire().unwrap();
                    conn.query_row(
                        "SELECT project_tag, project_path FROM session_meta WHERE session_id = ?1",
                        params![id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap()
                };
                assert_eq!(stored_tag, tag, "归属 tag 必须与记忆检索用的 tag 同值");
                assert_eq!(stored_path, dir, "归属路径必须与当前项目目录同值");
                assert_eq!(
                    session_project_path(&id).unwrap().as_deref(),
                    Some(dir.as_str())
                );
                // 再登记（模拟切目录后重入）仍返回 false 且不覆盖
                assert!(!register_session_project(&id).unwrap());
                let conn = crate::store::db::acquire().unwrap();
                conn.execute(
                    "DELETE FROM session_meta WHERE session_id = ?1",
                    params![id],
                )
                .unwrap();
            }
            None => assert!(!registered, "未配置项目目录时不得登记任何归属"),
        }
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

    /// 回归（任务链 87f4fc7a）：白名单保留语义。模拟「活跃会话持续对话 +
    /// 多成员驻留 shelf」：新建 12 个快照，保护其中 10 个驻留成员 →
    /// 恰清理名单外 2 个、返回值=2；二次调用幂等收敛；名单含不存在 id 不报错。
    /// 测试前库中既有的其他快照一并纳入保护名单——本测试绝不触碰用户现存数据。
    #[serial]
    #[test]
    fn prune_snapshots_whitelist_protects_members_and_cleans_rest() {
        let mut ids = Vec::new();
        for _ in 0..12 {
            let id = random_id();
            upsert_snapshot(&id, "leader", r#"{"t":1}"#).unwrap();
            ids.push(id);
        }

        // 既存快照（含真实库中用户数据）并入保护名单
        let existing: Vec<String> = list_snapshots(100_000)
            .unwrap()
            .into_iter()
            .map(|(id, _, _)| id)
            .filter(|id| !ids.contains(id))
            .collect();
        let mut protected = ids[..10].to_vec();
        protected.extend(existing.iter().cloned());

        let cleaned = prune_snapshots(&protected).unwrap();
        assert_eq!(cleaned, 2, "名单外恰有本次新建的 2 个应被清理");

        for (i, id) in ids[..10].iter().enumerate() {
            assert!(
                get_snapshot(id).unwrap().is_some(),
                "驻留成员 #{i} ({id}) 快照应幸存"
            );
        }
        for id in &ids[10..] {
            assert!(get_snapshot(id).unwrap().is_none(), "名单外 {id} 应被清");
        }
        for id in &existing {
            assert!(
                get_snapshot(id).unwrap().is_some(),
                "用户现存快照 {id} 不得被触碰"
            );
        }

        // 二次 prune 幂等收敛（无更多清理）
        let again = prune_snapshots(&protected).unwrap();
        assert_eq!(again, 0, "二次 prune 应幂等收敛");

        // 白名单含不存在的 id 不报错且零清除
        let mut with_ghost = protected.clone();
        with_ghost.push("ghost-id-does-not-exist".to_string());
        assert_eq!(prune_snapshots(&with_ghost).unwrap(), 0);

        for id in &ids {
            delete_session(id).unwrap();
        }
    }

    /// 回归（任务链 87f4fc7a）：空保护名单必须零清除（防御性短路）——
    /// 调用方拿不到状态时禁止静默清空全库快照。
    #[serial]
    #[test]
    fn prune_snapshots_empty_protection_clears_nothing() {
        let before: Vec<String> = list_snapshots(100_000)
            .unwrap()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();

        let n = prune_snapshots(&[]).unwrap();
        assert_eq!(n, 0, "空名单应短路返回 Ok(0)");

        let after: Vec<String> = list_snapshots(100_000)
            .unwrap()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(before.len(), after.len(), "空名单不得清除任何快照");
    }
}
