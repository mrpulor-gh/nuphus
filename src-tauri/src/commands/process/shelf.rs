//! Session Shelf —— 浅层会话展示台
//!
//! 内存 LRU（≤10）+ SQLite 完整快照（sessions.snapshot 列，方案A）。
//! 存储原始 Session 对象本身：切换 = 整对象换入换出，tool_use/tool_result
//! 配对由构造保证，不经过任何「重建/转换」路径（规避上下文正确性风险）。
//!
//! 切换守卫：1) !busy（执行中 agent 被 take 出 RuntimeContext）
//!           2) !mobile_append::has_pending()（追加队列在轮次边界消费，非空切走会丢）
//!           3) 同 backing mode（v1 不触碰 set_mode 联动语义）
//!
//! 持久化时机：归档（切换/新建让位）、任务完成回填、退出钩子。
//! 启动时惰性装载最近快照为 active（见 leader.rs 恢复链最前端）。
//! 旧磁盘镜像（config_dir/nuphus/sessions/{id}.json）由 migrate_legacy_mirrors
//! 幂等导入 SQLite 后保留不删（保守）。

use crate::state::AppState;
use nuphus::session::{MessageRole, Session};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::State;

/// 展示台容量上限
pub const SHELF_CAPACITY: usize = 10;

/// 旧磁盘镜像目录（迁移用：扫描导入 SQLite；导入后文件保留不删）
fn mirror_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("nuphus")
        .join("sessions")
}

/// 旧镜像文件包装（仅迁移解析用，新 IO 走 SQLite snapshot 列）
#[derive(Serialize, Deserialize)]
struct MirrorFile {
    mode: String,
    session: Session,
}

/// 单个槽位元数据
#[derive(Debug, Clone, Serialize)]
pub struct ShelfEntry {
    pub id: String,
    pub mode: String,
    pub title: String,
    pub message_count: usize,
    /// Unix 毫秒；最后一条消息 timestamp，缺省为归档时刻
    pub updated_at: u64,
}

/// 内存展示台。active 会话不在此处（活在 agent 里），命令层动态拼装。
#[derive(Default)]
pub struct ShelfState {
    /// newest-first
    pub order: Vec<String>,
    pub entries: HashMap<String, ShelfEntry>,
    pub sessions: HashMap<String, Session>,
    /// 重命名覆盖表（active 会话改名时先记此处在归档时生效）
    pub titles: HashMap<String, String>,
}

impl ShelfState {
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// 归档一个会话；返回被淘汰的 id（若有）。已存在则更新并提到最前。
    pub fn put(&mut self, entry: ShelfEntry, session: Session) -> Option<String> {
        let id = entry.id.clone();
        if let Some(pos) = self.order.iter().position(|x| x == &id) {
            self.order.remove(pos);
        }
        self.order.insert(0, id.clone());
        self.entries.insert(id.clone(), entry);
        self.sessions.insert(id.clone(), session);
        if self.order.len() > SHELF_CAPACITY {
            return self.order.pop();
        }
        None
    }

    /// 取出（换装到 agent 后从展示台移除）
    pub fn take(&mut self, id: &str) -> Option<(ShelfEntry, Session)> {
        let pos = self.order.iter().position(|x| x == id)?;
        self.order.remove(pos);
        let entry = self.entries.remove(id)?;
        let session = self.sessions.remove(id)?;
        Some((entry, session))
    }

    pub fn get(&self, id: &str) -> Option<&ShelfEntry> {
        self.entries.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }
}

// ── 纯函数辅助 ──

/// 默认标题：第一条可见 user 消息截断（跳过内部提示/追加段/提炼词/系统方括号前缀）
pub(crate) fn derive_title(session: &Session) -> String {
    for m in session.messages() {
        if !matches!(m.role, MessageRole::User) {
            continue;
        }
        let text = m.text_content();
        let t = text.trim();
        if t.is_empty()
            || m.internal
            || t.starts_with('[')
            || t.starts_with("开始进行上下文提炼")
            || nuphus::mobile_append::is_append_section(&text)
        {
            continue;
        }
        return truncate_chars(t, 30);
    }
    String::new()
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

/// 归属模式归一化："workflow" → workflow；其余（leader/custom）→ leader
pub(crate) fn normalize_mode(current_mode: &str) -> &'static str {
    if current_mode == "workflow" {
        "workflow"
    } else {
        "leader"
    }
}

