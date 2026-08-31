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
use nuphus::agent::events::{EventEmitter, NuphusEvent};
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
    /// hover 预览：最后一条可见消息脱敏截断（≤400 字符），与标题「话题 ↔ 细节」互补
    pub preview: String,
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

/// 默认标题：**最后一条**可见 user 消息截断（跳过内部提示/追加段/提炼词/系统方括号前缀）。
/// 反向取最近话题——首条 user 作标题会随对话演进失真，最后一条常读常新（2026-08-27 设计）。
/// 自定义标题（rename_session_cmd 持久化到 store.summary）优先级不受影响。
pub(crate) fn derive_title(session: &Session) -> String {
    for m in session.messages().iter().rev() {
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

/// 预览脱敏：疑似密钥/token 的词元打码——`sk-`/`ghp_`/`gho_`/`xox`/`github_pat_` 前缀、
/// `Bearer` 授权头、≥32 位连续字母数字串（JWT/hex）。rail 常驻展示，防敏感信息上屏。
fn sanitize_preview(s: &str) -> String {
    let is_token_char = |c: char| c.is_alphanumeric() || c == '-' || c == '_' || c == '.';
    let sensitive_prefixes = [
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "bearer",
    ];
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if is_token_char(ch) {
            let start = idx;
            let mut end = idx + ch.len_utf8();
            while let Some(&(j, c2)) = chars.peek() {
                if is_token_char(c2) {
                    end = j + c2.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let word = &s[start..end];
            let lower = word.to_lowercase();
            let masked = sensitive_prefixes.iter().any(|p| lower.starts_with(p))
                || (word.len() >= 32
                    && word
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.'));
            out.push_str(if masked { "***" } else { word });
        } else {
            out.push(ch);
        }
    }
    out
}

/// 会话预览：**agent 最终回复**脱敏截断（≤400 字符）——hover 呈现「这个会话产出了什么」，
/// 与派生标题（最后一轮 user，短）形成「话题 ↔ 结果」互补（2026-08-27 大王定调）。
/// rail 可见状态下会话的最后完整消息几乎总是 assistant 回复（执行中 rail 隐藏）；
/// 无回复时（新会话/发送失败等边缘态）回退最后一条可见 user 消息。assistant 侧剥离
/// thinking 块与泄漏的工具 XML；user 侧沿用 derive_title 的可见性过滤。
pub(crate) fn derive_preview(session: &Session) -> String {
    // 第一优先：最后一条可见 assistant 消息（agent 最终回复）
    for m in session.messages().iter().rev() {
        if m.internal || !matches!(m.role, MessageRole::Assistant) {
            continue;
        }
        let text = m.text_content();
        let t = nuphus::utils::strip_tool_xml_tags(&nuphus::utils::strip_think_tags(&text));
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        return truncate_chars(&sanitize_preview(t), 400);
    }
    // 回退：无 assistant 回复时取最后一条可见 user 消息
    for m in session.messages().iter().rev() {
        if m.internal || !matches!(m.role, MessageRole::User) {
            continue;
        }
        let text = m.text_content();
        let t = text.trim();
        if t.is_empty()
            || t.starts_with('[')
            || t.starts_with("开始进行上下文提炼")
            || nuphus::mobile_append::is_append_section(&text)
        {
            continue;
        }
        return truncate_chars(&sanitize_preview(t), 400);
    }
    // 回退 2：仅剩提炼摘要的会话——refine 后旧历史清空、只剩 internal System 摘要
    // （replace_with_distill / accumulate_distill），前两循环全部跳过导致预览恒空
    // （实测回归）。摘要本身就是「这个会话浓缩了什么」，剥离元说明前缀后展示。
    if session.is_refined() {
        for m in session.messages().iter().rev() {
            if m.internal || !matches!(m.role, MessageRole::System) {
                continue;
            }
            let text = m.text_content();
            let t = text.trim();
            if t.is_empty() {
                continue;
            }
            let body = t
                .strip_prefix(nuphus::session::session::REFINE_SYSTEM_PREFIX)
                .map(|s| s.trim())
                .unwrap_or(t);
            if body.is_empty() {
                continue;
            }
            return truncate_chars(&sanitize_preview(body), 400);
        }
    }
    String::new()
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

/// 归属模式归一化：workflow → workflow；custom → custom（custom 会话走 leader 主循环，
/// 但 mode 标签须保留 custom，保证展示台/镜像/切换不丢失身份）；其余（leader/free/plan 残留）→ leader
pub(crate) fn normalize_mode(current_mode: &str) -> &'static str {
    if current_mode == "workflow" {
        "workflow"
    } else if current_mode == "custom" {
        "custom"
    } else {
        "leader"
    }
}

// ── 快照保护名单（prune 白名单）──
//
// prune_snapshots 按「当前可恢复名单」裁剪 SQLite 快照，名单必须显式覆盖：
//   ① runtime 内 leader / workflow 两个 active 会话槽
//   ② shelf 内存展示台全部驻留成员（order 全量）
//   ③ session_backup JSON 中转持有的会话（解析失败忽略该项）
// 此前按 updated_at 截断的保留策略曾系统性误杀长期驻留成员的快照
// （active 时间戳被每轮执行刷新、静坐成员时间戳冻结），回归见任务链 87f4fc7a。

/// 保护名单公共收集段。调用方负责自身的 runtime 锁序：已持 runtime 锁的场景
/// 必须经 [`protected_snapshot_ids_with_ctx`] 传入 active id，禁止嵌套加锁。
fn collect_protected(
    leader_id: Option<String>,
    workflow_id: Option<String>,
    state: &AppState,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(id) = leader_id {
        out.push(id);
    }
    if let Some(id) = workflow_id {
        out.push(id);
    }
    // ② shelf 驻留成员全量（含被 LRU 淘汰前的全部在台成员）
    if let Ok(shelf) = state.shelf.lock() {
        out.extend(shelf.order.iter().cloned());
    }
    // ③ session_backup 中转会话（半解析 JSON 取 id 字段；失败忽略该项）
    if let Ok(sb) = state.session.lock() {
        if let Some(json) = sb.session_backup.as_deref() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
                if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                    out.push(id.to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 已持 runtime 读侧锁场景的保护名单收集：active 会话从 ctx 直取，
/// 不再触碰 runtime 锁（archive_active 等调用方持锁期间专用）。
pub(crate) fn protected_snapshot_ids_with_ctx(
    ctx: &crate::state::RuntimeContext,
    state: &AppState,
) -> Vec<String> {
    collect_protected(
        ctx.leader_agent.as_ref().map(|rt| rt.session().id.clone()),
        ctx.workflow_agent.as_ref().map(|a| a.session().id.clone()),
        state,
    )
}

/// 自主获取 runtime 锁的保护名单收集。调用方不得已持有 runtime 锁时禁止使用
/// （std Mutex 不可重入）——先收集名单、再进入长锁段。
/// runtime 锁中毒时降级：至少保住 shelf 全员与 backup 中转。
pub(crate) fn protected_snapshot_ids(state: &AppState) -> Vec<String> {
    match state.runtime.lock() {
        Ok(ctx) => protected_snapshot_ids_with_ctx(&ctx, state),
        Err(_) => collect_protected(None, None, state),
    }
}

/// 切换守卫。Err(稳定错误码) 供前端映射文案。
/// pub(crate)：mobile_server /new-chat 纯意图广播复用同一守卫（busy/append 拒绝）。
pub(crate) fn guard_switch(state: &AppState) -> Result<(), &'static str> {
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

pub(crate) fn write_mirror(mode: &str, session: &Session, protected: &[String]) {
    match serde_json::to_string(session) {
        Ok(json) => {
            if let Err(e) = nuphus::store::session::upsert_snapshot(&session.id, mode, &json) {
                tracing::warn!("[Shelf] 写快照失败 id={}: {e}", session.id);
            }
            // 保留策略：SQLite 快照集合以「当前可恢复名单」（runtime active ∪
            // shelf 全员 ∪ backup 中转，由调用方收集传入）为准做白名单裁剪，
            // 名单外清空 snapshot 列防无界增长；元数据行保留。best-effort。
            if let Err(e) = nuphus::store::session::prune_snapshots(protected) {
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
        // 标题回读：优先 DB 已存标题（用户改过名），为空才派生默认——此前无条件
        // derive_title，重启后自定义标题被打回第一条 user 消息（实测回归）
        let stored_title = nuphus::store::session::get_session(&file_session.id)
            .ok()
            .flatten()
            .map(|r| r.summary)
            .filter(|s| !s.is_empty());
        let entry = ShelfEntry {
            id: file_session.id.clone(),
            mode,
            title: stored_title
                .clone()
                .unwrap_or_else(|| derive_title(&file_session)),
            preview: derive_preview(&file_session),
            message_count: file_session.messages().len(),
            updated_at: rfc3339_to_millis(&updated_at).unwrap_or_else(now_millis),
        };
        let id = entry.id.clone();
        // 回填钉住表：titles 是内存态，重启即清空；不回填的话，后续 flush/
        // archive 的兜底派生路径会再次无视自定义标题
        if let Some(t) = stored_title {
            shelf.titles.insert(id.clone(), t);
        }
        shelf.entries.insert(id.clone(), entry);
        shelf.sessions.insert(id.clone(), file_session);
        // order 保持 newest-first（与 ShelfState::put 语义一致）：list_snapshots
        // 返回 updated_at DESC（最新在前），逐个 append 到末尾 → order[0]=最新、
        // 末尾=最旧；此后 put 超限 pop() 移除的正是最旧（回归 2026-08-30：
        // 此前 insert(0) 把顺序倒转，重启后首次 put 会误淘汰「最新」）。
        shelf.order.push(id);
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

/// 元数据行 + 镜像一并落盘（退出钩子等调用方使用）。
/// `protected` 由调用方先行收集（[`protected_snapshot_ids`]），避免钩子内
/// 嵌套获取 runtime 锁。
pub(crate) fn persist_and_mirror(kind: &str, session: &Session, protected: &[String]) {
    upsert_meta_row(session, "");
    write_mirror(kind, session, protected);
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
        preview: derive_preview(session),
        message_count: session.messages().len(),
        updated_at: session
            .messages()
            .last()
            .and_then(|m| m.timestamp)
            .unwrap_or_else(now_millis),
    }
}

/// 归档 active 到展示台 + 镜像 + 元数据行。空会话跳过（不占槽）。
/// 注意：调用方持有 runtime 锁期间传入 ctx——保护名单经
/// protected_snapshot_ids_with_ctx 从 ctx 直取，绝不嵌套加锁。
pub(crate) fn archive_active(state: &AppState, ctx: &mut crate::state::RuntimeContext, kind: &str) {
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
    let protected = protected_snapshot_ids_with_ctx(ctx, state);
    write_mirror(kind, &snapshot, &protected);
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
            // mode 以存储归属为准（sessions 表快照列随 upsert_snapshot 绑定），
            // 不依赖 current_mode 推断——跨 mode 切换后 active 会话的真实归属
            // 仍以持久化记录为准；新会话尚未持久化时 fallback 当前 kind。
            let stored_mode = nuphus::store::session::get_snapshot(&sess.id)
                .ok()
                .flatten()
                .map(|(m, _)| m)
                .unwrap_or_else(|| kind.to_string());
            let title = state
                .shelf
                .lock()
                .ok()
                .and_then(|s| s.titles.get(&sess.id).cloned())
                .unwrap_or_default();
            let e = build_entry(sess.id.clone(), &stored_mode, sess, Some(&title));
            active_id = Some(e.id.clone());
            candidates.push((
                e.id.clone(),
                serde_json::json!({
                    "id": e.id, "mode": e.mode, "title": e.title,
                    "preview": e.preview,
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
                    "preview": e.preview,
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
///
/// `mode` 为可选目标 mode：跨 mode 会话切换由后端原子完成（归档原槽 →
/// 切 current_mode → 安装目标），前端**不再**先 set_mode 再 switch_session
/// 两次 IPC——此前 split 调用存在竞态：set_mode 触发的 mode_changed 事件
/// 会抢先 reloadChatFromBackend，若 switch_session 随后失败，聊天区与
/// mode chip 已错乱（回归 2026-08-30）。
#[tauri::command]
pub fn switch_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    mode: Option<String>,
) -> Result<(), String> {
    let before = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    switch_session_inner_mode(&state, id, mode)?;
    let after = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    // 跨 mode 会话切换（current_mode 变化）→ 广播 ModeChanged 双推（桌面+手机跟随）。
    // 手机端依赖 ModeChanged 同步 mode 显示；原子切换此前只广播 SessionChanged，手机
    // 端 mode 显示滞后（回归 2026-08-30）。
    if normalize_mode(&before) != normalize_mode(&after) {
        let emitter = crate::emitter::CompoundEmitter::new(app, &state);
        emitter.emit(NuphusEvent::ModeChanged {
            mode: after.clone(),
        });
    }
    // 会话台点击 = 显式选择已有会话：会话归属由发送时「输入框 mode vs session 绑定
    // mode」实时比较判定（规则2），不再需要 pending 状态机（2026-08-30 解耦）。
    Ok(())
}

/// 手机端跟随广播：会话切换后经 mobile WS 通道通知（mobile_server 未启动时 no-op）。
/// 镜像模型：手机不维护独立会话状态，收到 SessionChanged 后重拉 /history，
/// 呈现桌面当前会话。帧格式与 CompoundEmitter 的 WS 分支一致（裸 NuphusEvent JSON）。
fn broadcast_session_changed_mobile(state: &AppState, session_id: &str) {
    let Some(tx) = state.mobile_ws_tx.lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    crate::emitter::MobileWsEmitter::new(tx).emit(
        nuphus::agent::events::NuphusEvent::SessionChanged {
            session_id: session_id.to_string(),
        },
    );
}

/// 切换会话核心。`requested_mode` 为 None = 同 mode 切换（手机/测试兼容）；
/// Some(target) = 跨 mode 原子切换——先归档**原 mode** 的 active 会话，再切
/// current_mode，最后把目标安装进目标 mode 槽，全程一次加锁无竞态窗口。
/// 目标归属与目标 mode 不符返回稳定错误码 mode_mismatch。
pub(crate) fn switch_session_inner_mode(
    state: &AppState,
    id: String,
    requested_mode: Option<String>,
) -> Result<(), String> {
    guard_switch(state).map_err(|c| c.to_string())?;

    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let current_kind = normalize_mode(&current_mode);
    let target_kind = requested_mode
        .as_deref()
        .map(normalize_mode)
        .unwrap_or(current_kind);

    // 目标归属校验（内存中的条目）
    {
        let shelf = state.shelf.lock().map_err(|e| e.to_string())?;
        if let Some(e) = shelf.get(&id) {
            if e.mode != target_kind {
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
                if mode != target_kind {
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

    // 跨 mode：先切换 current_mode（在归档/安装之前，确保此后 get_chat_history
    // 与后续命令按目标 mode 路由）
    if target_kind != current_kind {
        if let Ok(mut cm) = state.current_mode.write() {
            *cm = target_kind.to_string();
        }
    }

    let mut ctx = state.runtime.lock().map_err(|e| e.to_string())?;

    // 归档**当前（原）**mode 的 active 会话——跨 mode 时必须归档用户正在离开的
    // 槽位，而非目标槽位（此前用切换后的 kind 归档，会把原会话留在原槽不归档，
    // 展示台/恢复链错乱：切走 leader 会话后 leader 槽仍驻留旧会话）
    archive_active(state, &mut ctx, current_kind);

    let Some(slot) = active_session_mut(&mut ctx, target_kind) else {
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
        tracing::info!(
            "[Shelf] 无 agent 槽，降级 backup 中转切换会话 {sid} ({current_kind} -> {target_kind})"
        );
        broadcast_session_changed_mobile(state, &sid);
        return Ok(());
    };
    *slot = target_session;

    tracing::info!(
        "[Shelf] 切换到会话 {} ({current_kind} -> {target_kind})",
        entry.id
    );
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
    app: tauri::AppHandle,
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
    nuphus::store::session::upsert_session(&row).map_err(|e| e.to_string())?;
    // 展示台列表变化 → 双端同步（手机刷新会话清单标题；当前会话未变）
    crate::emitter::CompoundEmitter::new(app, state.inner())
        .emit(nuphus::agent::events::NuphusEvent::ShelfUpdated);
    Ok(())
}

/// 用户手动归档：把 rail 中指定会话移出展示台并清快照（元数据行+文本记忆保留可查）。
/// 与 LRU 淘汰语义一致，由用户主动触发（前端非 active 条目显示归档按钮 + 确认弹窗）。
/// active 会话在 runtime 不在 shelf，无法经此归档（前端不显示按钮）。错误码：
/// busy / append_pending / not_found。
#[tauri::command]
pub fn archive_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
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
    // 展示台列表变化 → 双端同步（手机刷新会话清单；当前会话未变，手机不重拉历史）
    crate::emitter::CompoundEmitter::new(app, state.inner())
        .emit(nuphus::agent::events::NuphusEvent::ShelfUpdated);
    Ok(())
}

/// 是否存在可恢复的最近会话镜像（leader/workflow/custom 全 mode 支持）——
/// 欢迎页「继续对话」按钮显示条件：只看重启前最后对话镜像是否非空，
/// 不按 mode 排除（2026-08-30 起全 mode 统一支持继续对话）。
#[tauri::command]
pub fn has_resume_candidate() -> bool {
    matches!(
        load_latest_mirror(),
        Some((_, ref s)) if !s.is_empty()
    )
}

/// 「继续对话」：把最新镜像写入 session_backup——
/// 1) get_chat_history 的 backup 回退路径立即返回完整历史（无需构建 Runtime，
///    欢迎页保持存在，恢复是显式用户动作）
/// 2) 下一条消息提交时 run_runtime_with_config 经同一 JSON 恢复完整上下文
///    （retry.rs 同一先例），新指令即续聊
/// 3) current_mode 跟随镜像 mode（leader/workflow/custom 均支持）——重启后
///    mode 先落镜像，随后用户选择（继续对话/会话台/手动 chip）覆盖
#[tauri::command]
pub fn resume_latest_session(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::HistoryMessage>, String> {
    let Some((mode, sess)) = load_latest_mirror() else {
        return Err("no_resume".to_string());
    };
    if sess.is_empty() {
        return Err("no_resume".to_string());
    }
    let json = serde_json::to_string(&sess).map_err(|e| e.to_string())?;
    {
        let mut sb = state.session.lock().map_err(|e| e.to_string())?;
        sb.session_backup = Some(json);
        sb.last_message.clear();
        sb.last_message_images.clear();
    }
    // 镜像 mode 同步为当前权威（跨 mode 恢复：workflow/custom 会话不再被强制归 leader）
    if let Ok(mut cm) = state.current_mode.write() {
        *cm = mode.clone();
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
    // 保护名单先于 runtime 长锁收集（protected_snapshot_ids 内部需短暂 lock
    // runtime，先行完成后下方长锁段内不再触碰该锁）
    let protected = protected_snapshot_ids(state);
    if let Ok(ctx) = state.runtime.lock() {
        if let Some(sess) = active_session(&ctx, kind) {
            if !sess.is_empty() {
                write_mirror(kind, sess, &protected);
                // 标题保护：用户改过名 → 钉死自定义标题；否则保留 meta 既有
                // 标题，仅首次落库才写派生默认。此前每轮 derive_title 强制覆盖，
                // 是「编辑后切换/执行一轮，标题打回默认」的根因（实测回归）。
                match state
                    .shelf
                    .lock()
                    .ok()
                    .and_then(|s| s.titles.get(&sess.id).cloned())
                {
                    Some(custom) => upsert_meta_row(sess, &custom),
                    None => {
                        let exists = nuphus::store::session::get_session(&sess.id)
                            .map(|r| r.is_some())
                            .unwrap_or(false);
                        if exists {
                            // 空标题语义 = upsert 保留既有 summary，不覆盖
                            upsert_meta_row(sess, "");
                        } else {
                            upsert_meta_row(sess, &derive_title(sess));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockApiClient;
    #[async_trait::async_trait]
    impl nuphus::api::ApiClient for MockApiClient {
        async fn stream(
            &self,
            _request: nuphus::api::MessageRequest,
        ) -> nuphus::Result<Vec<nuphus::api::AssistantEvent>> {
            Ok(vec![])
        }
        fn model_name(&self) -> &str {
            "mock"
        }
        fn provider_kind(&self) -> nuphus::api::ProviderKind {
            nuphus::api::ProviderKind::MiniMax
        }
    }

    fn workflow_agent_with(sess: nuphus::session::Session) -> nuphus::runtime::WorkflowAgent {
        let mut wa = nuphus::runtime::WorkflowAgent::new(
            std::sync::Arc::new(MockApiClient),
            nuphus::ToolRegistry::work_agent(),
            None,
            None,
            "mock".to_string(),
            "user".to_string(),
            "Nuphus".to_string(),
            nuphus::permissions::ToolPermissions::default(),
            0.5,
        );
        wa.session_mut().replace_messages(sess.messages().to_vec());
        wa
    }

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
                preview: String::new(),
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
        write_mirror("leader", &s, &[]);
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
        write_mirror("leader", &a, &[]);
        // upsert_snapshot 的 updated_at 为 RFC3339 秒级精度——sleep 必须跨秒，
        // 否则两条快照时间戳相同、ORDER BY updated_at DESC 排序不稳定（回归 2026-08-30）
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_mirror("workflow", &b, &[]);

        let mut shelf = ShelfState::default();
        warm_from_disk(&mut shelf);
        assert!(shelf.contains(&a.id), "A 应被装载");
        assert!(shelf.contains(&b.id), "B 应被装载");
        let entry_b = shelf.get(&b.id).expect("B 应有条目");
        assert_eq!(entry_b.mode, "workflow", "mode 应来自快照");
        // order 必须 newest-first：最新（B）在 order[0]，较旧（A）排在其后——
        // 保证此后 put 超限 pop() 淘汰的是最旧而非最新（回归 2026-08-30）。
        // 注意：共享测试库可能存在其他测试残留快照，order 末尾不一定是 A，
        // 因此断言位置先后而非「A 恰在末尾」。
        let pos_a = shelf
            .order
            .iter()
            .position(|id| id == &a.id)
            .expect("A 应在 order 中");
        let pos_b = shelf
            .order
            .iter()
            .position(|id| id == &b.id)
            .expect("B 应在 order 中");
        assert_eq!(pos_b, 0, "最新快照 B 应在 order[0]");
        assert!(
            pos_a > pos_b,
            "较旧快照 A 应排在较新快照 B 之后（newest-first）"
        );

        let _ = nuphus::store::session::delete_session(&a.id);
        let _ = nuphus::store::session::delete_session(&b.id);
    }

    #[test]
    fn load_latest_mirror_prefers_most_recent_snapshot() {
        let a = session_with_user(&["旧快照"]);
        let b = session_with_user(&["新快照"]);
        write_mirror("leader", &a, &[]);
        // upsert_snapshot 的 updated_at 为 RFC3339 秒级精度——sleep 必须跨秒，
        // 否则两条快照时间戳相同、ORDER BY updated_at DESC 排序不稳定（Windows 偶发返回旧快照）
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_mirror("workflow", &b, &[]);

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

    /// 回归（任务链 87f4fc7a）：保护名单必须包含 shelf.order 全量驻留成员——
    /// prune 白名单漏掉任一在台成员都会导致其快照被误清、重启后从 rail 消失。
    #[test]
    fn protected_snapshot_ids_contains_all_shelf_order_entries() {
        let state = AppState::default();
        let want: Vec<String> = {
            let mut shelf = state.shelf.lock().unwrap();
            for i in 0..3 {
                let s = session_with_user(&[&format!("驻留成员{i}")]);
                let title = format!("驻留成员{i}");
                shelf.put(
                    build_entry(s.id.clone(), "leader", &s, Some(title.as_str())),
                    s,
                );
            }
            shelf.order.clone()
        };
        assert!(!want.is_empty(), "前置：shelf 应已有驻留成员");

        let got = protected_snapshot_ids(&state);
        for id in &want {
            assert!(got.contains(id), "保护名单应包含 shelf 驻留成员 {id}");
        }
    }

    /// 拉取会话台：active 条目 mode 必须来自存储快照归属（upsert_snapshot 写入的
    /// mode），不依赖 current_mode 推断。构造：current_mode=workflow + workflow_agent
    /// 槽内有会话，但该会话 SQLite 快照归属为 leader（跨 mode 切换后真实归属）。
    /// → active 条目 mode 应显示 leader，而非 workflow。
    #[test]
    fn list_shelf_active_mode_uses_stored_snapshot_not_current_mode() {
        let state = AppState::default();
        // current_mode = workflow
        {
            let mut cm = state.current_mode.write().unwrap();
            *cm = "workflow".to_string();
        }
        let sess = session_with_user(&["跨 mode 会话"]);
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.workflow_agent = Some(workflow_agent_with(sess));
        }
        // workflow_agent_with 内部创建新 Session（只复制消息、id 为新生成）——
        // 快照 key 必须用 agent 实际 session id，否则 get_snapshot 查不到
        // 而 fallback current_mode（回归 2026-08-30：断言拿到 workflow 而非 leader）
        let agent_sess_id = {
            let guard = state.runtime.lock().unwrap();
            guard.workflow_agent.as_ref().unwrap().session().id.clone()
        };
        // 存储快照归属 leader（upsert_snapshot 绑定 mode 与快照）
        let json = serde_json::to_string(&session_with_user(&["跨 mode 会话"])).unwrap();
        nuphus::store::session::upsert_snapshot(&agent_sess_id, "leader", &json).unwrap();

        let r = list_shelf_sessions_inner(&state).unwrap();
        let items = r["items"].as_array().unwrap();
        let active = items.iter().find(|i| i["is_active"] == true).unwrap();
        assert_eq!(
            active["mode"].as_str().unwrap(),
            "leader",
            "active 条目 mode 应来自存储快照归属，而非 current_mode"
        );

        let _ = nuphus::store::session::delete_session(&agent_sess_id);
    }

    /// 欢迎页「继续对话」：workflow 镜像也应显示按钮（全 mode 统一支持，
    /// 不再排除 workflow——只看重启前最后对话镜像是否非空）。
    #[test]
    fn has_resume_candidate_accepts_workflow_mirror() {
        let sess = session_with_user(&["工作流最后对话"]);
        write_mirror("workflow", &sess, &[]);
        assert!(
            has_resume_candidate(),
            "workflow 镜像也应可继续对话（全 mode 支持）"
        );
        let _ = nuphus::store::session::delete_session(&sess.id);
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
            preview: String::new(),
            message_count: target_msg_count,
            updated_at: now_millis(),
        };
        {
            let mut shelf = state.shelf.lock().unwrap();
            shelf.put(entry, target);
        }
        let r = switch_session_inner_mode(&state, target_id.clone(), None);
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
