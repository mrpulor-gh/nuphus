//! Session Shelf —— 浅层会话展示台
//!
//! 内存 LRU（≤10）+ 磁盘镜像（config_dir/nuphus/sessions/{id}.json）。
//! 存储原始 Session 对象本身：切换 = 整对象换入换出，tool_use/tool_result
//! 配对由构造保证，不经过任何「重建/转换」路径（规避上下文正确性风险）。
//!
//! 切换守卫：1) !busy（执行中 agent 被 take 出 RuntimeContext）
//!           2) !mobile_append::has_pending()（追加队列在轮次边界消费，非空切走会丢）
//!           3) 同 backing mode（v1 不触碰 set_mode 联动语义）
//!
//! 持久化时机：归档（切换/新建让位）、任务完成回填、退出钩子。
//! 启动时惰性装载最近镜像为 active（见 leader.rs 恢复链最前端）。

use crate::state::AppState;
use nuphus::session::{MessageRole, Session};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::State;

/// 展示台容量上限
pub const SHELF_CAPACITY: usize = 10;

fn mirror_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("nuphus")
        .join("sessions")
}

fn mirror_path(id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    mirror_dir().join(format!("{safe}.json"))
}

/// 镜像文件包装：记录 backing mode 以便跨重启恢复归属
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

// ── 镜像 IO（best-effort，失败只 warn 不阻塞主流程）──

pub(crate) fn write_mirror(mode: &str, session: &Session) {
    let dir = mirror_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("[Shelf] 创建镜像目录失败: {e}");
        return;
    }
    let file = MirrorFile {
        mode: mode.to_string(),
        session: session.clone(),
    };
    match serde_json::to_string_pretty(&file) {
        Ok(json) => {
            let final_path = mirror_path(&session.id);
            let tmp = final_path.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &final_path)) {
                tracing::warn!("[Shelf] 写镜像失败 id={}: {e}", session.id);
            }
        }
        Err(e) => tracing::warn!("[Shelf] 序列化镜像失败 id={}: {e}", session.id),
    }
}

pub(crate) fn read_mirror(id: &str) -> Option<(String, Session)> {
    let content = std::fs::read_to_string(mirror_path(id)).ok()?;
    let file: MirrorFile = serde_json::from_str(&content).ok()?;
    Some((file.mode, file.session))
}

fn delete_mirror(id: &str) {
    let _ = std::fs::remove_file(mirror_path(id));
}

/// 启动恢复：磁盘上最新的镜像（按 mtime）。供 leader.rs 恢复链最前端调用。
pub(crate) fn load_latest_mirror() -> Option<(String, Session)> {
    let dir = mirror_dir();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(mt) = e.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if best.as_ref().map(|(t, _)| mt > *t).unwrap_or(true) {
            best = Some((mt, p));
        }
    }
    let (_, path) = best?;
    let file: MirrorFile = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if file.session.is_empty() {
        return None;
    }
    Some((file.mode, file.session))
}

/// 启动预热：磁盘镜像装回内存展示台（≤10 个最新），供列表命令直接消费。
pub(crate) fn warm_from_disk(shelf: &mut ShelfState) {
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
    for (_, path) in files.into_iter().take(SHELF_CAPACITY) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = serde_json::from_str::<MirrorFile>(&content) else {
            continue;
        };
        if file.session.is_empty() || shelf.contains(&file.session.id) || shelf.len() >= SHELF_CAPACITY {
            continue;
        }
        let entry = ShelfEntry {
            id: file.session.id.clone(),
            mode: file.mode,
            title: derive_title(&file.session),
            message_count: file.session.messages().len(),
            updated_at: now_millis(),
        };
        let id = entry.id.clone();
        shelf.entries.insert(id.clone(), entry);
        shelf.sessions.insert(id.clone(), file.session);
        shelf.order.insert(0, id);
    }
}

/// 元数据行 upsert（title 空串时保留已有 summary，与退出钩子语义一致）
pub(crate) fn upsert_meta_row(session: &Session, title: &str) {
    let existing = nuphus::store::session::get_session(&session.id).ok().flatten();
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
            delete_mirror(&evicted);
            tracing::info!("[Shelf] 淘汰最旧会话 {evicted}");
        }
    }
}