/// 切换守卫。Err(稳定错误码) 供前端映射文案。
fn guard_switch(state: &AppState) -> Result<(), &'static str> {
    if state.busy.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("busy");
    }
    if nuphus::mobile_append::has_pending() {
        return Err("append_pending");
    }
    Ok(())
}

fn active_session<'a>(ctx: &'a crate::state::RuntimeContext, kind: &str) -> Option<&'a Session> {
    if kind == "workflow" {
        ctx.workflow_agent.as_ref().map(|a| a.session())
    } else {
        ctx.leader_agent.as_ref().map(|rt| rt.session())
    }
}

fn active_session_mut<'a>(
    ctx: &'a mut crate::state::RuntimeContext,
    kind: &str,
) -> Option<&'a mut Session> {
    if kind == "workflow" {
        ctx.workflow_agent.as_mut().map(|a| a.session_mut())
    } else {
        ctx.leader_agent.as_mut().map(|rt| rt.session_mut())
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// RFC3339 时间字符串 → Unix 毫秒（sessions.updated_at 为 RFC3339 文本）。
/// 解析失败返回 None，调用方回退 now_millis()。
fn rfc3339_to_millis(s: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis().max(0) as u64)
}

// ── 镜像 IO（SQLite 快照，best-effort，失败只 warn 不阻塞主流程）──

pub(crate) fn write_mirror(mode: &str, session: &Session) {
    match serde_json::to_string(session) {
        Ok(json) => {
            if let Err(e) = nuphus::store::session::upsert_snapshot(&session.id, mode, &json) {
                tracing::warn!("[Shelf] 写快照失败 id={}: {e}", session.id);
            }
            // 保留策略：SQLite 只留最近 SHELF_CAPACITY 个快照（轻量切换语义），
            // 超出清空 snapshot 列防无界增长；元数据行保留。best-effort。
            if let Err(e) = nuphus::store::session::prune_snapshots(SHELF_CAPACITY) {
                tracing::warn!("[Shelf] 快照保留策略执行失败: {e}");
            }
        }
        Err(e) => tracing::warn!("[Shelf] 序列化快照失败 id={}: {e}", session.id),
    }
}

pub(crate) fn read_mirror(id: &str) -> Option<(String, Session)> {
    let Ok(Some((mode, json))) = nuphus::store::session::get_snapshot(id) else {
        return None;
    };
    let session: Session = serde_json::from_str(&json).ok()?;
    Some((mode, session))
}

fn delete_mirror(id: &str) {
    let _ = nuphus::store::session::delete_snapshot(id);
}

/// 启动恢复：SQLite 中最新快照（按 updated_at）。供 leader.rs 恢复链最前端调用。
pub(crate) fn load_latest_mirror() -> Option<(String, Session)> {
    let Ok(Some((mode, json))) = nuphus::store::session::latest_snapshot() else {
        return None;
    };
    let session: Session = serde_json::from_str(&json).ok()?;
    if session.is_empty() {
        return None;
    }
    Some((mode, session))
}

/// 启动预热：SQLite 快照装回内存展示台（≤10 个最新），供列表命令直接消费。
/// updated_at 使用 sessions 表时间（RFC3339），非文件 mtime。
pub(crate) fn warm_from_disk(shelf: &mut ShelfState) {
    let Ok(snapshots) = nuphus::store::session::list_snapshots(SHELF_CAPACITY) else {
        return;
    };
    for (id, mode, updated_at) in snapshots {
        let Ok(Some((_, json))) = nuphus::store::session::get_snapshot(&id) else {
            continue;
        };
        let Ok(file_session) = serde_json::from_str::<Session>(&json) else {
            continue;
        };
        if file_session.is_empty()
            || shelf.contains(&file_session.id)
            || shelf.len() >= SHELF_CAPACITY
        {
            continue;
        }
        let entry = ShelfEntry {
            id: file_session.id.clone(),
            mode,
            title: derive_title(&file_session),
            message_count: file_session.messages().len(),
            updated_at: rfc3339_to_millis(&updated_at).unwrap_or_else(now_millis),
        };
        let id = entry.id.clone();
        shelf.entries.insert(id.clone(), entry);
        shelf.sessions.insert(id.clone(), file_session);
        shelf.order.insert(0, id);
    }
}

/// 旧磁盘镜像迁移：扫描 mirror_dir()/*.json（MirrorFile{mode,session} 格式），
/// 按文件修改时间倒序，仅对 sessions 表无 snapshot 的 id 导入（有则跳过）。
/// 文件解析失败仅 warn 不中断；旧文件保留不删。幂等（多次调用安全）。
pub(crate) fn migrate_legacy_mirrors() {
    let dir = mirror_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x == "json")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            e.metadata()
                .ok()
                .and_then(|m| m.modified().ok().map(|t| (t, e.path())))
        })
        .collect();
    files.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    let mut imported = 0usize;
    for (_, path) in files {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = serde_json::from_str::<MirrorFile>(&content) else {
            tracing::warn!("[Shelf] 旧镜像解析失败，跳过: {}", path.display());
            continue;
        };
        if file.session.is_empty() {
            continue;
        }
        // 已有快照则跳过（幂等，不覆盖已有数据）
        if let Ok(Some(_)) = nuphus::store::session::get_snapshot(&file.session.id) {
            continue;
        }
        if let Ok(json) = serde_json::to_string(&file.session) {
            if nuphus::store::session::upsert_snapshot(&file.session.id, &file.mode, &json).is_ok()
            {
                imported += 1;
            }
        }
    }
    if imported > 0 {
        tracing::info!("[Shelf] 旧镜像迁移完成，导入 {imported} 个快照");
    }
}

/// 元数据行 upsert（title 空串时保留已有 summary，与退出钩子语义一致）
pub(crate) fn upsert_meta_row(session: &Session, title: &str) {
    let existing = nuphus::store::session::get_session(&session.id)
        .ok()
        .flatten();
    let row = nuphus::store::session::SessionRow {
        id: session.id.clone(),
        parent_id: existing.as_ref().and_then(|r| r.parent_id.clone()),
        depth: existing
            .as_ref()
            .map(|r| r.depth)
            .unwrap_or(session.depth as i32),
        created_at: existing
            .as_ref()
            .map(|r| r.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        updated_at: chrono::Utc::now().to_rfc3339(),
        message_count: session.messages().len() as i32,
        token_count: session.api_input_tokens as i32,
        summary: if title.is_empty() {
            existing
                .as_ref()
                .map(|r| r.summary.clone())
                .unwrap_or_default()
        } else {
            title.to_string()
        },
    };
    let _ = nuphus::store::session::upsert_session(&row);
}

/// 元数据行 + 镜像一并落盘（退出钩子等调用方使用）
pub(crate) fn persist_and_mirror(kind: &str, session: &Session) {
    upsert_meta_row(session, "");
    write_mirror(kind, session);
}

// ── 命令层 ──

fn build_entry(
    id: String,
    mode: &str,
    session: &Session,
    title_override: Option<&str>,
) -> ShelfEntry {
    ShelfEntry {
        id,
        mode: mode.to_string(),
        title: title_override
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.to_string())
            .unwrap_or_else(|| derive_title(session)),
        message_count: session.messages().len(),
        updated_at: session
            .messages()
            .last()
            .and_then(|m| m.timestamp)
            .unwrap_or_else(now_millis),
    }
}

/// 归档 active 到展示台 + 镜像 + 元数据行。空会话跳过（不占槽）。
fn archive_active(state: &AppState, ctx: &mut crate::state::RuntimeContext, kind: &str) {
    let Some(sess_ref) = active_session(ctx, kind) else {
        return;
    };
    if sess_ref.is_empty() {
        return;
    }
    let snapshot = sess_ref.clone();
    let title = state
        .shelf
        .lock()
        .ok()
        .and_then(|s| s.titles.get(&snapshot.id).cloned());
    let entry = build_entry(snapshot.id.clone(), kind, &snapshot, title.as_deref());
    write_mirror(kind, &snapshot);
    upsert_meta_row(&snapshot, &entry.title);
    if let Ok(mut shelf) = state.shelf.lock() {
        if let Some(evicted) = shelf.put(entry, snapshot) {
            // 淘汰仅移除内存 LRU 条目；SQLite 快照永久保留，重启后仍可恢复
            tracing::info!("[Shelf] 淘汰最旧会话 {evicted}（SQLite 快照保留）");
        }
    }
}