/// 列出展示台：active 在首位，其后 newest-first。附 can_switch 供前端置灰。
#[tauri::command]
pub fn list_shelf_sessions(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let can_switch = guard_switch(&state).is_ok();
    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    let mut items: Vec<serde_json::Value> = Vec::new();

    if let Ok(ctx) = state.runtime.lock() {
        if let Some(sess) = active_session(&ctx, kind) {
            if !sess.is_empty() {
                let title = state
                    .shelf
                    .lock()
                    .ok()
                    .and_then(|s| s.titles.get(&sess.id).cloned())
                    .unwrap_or_default();
                let e = build_entry(sess.id.clone(), kind, sess, Some(&title));
                items.push(serde_json::json!({
                    "id": e.id, "mode": e.mode, "title": e.title,
                    "message_count": e.message_count, "updated_at": e.updated_at,
                    "is_active": true,
                }));
            }
        }
    }

    if let Ok(shelf) = state.shelf.lock() {
        for id in &shelf.order {
            let Some(e) = shelf.get(id) else { continue };
            items.push(serde_json::json!({
                "id": e.id, "mode": e.mode, "title": e.title,
                "message_count": e.message_count, "updated_at": e.updated_at,
                "is_active": false,
            }));
        }
    }

    Ok(serde_json::json!({ "can_switch": can_switch, "items": items }))
}

/// 切换会话。守卫/归属校验失败返回稳定错误码字符串（busy / append_pending /
/// mode_mismatch / not_found / no_agent），前端映射文案。
#[tauri::command]
pub fn switch_session(state: State<'_, AppState>, id: String) -> Result<(), String> {
    guard_switch(&state).map_err(|c| c.to_string())?;

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

    archive_active(&state, &mut ctx, kind);

    let Some(slot) = active_session_mut(&mut ctx, kind) else {
        // 无 agent 可装：目标放回展示台避免丢失
        if let Ok(mut shelf) = state.shelf.lock() {
            shelf.put(entry, target_session);
        }
        return Err("no_agent".to_string());
    };
    *slot = target_session;

    tracing::info!("[Shelf] 切换到会话 {} ({kind})", entry.id);
    Ok(())
}

/// 新建对话：归档当前（有内容才占槽）→ 安装空白会话，返回新 id
#[tauri::command]
pub fn new_chat_session_cmd(state: State<'_, AppState>) -> Result<String, String> {
    guard_switch(&state).map_err(|c| c.to_string())?;

    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    let mut ctx = state.runtime.lock().map_err(|e| e.to_string())?;
    archive_active(&state, &mut ctx, kind);

    let fresh = Session::new();
    let new_id = fresh.id.clone();
    if let Some(slot) = active_session_mut(&mut ctx, kind) {
        *slot = fresh;
    } else {
        return Err("no_agent".to_string());
    }
    tracing::info!("[Shelf] 新建对话 {new_id} ({kind})");
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

/// 任务完成后的镜像回填（crash 安全）：把 active 会话刷盘。
/// 由 process.rs 完成路径调用；失败不影响执行结果上报。
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
    fn mirror_roundtrip_preserves_session() {
        // 用临时目录验证序列化往返（mirror_dir 固定，测试后清理）
        let s = session_with_user(&["镜像往返测试"]);
        write_mirror("leader", &s);
        let (mode, restored) = read_mirror(&s.id).expect("镜像应可读回");
        assert_eq!(mode, "leader");
        assert_eq!(restored.id, s.id);
        assert_eq!(restored.messages().len(), s.messages().len());
        delete_mirror(&s.id);
        assert!(read_mirror(&s.id).is_none());
    }

    #[test]
    fn normalize_mode_maps_custom_to_leader() {
        assert_eq!(normalize_mode("workflow"), "workflow");
        assert_eq!(normalize_mode("leader"), "leader");
        assert_eq!(normalize_mode("custom-agent-x"), "leader");
    }
}