/// 列出展示台：按 created_at 降序稳定排序（最新创建在上，切换/激活不改变位置，
/// 只通过 is_active 变化颜色/效果）。附 can_switch 供前端置灰。
#[tauri::command]
pub fn list_shelf_sessions(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    list_shelf_sessions_inner(&state)
}

/// 内部实现（&AppState 直取）：mobile_server 的会话清单镜像端点复用
pub(crate) fn list_shelf_sessions_inner(state: &AppState) -> Result<serde_json::Value, String> {
    let can_switch = guard_switch(state).is_ok();
    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    // 收集候选 (id, item_json, is_active)；active 与会话台条目统一参与稳定排序
    let mut candidates: Vec<(String, serde_json::Value, bool)> = Vec::new();
    let mut active_id: Option<String> = None;

    // active（runtime）：backup 中转残留路径下同一 id 可能同时在 runtime 与 shelf，
    // 以 active 为准展示，shelf 循环跳过同 id 去重。
    //
    // 空 messages 的 active 会话也作为 active 返回：0825-02 修复后 SessionRail 5s 轮询
    // 比对 active id 变化以感知外部会话切换，若 active 在「empty → non-empty」
    // 之间跳变，会被误判为外部变更触发无意义重拉。保持 active id 从创建那一刻起
    // 稳定，让 SessionRail 只在真正切换时刷新。
    if let Ok(ctx) = state.runtime.lock() {
        if let Some(sess) = active_session(&ctx, kind) {
            let title = state
                .shelf
                .lock()
                .ok()
                .and_then(|s| s.titles.get(&sess.id).cloned())
                .unwrap_or_default();
            let e = build_entry(sess.id.clone(), kind, sess, Some(&title));
            active_id = Some(e.id.clone());
            candidates.push((
                e.id.clone(),
                serde_json::json!({
                    "id": e.id, "mode": e.mode, "title": e.title,
                    "message_count": e.message_count, "updated_at": e.updated_at,
                    "is_active": true,
                }),
                true,
            ));
        }
    }

    if let Ok(shelf) = state.shelf.lock() {
        for id in &shelf.order {
            if active_id.as_deref() == Some(id.as_str()) {
                continue;
            }
            let Some(e) = shelf.get(id) else { continue };
            candidates.push((
                e.id.clone(),
                serde_json::json!({
                    "id": e.id, "mode": e.mode, "title": e.title,
                    "message_count": e.message_count, "updated_at": e.updated_at,
                    "is_active": false,
                }),
                false,
            ));
        }
    }

    // 稳定排序：created_at 降序（最新创建在上）；缺失/解析失败排最后；同时间按 id 保序。
    let created_at_map = nuphus::store::session::list_created_at(
        &candidates
            .iter()
            .map(|c| c.0.clone())
            .collect::<Vec<String>>(),
    )
    .unwrap_or_default();
    candidates.sort_by(|a, b| {
        let ta = created_at_map
            .get(&a.0)
            .and_then(|s| rfc3339_to_millis(s))
            .unwrap_or(0);
        let tb = created_at_map
            .get(&b.0)
            .and_then(|s| rfc3339_to_millis(s))
            .unwrap_or(0);
        tb.cmp(&ta).then_with(|| a.0.cmp(&b.0))
    });

    Ok(serde_json::json!({
        "can_switch": can_switch,
        "items": candidates.into_iter().map(|(_, v, _)| v).collect::<Vec<_>>(),
    }))
}

/// 切换会话。守卫/归属校验失败返回稳定错误码字符串（busy / append_pending /
/// mode_mismatch / not_found），前端映射文案。无 agent 槽（重启后新进程
/// leader/workflow 槽为空）时降级 backup 中转成功返回，不再报 no_agent。
#[tauri::command]
pub fn switch_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    switch_session_inner(&state, id)
}

/// 手机端跟随广播：会话切换后经 mobile WS 通道通知（mobile_server 未启动时 no-op）。
/// 镜像模型：手机不维护独立会话状态，收到 SessionChanged 后重拉 /history，
/// 呈现桌面当前会话。帧格式与 CompoundEmitter 的 WS 分支一致（裸 NuphusEvent JSON）。
fn broadcast_session_changed_mobile(state: &AppState, session_id: &str) {
    use nuphus::agent::events::EventEmitter;
    let Some(tx) = state.mobile_ws_tx.lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    crate::emitter::MobileWsEmitter::new(tx).emit(
        nuphus::agent::events::NuphusEvent::SessionChanged {
            session_id: session_id.to_string(),
        },
    );
}

/// 内部实现（&AppState 直取）：mobile_server 的遥控切换端点复用
pub(crate) fn switch_session_inner(state: &AppState, id: String) -> Result<(), String> {
    guard_switch(state).map_err(|c| c.to_string())?;

    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    // 目标归属校验（内存中的条目）
    {
        let shelf = state.shelf.lock().map_err(|e| e.to_string())?;
        if let Some(e) = shelf.get(&id) {
            if e.mode != kind {
                return Err("mode_mismatch".to_string());
            }
        }
    }

    // 取出目标：优先内存展示台，回落磁盘镜像
    let taken = {
        let mut shelf = state.shelf.lock().map_err(|e| e.to_string())?;
        shelf.take(&id)
    };
    let (entry, target_session) = match taken {
        Some(pair) => pair,
        None => match read_mirror(&id) {
            Some((mode, session)) => {
                if mode != kind {
                    return Err("mode_mismatch".to_string());
                }
                let title = state
                    .shelf
                    .lock()
                    .ok()
                    .and_then(|s| s.titles.get(&session.id).cloned());
                (
                    build_entry(session.id.clone(), &mode, &session, title.as_deref()),
                    session,
                )
            }
            None => return Err("not_found".to_string()),
        },
    };

    let mut ctx = state.runtime.lock().map_err(|e| e.to_string())?;

    archive_active(state, &mut ctx, kind);

    let Some(slot) = active_session_mut(&mut ctx, kind) else {
        // 无 agent 槽可装（重启/build 后新进程 leader/workflow 槽为 None，agent 仅在
        // 发送消息时才创建）：
        // 降级为 backup 中转——与 resume_latest_session 同机制：目标会话序列化进
        // session_backup，前端 get_chat_history 经 backup 回退路径显示目标历史；
        // 下次发消息时 run_runtime_with_config 从 session_backup_json 恢复完整上下文
        // （含 ToolUse/ToolResult，非 text-only）。
        // 目标放回展示台避免 rail 丢条目（take/put 仅动内存，磁盘镜像不动，重启仍可恢复）。
        let sid = entry.id.clone();
        if let Ok(json) = serde_json::to_string(&target_session) {
            if let Ok(mut sb) = state.session.lock() {
                sb.session_backup = Some(json);
                sb.last_message.clear();
                sb.last_message_images.clear();
            }
        }
        if let Ok(mut shelf) = state.shelf.lock() {
            shelf.put(entry, target_session);
        }
        tracing::info!("[Shelf] 无 agent 槽，降级 backup 中转切换会话 {sid} ({kind})");
        broadcast_session_changed_mobile(state, &sid);
        return Ok(());
    };
    *slot = target_session;

    tracing::info!("[Shelf] 切换到会话 {} ({kind})", entry.id);
    broadcast_session_changed_mobile(state, &entry.id);
    Ok(())
}

/// 新建对话：归档当前（有内容才占槽）→ 安装空白会话，返回新 id
/// 广播：CompoundEmitter 双推（桌面 Tauri IPC + 手机 WS）——手机「新建对话」遥控桌面
/// 走同一入口，变更经 SessionChanged 事件回传，双端跟随显示（单一路径，手机跟随）。
#[tauri::command]
pub fn new_chat_session_cmd(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    new_chat_session_with_event(&app, state.inner())
}

/// 内部实现（&AppState 直取）：mobile_server 的 /new-chat 端点复用（避免构造 tauri State）
pub(crate) fn new_chat_session_with_event<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
) -> Result<String, String> {
    guard_switch(state).map_err(|c| c.to_string())?;

    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    let mut ctx = state.runtime.lock().map_err(|e| e.to_string())?;
    archive_active(state, &mut ctx, kind);

    let fresh = Session::new();
    let new_id = fresh.id.clone();
    if let Some(slot) = active_session_mut(&mut ctx, kind) {
        *slot = fresh;
    }
    drop(ctx);

    // 会话边界必须同步清理恢复快照与消息去重键。否则新会话第一条消息若恰好与
    // 上一会话末条相同，会被 completion dedup 当成重复提交而静默丢弃。
    if let Ok(mut sb) = state.session.lock() {
        sb.session_backup = None;
        sb.last_message.clear();
        sb.last_send_id = None;
        sb.last_message_images.clear();
    }
    tracing::info!("[Shelf] 新建对话 {new_id} ({kind})");
    use nuphus::agent::events::EventEmitter;
    crate::emitter::CompoundEmitter::new(app.clone(), state).emit(
        nuphus::agent::events::NuphusEvent::SessionChanged {
            session_id: new_id.clone(),
        },
    );
    Ok(new_id)
}

/// 重命名：覆盖表 + 元数据行；对 active 会话立即生效（归档时沿用）
#[tauri::command]
pub fn rename_session_cmd(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    let title = title.trim().to_string();
    if title.is_empty() || title.chars().count() > 60 {
        return Err("invalid_title".to_string());
    }
    {
        let mut shelf = state.shelf.lock().map_err(|e| e.to_string())?;
        shelf.titles.insert(id.clone(), title.clone());
        if let Some(e) = shelf.entries.get_mut(&id) {
            e.title = title.clone();
        }
    }
    let existing = nuphus::store::session::get_session(&id).ok().flatten();
    let row = nuphus::store::session::SessionRow {
        created_at: existing
            .as_ref()
            .map(|r| r.created_at.clone())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        summary: title,
        ..existing.unwrap_or(nuphus::store::session::SessionRow {
            id: id.clone(),
            parent_id: None,
            depth: 0,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            message_count: 0,
            token_count: 0,
            summary: String::new(),
        })
    };
    nuphus::store::session::upsert_session(&row).map_err(|e| e.to_string())
}

/// 用户手动归档：把 rail 中指定会话移出展示台并清快照（元数据行+文本记忆保留可查）。
/// 与 LRU 淘汰语义一致，由用户主动触发（前端非 active 条目显示归档按钮 + 确认弹窗）。
/// active 会话在 runtime 不在 shelf，无法经此归档（前端不显示按钮）。错误码：
/// busy / append_pending / not_found。
#[tauri::command]
pub fn archive_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    guard_switch(&state).map_err(|c| c.to_string())?;

    let removed = {
        let mut shelf = state.shelf.lock().map_err(|e| e.to_string())?;
        shelf.take(&id)
    };
    let Some((entry, session)) = removed else {
        return Err("not_found".to_string());
    };
    // 元数据行确保落库（记忆页列表可查）；快照清空（与 LRU 淘汰一致：rail 移除后
    // 不再保留完整执行上下文，对话文本记忆仍可经记忆页/搜索查看）
    upsert_meta_row(&session, &entry.title);
    delete_mirror(&id);
    tracing::info!("[Shelf] 用户手动归档会话 {} ({})", entry.id, entry.mode);
    Ok(())
}

/// 是否存在可恢复的最近会话镜像（leader 归属、非空）——欢迎页「继续对话」按钮显示条件
#[tauri::command]
pub fn has_resume_candidate() -> bool {
    matches!(
        load_latest_mirror(),
        Some((mode, ref s)) if mode != "workflow" && !s.is_empty()
    )
}

/// 「继续对话」：把最新 leader 镜像写入 session_backup——
/// 1) get_chat_history 的 backup 回退路径立即返回完整历史（无需构建 Runtime，
///    欢迎页保持存在，恢复是显式用户动作）
/// 2) 下一条消息提交时 run_runtime_with_config 经同一 JSON 恢复完整上下文
///    （retry.rs 同一先例），新指令即续聊
#[tauri::command]
pub fn resume_latest_session(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::HistoryMessage>, String> {
    let Some((mode, sess)) = load_latest_mirror() else {
        return Err("no_resume".to_string());
    };
    if mode == "workflow" || sess.is_empty() {
        return Err("no_resume".to_string());
    }
    let json = serde_json::to_string(&sess).map_err(|e| e.to_string())?;
    {
        let mut sb = state.session.lock().map_err(|e| e.to_string())?;
        sb.session_backup = Some(json);
        sb.last_message.clear();
        sb.last_message_images.clear();
    }
    crate::commands::process::session::chat_history(&state)
}

/// 任务完成后的回填（crash 安全）：active 会话镜像刷盘 + 元数据行实时落库。
/// 元数据行不再依赖退出方式——此前仅托盘 quit 才写，点 ✕ 隐藏/杀进程的用户
/// 记忆页会话列表永远为空。失败不影响执行结果上报。
pub fn flush_active_mirror(state: &AppState) {
    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);
    if let Ok(ctx) = state.runtime.lock() {
        if let Some(sess) = active_session(&ctx, kind) {
            if !sess.is_empty() {
                write_mirror(kind, sess);
                upsert_meta_row(sess, &derive_title(sess));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_user(texts: &[&str]) -> Session {
        let mut s = Session::new();
        for t in texts {
            s.push_user(t.to_string());
            s.push_assistant(vec![nuphus::session::ContentBlock::Text {
                text: "好".to_string(),
                reasoning: None,
            }]);
        }
        s
    }

    #[test]
    fn derive_title_skips_internal_and_takes_first_real_user() {
        let mut s = Session::new();
        s.push_user_internal("[系统提示] 内部注入".to_string());
        s.push_user("开始进行上下文提炼".to_string());
        s.push_user("帮我重构路由层".to_string());
        assert_eq!(derive_title(&s), "帮我重构路由层");
    }

    #[test]
    fn derive_title_truncates_long_text() {
        let long = "这是一条非常长的用户消息用来测试截断逻辑是否正常工作并且不会 panic 超出限制";
        let t = derive_title(&session_with_user(&[long]));
        assert!(t.chars().count() <= 31);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn shelf_put_evicts_oldest_and_take_removes() {
        let mut shelf = ShelfState::default();
        let mut evicted = Vec::new();
        for i in 0..=SHELF_CAPACITY {
            let s = session_with_user(&[&format!("会话{i}")]);
            let entry = ShelfEntry {
                id: s.id.clone(),
                mode: "leader".into(),
                title: format!("会话{i}"),
                message_count: s.messages().len(),
                updated_at: now_millis(),
            };
            if let Some(e) = shelf.put(entry, s) {
                evicted.push(e);
            }
        }
        // 放入 11 个 → 淘汰 1 个（最旧的「会话0」）
        assert_eq!(evicted.len(), 1);
        assert_eq!(shelf.len(), SHELF_CAPACITY);
        // take 后不再存在
        let first_id = shelf.order[0].clone();
        assert!(shelf.take(&first_id).is_some());
        assert!(!shelf.contains(&first_id));
    }

    #[test]
    fn snapshot_roundtrip_preserves_session() {
        // SQLite 快照往返（Session::new 生成随机 id，测试结束删除整行清理，不污染真实库）
        let s = session_with_user(&["快照往返测试"]);
        write_mirror("leader", &s);
        let (mode, restored) = read_mirror(&s.id).expect("快照应可读回");
        assert_eq!(mode, "leader");
        assert_eq!(restored.id, s.id);
        assert_eq!(restored.messages().len(), s.messages().len());
        delete_mirror(&s.id);
        assert!(read_mirror(&s.id).is_none(), "delete 后 read 应为 None");
        let _ = nuphus::store::session::delete_session(&s.id);
    }

    #[test]
    fn warm_from_disk_loads_snapshots_from_sqlite() {
        // 写两个快照（不同 updated_at），warm_from_disk 应从 SQLite 装回内存展示台
        let a = session_with_user(&["快照A"]);
        let b = session_with_user(&["快照B"]);
        write_mirror("leader", &a);
        std::thread::sleep(std::time::Duration::from_millis(5));
        write_mirror("workflow", &b);

        let mut shelf = ShelfState::default();
        warm_from_disk(&mut shelf);
        assert!(shelf.contains(&a.id), "A 应被装载");
        assert!(shelf.contains(&b.id), "B 应被装载");
        let entry_b = shelf.get(&b.id).expect("B 应有条目");
        assert_eq!(entry_b.mode, "workflow", "mode 应来自快照");

        let _ = nuphus::store::session::delete_session(&a.id);
        let _ = nuphus::store::session::delete_session(&b.id);
    }

    #[test]
    fn load_latest_mirror_prefers_most_recent_snapshot() {
        let a = session_with_user(&["旧快照"]);
        let b = session_with_user(&["新快照"]);
        write_mirror("leader", &a);
        std::thread::sleep(std::time::Duration::from_millis(5));
        write_mirror("workflow", &b);

        let (mode, latest) = load_latest_mirror().expect("应有最新快照");
        assert_eq!(latest.id, b.id, "最新写入的快照应优先");
        assert_eq!(mode, "workflow");

        let _ = nuphus::store::session::delete_session(&a.id);
        let _ = nuphus::store::session::delete_session(&b.id);
    }

    #[test]
    fn migrate_legacy_mirrors_imports_old_files_idempotent() {
        // 构造旧格式镜像文件（MirrorFile{mode,session}，随机 id），写临时目录
        let dir = mirror_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let s = session_with_user(&["旧镜像导入测试"]);
        let sid = s.id.clone();
        let file_path = dir.join(format!("legacy-{sid}.json"));
        let legacy = serde_json::json!({ "mode": "leader", "session": s });
        std::fs::write(&file_path, serde_json::to_string(&legacy).unwrap()).unwrap();

        migrate_legacy_mirrors();
        // get_snapshot 返回 Result<Option<(mode, snapshot_json)>>
        let imported = nuphus::store::session::get_snapshot(&sid)
            .expect("查询 SQLite 快照失败")
            .expect("旧镜像应导入 SQLite 快照");
        assert_eq!(imported.0, "leader", "导入的 mode 应保留");

        // 幂等：再次迁移不覆盖已有快照
        migrate_legacy_mirrors();
        let again = nuphus::store::session::get_snapshot(&sid)
            .expect("二次查询 SQLite 快照失败")
            .expect("二次查询应命中已有快照");
        assert_eq!(again.0, imported.0);
        assert_eq!(again.1, imported.1, "幂等迁移不得改变已有快照内容");

        // 清理：删除临时文件 + SQLite 行
        let _ = std::fs::remove_file(&file_path);
        let _ = nuphus::store::session::delete_session(&sid);
    }

    #[test]
    fn normalize_mode_maps_custom_to_leader() {
        assert_eq!(normalize_mode("workflow"), "workflow");
        assert_eq!(normalize_mode("leader"), "leader");
        assert_eq!(normalize_mode("custom-agent-x"), "leader");
    }

    /// 重启后新进程 leader_agent/workflow_agent 槽为 None（agent 仅在发送消息时创建）。
    /// switch_session 此时不应报 no_agent，而降级 backup 中转：session_backup 写入目标、
    /// 目标放回展示台。前端经 backup 回退路径显示历史，下次发消息经 JSON 恢复上下文。
    #[test]
    fn switch_session_without_agent_falls_back_to_backup() {
        let state = AppState::default();
        let target = session_with_user(&["目标会话"]);
        let target_id = target.id.clone();
        let target_msg_count = target.messages().len();
        let entry = ShelfEntry {
            id: target_id.clone(),
            mode: "leader".into(),
            title: "目标会话".into(),
            message_count: target_msg_count,
            updated_at: now_millis(),
        };
        {
            let mut shelf = state.shelf.lock().unwrap();
            shelf.put(entry, target);
        }
        let r = switch_session_inner(&state, target_id.clone());
        assert!(r.is_ok(), "无 agent 槽时应降级成功: {:?}", r.err());

        // 目标会话已写入 session_backup
        let sb = state.session.lock().unwrap();
        let backup = sb
            .session_backup
            .as_ref()
            .expect("session_backup 应被写入目标会话");
        let restored: Session = serde_json::from_str(backup).expect("backup 应为合法 Session");
        assert_eq!(restored.id, target_id);
        assert_eq!(restored.messages().len(), target_msg_count);
        drop(sb);

        // 目标放回展示台，rail 不丢条目
        let shelf = state.shelf.lock().unwrap();
        assert!(shelf.contains(&target_id), "目标应放回展示台");
    }
}
