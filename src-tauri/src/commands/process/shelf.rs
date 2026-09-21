//! Session Shelf —— 浅层会话展示台
//!
//! 内存 LRU（≤10）+ SQLite 完整快照（sessions.snapshot 列，方案A）。
//! 存储原始 Session 对象本身：切换 = 整对象换入换出，tool_use/tool_result
//! 配对由构造保证，不经过任何「重建/转换」路径（规避上下文正确性风险）。
//!
//! 切换守卫：1) !busy（执行中 agent 被 take 出 RuntimeContext）
//!           2) SignalState::append_queue 为空（追加队列在轮次边界消费，非空切走会丢）
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

/// 草稿对话（「新建项目文件夹」→ 立刻可开说、尚未诞生的一条空对话）。
///
/// 只活在内存（[`crate::state::SessionState::draft_session`]）：**不写** `sessions` 行 /
/// `session_meta` 归属行 / mirror / snapshot，所以进程退出即消失、重启后不会出现。
///
/// `project_path` 是**创建时的归属快照**（与真实会话诞生点 [`register_session_origin`]
/// 同源：`utils::active_project()` 读当前项目目录），展示台只用它回填 `project_path`——
/// **不按当前工作目录推断**：归属缺失就归「未分组」，不猜。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSession {
    /// 草稿 id：仅供 rail 条目标识与「当前」判定（点击恒为 no-op）；真实会话诞生时另铸 uuid
    pub id: String,
    /// 诞生时的 mode（归一化后与展示台 kind 同域）：mode 不匹配时不得冒充 active
    pub mode: String,
    /// 诞生时的项目目录快照（创建时已保证非空——无项目目录直接拒绝创建草稿）
    pub project_path: String,
    /// 创建时刻（Unix 毫秒）：条目的 created_at / updated_at 都用它，轮询期间保持稳定
    pub created_at: u64,
}

/// 项目文件夹分组条目（会话工作台「项目文件夹」数据源）。
///
/// `path` 是分组键：与 `items[].project_path` 精确对应；无归属会话（path 为 null）
/// 由前端归入「未分组」——后端不猜测、不伪造归属。
#[derive(Debug, Clone, Serialize)]
pub struct ProjectEntry {
    /// 归属目录（与 items[].project_path 同值）
    pub path: String,
    /// 展示名：书签自定义名优先，自动组取目录末段
    pub name: String,
    /// 当前工作目录（前端仅高亮，不上浮；排序仍按书签顺序）
    pub is_current: bool,
    /// true = 未收藏但有会话的自动组（只读组，不写入书签）
    pub auto: bool,
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
    /// 是否已从磁盘完成预热（`warm_from_disk` 成功路径置位）。
    ///
    /// false = 内存名单不可信（启动早期 / 预热失败）→ `collect_protected` 返回空名单，
    /// 使快照裁剪走「空名单短路」零裁剪。禁止用残缺内存态反推删除持久化快照。
    pub warmed: bool,
}

impl ShelfState {
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// 入台（归档/装载）一个会话；返回被淘汰的 id（若有）。已存在则更新并提到最前。
    ///
    /// 容量语义：驻留上限 [`SHELF_CAPACITY`]（10），超限淘汰最旧，并**同时清掉
    /// `entries` / `sessions` 的内存残留**——此前只 `pop` 了 `order`，另两张表仍留着
    /// 该 id，形成幽灵成员（`len()` 与 `order` 口径漂移 → 快照白名单口径跟着漂）。
    ///
    /// 被淘汰者的**快照处置由调用方决定**：`archive_active` 走主动归档（显式、可追溯），
    /// 其余路径由 `prune_snapshots` 兜底（受残缺防护约束）。
    pub fn put(&mut self, entry: ShelfEntry, session: Session) -> Option<String> {
        let id = entry.id.clone();
        if let Some(pos) = self.order.iter().position(|x| x == &id) {
            self.order.remove(pos);
        }
        self.order.insert(0, id.clone());
        self.entries.insert(id.clone(), entry);
        self.sessions.insert(id.clone(), session);
        if self.order.len() > SHELF_CAPACITY {
            let evicted = self.order.pop();
            if let Some(ref e) = evicted {
                self.entries.remove(e);
                self.sessions.remove(e);
            }
            return evicted;
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
    // ── 预热门禁（2026-09-21 用户会话丢失修复）──
    // 展示台尚未从磁盘预热完成时，内存态名单不可信（可能是空或残缺）。此时返回
    // 空名单，让上层 `prune_snapshots` 走「空名单短路」→ 本轮零裁剪。
    // 裁剪的输入必须来自已证明可信的内存态，禁止用未预热的状态反推删除持久化快照。
    match state.shelf.lock() {
        Ok(shelf) if shelf.warmed => {}
        Ok(_) => {
            tracing::warn!("[Shelf] 展示台尚未预热，本轮跳过镜像裁剪（返回空名单）");
            return Vec::new();
        }
        Err(_) => {
            tracing::warn!("[Shelf] 展示台锁中毒，本轮跳过镜像裁剪（返回空名单）");
            return Vec::new();
        }
    }

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
    if !nuphus::state::SignalState::read(&state.signals)
        .append_queue
        .is_empty()
    {
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
    // 预热完成（即使一个成员都没装回，也说明「读盘已成功、内存态可信」）。
    // list_snapshots 失败时函数在上方提前 return，warmed 保持 false → 禁止裁剪。
    shelf.warmed = true;
}

/// 旧磁盘镜像迁移：扫描 mirror_dir()/*.json（MirrorFile{mode,session} 格式），
/// 按文件修改时间倒序，仅对 sessions 表无 snapshot 的 id 导入（已有则只做退场）。
/// 文件解析失败仅 warn 不中断。**处理完毕即改名退场（`.json.migrated`）**，使迁移
/// 只生效一次——否则这些会话的快照被裁剪/归档清空后，会在下次启动「复活占位」，
/// 并把 updated_at 刷成启动时刻、长期霸占驻留位。幂等（已退场文件不再被扫描命中）。
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
    let mut retired = 0usize;
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
        // 已有快照则不覆盖（保留 DB 现有数据）；但仍要走下方退场标记——
        // 否则「快照将来被裁剪/归档清空」时它会再次满足导入条件而复活。
        let has_snapshot = matches!(
            nuphus::store::session::get_snapshot(&file.session.id),
            Ok(Some(_))
        );
        if !has_snapshot {
            if let Ok(json) = serde_json::to_string(&file.session) {
                if nuphus::store::session::upsert_snapshot(&file.session.id, &file.mode, &json)
                    .is_ok()
                {
                    imported += 1;
                }
            }
        }
        // ── 退场标记（2026-09-21 僵尸复活修复）──
        // 旧行为「文件保留不删」+ 导入条件只看「DB 里有无快照」= 死循环复活：
        // 一旦这些会话的快照被裁剪或归档清空，下次启动又满足条件 → 再次导入，且
        // upsert_snapshot 把 updated_at 刷成启动时刻 → 永远占据驻留位前几席，
        // 把近期会话挤出可恢复名单。改名后扩展名不再是 .json（扫描只认 .json），
        // 迁移天然只生效一次；文件本体保留，可回滚或人工检查。
        if let Some(renamed) = retire_migrated_file(&path) {
            retired += 1;
            tracing::info!(
                "[Shelf] 旧镜像退场: {} -> {}",
                path.display(),
                renamed.display()
            );
        }
    }
    if imported > 0 {
        tracing::info!("[Shelf] 旧镜像迁移完成，导入 {imported} 个快照");
    }
    if retired > 0 {
        tracing::info!("[Shelf] 旧镜像退场 {retired} 个（已改名 .json.migrated，不再参与扫描）");
    }
}

/// 旧镜像退场标记：`*.json` → `*.json.migrated`（改名而非删除，保留可回滚）。
/// 目标同名已存在时追加序号，绝不覆盖既有文件；改名失败返回 None（下次启动重试，
/// 不会丢数据）。退场后该文件不再被 `migrate_legacy_mirrors` 的 `.json` 扫描命中。
fn retire_migrated_file(path: &std::path::Path) -> Option<PathBuf> {
    let mut target = path.with_extension("json.migrated");
    if target.exists() {
        let base = target.display().to_string();
        target = (1..1000)
            .map(|i| PathBuf::from(format!("{base}.{i}")))
            .find(|c| !c.exists())?;
    }
    std::fs::rename(path, &target).ok()?;
    Some(target)
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

/// 将当前会话加入展示台列表。当前会话可能来自 runtime agent，也可能来自
/// `session_backup`（重启后首次切换 Workflow、或执行中 agent 被暂时 take 出槽位）。
/// 统一构造路径，确保两种来源的 active 条目字段完全一致。
fn push_active_candidate(
    state: &AppState,
    kind: &str,
    session: &Session,
    candidates: &mut Vec<(String, serde_json::Value, bool)>,
    active_id: &mut Option<String>,
    created_fallback: &mut HashMap<String, u64>,
) {
    // mode 以存储归属为准；新会话尚未持久化时回退到当前模式。
    let stored_mode = nuphus::store::session::get_snapshot(&session.id)
        .ok()
        .flatten()
        .map(|(mode, _)| mode)
        .unwrap_or_else(|| kind.to_string());
    let title = state
        .shelf
        .lock()
        .ok()
        .and_then(|s| s.titles.get(&session.id).cloned())
        .unwrap_or_default();
    let entry = build_entry(session.id.clone(), &stored_mode, session, Some(&title));
    *active_id = Some(entry.id.clone());
    created_fallback.insert(
        entry.id.clone(),
        session
            .messages()
            .first()
            .and_then(|m| m.timestamp)
            .unwrap_or(entry.updated_at),
    );
    candidates.push((
        entry.id.clone(),
        serde_json::json!({
            "id": entry.id,
            "mode": entry.mode,
            "title": entry.title,
            "preview": entry.preview,
            "message_count": entry.message_count,
            "updated_at": entry.updated_at,
            "is_active": true,
        }),
        true,
    ));
}

/// 读取会话列表的 backup 当前会话。该 backup 是 chat_history 在 runtime 槽为空时
/// 使用的权威回退，因此列表也必须与聊天区保持同一当前会话。
fn read_backup_session(state: &AppState) -> Option<Session> {
    let json = state.session.lock().ok()?.session_backup.clone()?;
    serde_json::from_str(&json).ok()
}

// ── 项目文件夹分组（会话工作台）──

/// 路径相等判定：忽略首尾空白与结尾分隔符；Windows 下忽略大小写。
/// 分组键比较用（书签路径由用户挑选、归属路径由配置快照，两者大小写/尾斜杠可能不同）。
fn same_project_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_string();
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

fn project_entry(path: &str, name: &str, current_dir: &str, auto: bool) -> ProjectEntry {
    ProjectEntry {
        path: path.trim().trim_end_matches(['\\', '/']).to_string(),
        name: if name.trim().is_empty() {
            nuphus::utils::dir_display_name(path)
        } else {
            name.trim().to_string()
        },
        is_current: same_project_path(path, current_dir),
        auto,
    }
}

/// 组装项目文件夹组：返回 `(可见组, 已归档组)`。
///
/// - 可见组 = 未归档书签（**按书签表顺序**，不退化成最近使用序）+ 未收藏但有会话的
///   自动组（按会话出现顺序，排在全部书签组之后，只读不落库）；
/// - 归属路径与书签路径重合时以书签为准（名字 / 顺序），不重复成组；
/// - 已归档书签不出现在可见组（归档 = 隐藏），单独返回供「已归档文件夹」恢复入口；
/// - 未出现在 `session_paths` 里的会话（含无归属会话）不产生任何组：不猜测。
pub(crate) fn build_project_groups(
    bookmarks: &[nuphus::config::ProjectBookmark],
    current_dir: &str,
    session_paths: &[String],
) -> (Vec<ProjectEntry>, Vec<ProjectEntry>) {
    let mut visible: Vec<ProjectEntry> = Vec::new();
    let mut archived: Vec<ProjectEntry> = Vec::new();

    for bm in bookmarks {
        if bm.path.trim().is_empty() {
            // 手改配置可能留空条目：跳过，避免生出无名幽灵组
            continue;
        }
        let entry = project_entry(&bm.path, &bm.name, current_dir, false);
        if bm.archived {
            archived.push(entry);
        } else {
            visible.push(entry);
        }
    }

    for path in session_paths {
        if path.trim().is_empty() {
            continue;
        }
        // 已收藏（含已归档）目录由书签组代表，不再生成自动组
        if bookmarks.iter().any(|b| same_project_path(&b.path, path)) {
            continue;
        }
        if visible.iter().any(|e| same_project_path(&e.path, path)) {
            continue; // 同目录多会话 → 只一个自动组
        }
        visible.push(project_entry(path, "", current_dir, true));
    }

    (visible, archived)
}

/// 会话诞生点归属登记：转发到 store 的唯一写入入口（创建时快照，幂等）。
///
/// 失败仅告警不阻断对话——归属缺失只影响分组展示（归入「未分组」），
/// 不得让登记失败影响会话本身。
pub(crate) fn register_session_origin(session_id: &str) {
    match nuphus::store::session::register_session_project(session_id) {
        Ok(true) => tracing::info!("[Shelf] 会话归属登记: {session_id}"),
        Ok(false) => {
            tracing::debug!("[Shelf] 会话 {session_id} 未登记归属（已登记过或未配置项目目录）")
        }
        Err(e) => tracing::warn!("[Shelf] 会话归属登记失败 {session_id}: {e}"),
    }
}

/// 会话诞生点一次性登记：归属快照（既有语义，见 [`register_session_origin`]）+
/// 「新建对话」弹窗记录的标题（见 [`apply_recorded_title`]）。
///
/// 调用点只有两处（leader 与 workflow 各自的诞生分支），二者都要求「全新 uuid + 空
/// session」；恢复 / 续聊路径不经过这里，因此归属与标题都不会改写既有会话。
///
/// 真实会话一诞生，草稿对话（[`DraftSession`]）的历史使命即结束：清掉它，
/// 否则 rail 上会同时存在「已开说的真实会话」与「等待开说的草稿」两条当前对话。
pub(crate) fn register_session_birth(state: &AppState, session: &Session) {
    register_session_origin(&session.id);
    apply_recorded_title(state, session);
    clear_draft_session(state);
}

// ── 草稿对话（新建项目文件夹 → 立刻可开说的空对话，内存态）──

/// 记录草稿对话。同一时刻最多一条（覆盖式写入：重新创建项目即换新草稿）。
pub(crate) fn set_draft_session(state: &AppState, draft: DraftSession) {
    if let Ok(mut sb) = state.session.lock() {
        sb.draft_session = Some(draft);
    }
}

/// 读草稿对话（展示台组装与测试用；不做任何 mode / 槽位有效性判定）。
pub(crate) fn draft_session(state: &AppState) -> Option<DraftSession> {
    state
        .session
        .lock()
        .ok()
        .and_then(|sb| sb.draft_session.clone())
}

/// 清掉草稿对话：任何把「当前对话」从草稿移开的操作都走这里——
/// 切换会话（成功路径）/ 新建对话（回欢迎页）/「继续对话」装载最近会话 /
/// 真实会话在诞生点落成。退出进程时内存态随进程消失，无需显式清理。
pub(crate) fn clear_draft_session(state: &AppState) {
    if let Ok(mut sb) = state.session.lock() {
        if let Some(dropped) = sb.draft_session.take() {
            tracing::debug!("[Shelf] 草稿对话 {} 结束（当前对话已移开）", dropped.id);
        }
    }
}

/// 草稿诞生核心（`path` = 已切好的当前项目目录；测试可直接注入）：
/// 全新 uuid + 归属快照 + 当前 mode，登记进内存态。**不写任何持久化**。
pub(crate) fn create_draft_session(
    state: &AppState,
    mode: &str,
    path: &str,
) -> Result<DraftSession, String> {
    guard_switch(state).map_err(|c| c.to_string())?;
    let path = path.trim();
    if path.is_empty() {
        // 与 register_session_project 同一原则：宁可缺失，不可错记
        return Err("no_project_dir".to_string());
    }
    let draft = DraftSession {
        id: uuid::Uuid::new_v4().to_string(),
        mode: normalize_mode(mode).to_string(),
        project_path: path.to_string(),
        created_at: now_millis(),
    };
    set_draft_session(state, draft.clone());
    Ok(draft)
}

/// 「新建项目文件夹」第 ③ 步：在**当前项目目录**下立刻生成一条草稿对话并成为当前对话。
///
/// 调用方（创建项目弹窗）已按序完成 ① 写书签（`set_project_bookmarks`）② 切当前目录
/// （`set_project_dir`），本命令只做「归属快照 + 草稿登记」：**不落库**（无 sessions 行 /
/// session_meta / mirror / snapshot），因此用户未发消息就切换会话会消失、退出进程也不会
/// 留下任何痕迹；发出首条消息时真实会话在诞生点登记归属，标题走既有派生规则。
///
/// 归属路径取 `utils::active_project()`（与真实会话诞生点同源）——未配置项目目录时
/// 返回稳定错误码 `no_project_dir`，**不猜测路径**。失败码：busy / append_pending /
/// no_project_dir。
#[tauri::command]
pub fn create_project_chat(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let Some((_tag, dir)) = nuphus::utils::active_project() else {
        return Err("no_project_dir".to_string());
    };
    let draft = create_draft_session(state.inner(), &current_mode, &dir)?;
    tracing::info!(
        "[Shelf] 新建项目文件夹：草稿对话 {} @ {}（内存态，不落库）",
        draft.id,
        draft.project_path
    );
    // 展示台列表变化 → 双端同步（手机刷新会话清单；当前会话未变，不重拉历史）
    crate::emitter::CompoundEmitter::new(app, state.inner())
        .emit(nuphus::agent::events::NuphusEvent::ShelfUpdated);
    Ok(serde_json::json!({
        "id": draft.id,
        "mode": draft.mode,
        "project_path": draft.project_path,
    }))
}

/// 标题规范化：空白 → None（视为未填：不记录、不报错，会话走既有派生标题语义）；
/// 超长 → 稳定错误码（与 rename_session_cmd 同一 60 字上限；弹窗内已按 40 字截断，
/// 正常路径到不了这里）。
fn normalize_new_chat_title(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let title = raw.trim().to_string();
    if title.is_empty() {
        return Ok(None);
    }
    if title.chars().count() > 60 {
        return Err("invalid_title".to_string());
    }
    Ok(Some(title))
}

/// 把「新建对话」弹窗确认时记录的标题落到刚诞生的会话上。
///
/// **只在此处消费，且只消费一次**——记录取走即清空，不会泄漏给之后的会话。
/// 记录为空（Ctrl+N / TitleBar / 手机遥控 / 未填标题）时不做任何写。
fn apply_recorded_title(state: &AppState, session: &Session) {
    let title = match state
        .session
        .lock()
        .ok()
        .and_then(|mut sb| sb.pending_new_chat_title.take())
    {
        Some(t) => t,
        None => return,
    };
    // ① 展示台覆盖表：rail 显示、归档入台、完成回填都以它为准（与 rename_session_cmd
    //    同一优先级语义）——只写 sessions.summary 不够，派生标题仍会在 rail 上盖过它
    // ② 元数据行：记忆页 / 重启后按快照回读都取 sessions.summary（warm_from_disk 还会
    //    用它回填覆盖表，重启后自定义标题不丢）
    if let Ok(mut shelf) = state.shelf.lock() {
        shelf.titles.insert(session.id.clone(), title.clone());
    }
    upsert_meta_row(session, &title);
    tracing::info!("[Shelf] 会话诞生点应用记录标题 {}: {title}", session.id);
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
    // 超容量 = **主动归档**被淘汰者（元数据行与文本记忆保留，快照显式清空）。
    // 出 shelf 锁后再落库：锁序规定 db 锁在 shelf 锁之外，避免与 write_mirror /
    // upsert_meta_row 的加锁顺序交叉。
    let evicted = match state.shelf.lock() {
        Ok(mut shelf) => shelf.put(entry, snapshot),
        Err(_) => None,
    };
    if let Some(id) = evicted {
        // 旧实现只打一行日志，并声称「SQLite 快照永久保留」——与事实相反：下一次
        // persist 的白名单裁剪会把它置 NULL，用户侧表现为「会话凭空消失」且无据可查。
        // 现在改为显式归档：立刻清该会话快照，谁被归档、何时归档在日志与记忆页都可追溯。
        delete_mirror(&id);
        tracing::info!("[Shelf] 超容量主动归档会话 {id}：快照已清空，元数据行与文本记忆保留");
    }
}

/// 列出展示台：按 created_at 降序稳定排序（最新创建在上，切换/激活不改变位置，
/// 只通过 is_active 变化颜色/效果）。附 stage（执行态权威）与 can_switch（切换守卫）。
#[tauri::command]
pub fn list_shelf_sessions(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    list_shelf_sessions_inner(&state)
}

/// 内部实现（&AppState 直取）：mobile_server 的会话清单镜像端点复用
pub(crate) fn list_shelf_sessions_inner(state: &AppState) -> Result<serde_json::Value, String> {
    let can_switch = guard_switch(state).is_ok();
    // 执行态（唯一真相源）：桌面 rail 与手机 NavBar 的「执行中锁定」读这一个字段，
    // can_switch 只表达「切换动作是否会被守卫拒绝」，两者不再 OR 派生（见 ExecutionStage）。
    let stage = state.busy.stage();
    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    // 草稿对话（新建项目文件夹 → 尚未开说的那一条）：先于 runtime 长锁读数（避免嵌套加锁）。
    // 是否作为 active 展示由下方「槽里没有别的 active 会话」决定——当前对话只能有一个。
    let draft = draft_session(state);

    // 收集候选 (id, item_json, is_active)；active 与会话台条目统一参与稳定排序
    let mut candidates: Vec<(String, serde_json::Value, bool)> = Vec::new();
    let mut active_id: Option<String> = None;
    // 被展示的草稿对话 `(id, 归属路径)`：归属快照的唯一出口（无草稿/未展示时为 None）
    let mut draft_origin: Option<(String, String)> = None;
    // 创建时间的兜底表（仅「尚无 SQLite 行」的会话）：id → Unix 毫秒
    let mut created_fallback: HashMap<String, u64> = HashMap::new();

    // active（runtime）：backup 中转残留路径下同一 id 可能同时在 runtime 与 shelf，
    // 以 active 为准展示，shelf 循环跳过同 id 去重。
    //
    // 空 messages 的 active 会话也作为 active 返回：0825-02 修复后 SessionRail 5s 轮询
    // 比对 active id 变化以感知外部会话切换，若 active 在「empty → non-empty」
    // 之间跳变，会被误判为外部变更触发无意义重拉。保持 active id 从创建那一刻起
    // 稳定，让 SessionRail 只在真正切换时刷新。
    let runtime_active = state
        .runtime
        .lock()
        .ok()
        .and_then(|ctx| active_session(&ctx, kind).cloned());
    if let Some(sess) = runtime_active.as_ref() {
        push_active_candidate(
            state,
            kind,
            sess,
            &mut candidates,
            &mut active_id,
            &mut created_fallback,
        );
    } else if let Some(d) = draft.filter(|d| d.mode == kind) {
        // 草稿对话：槽里没有别的 active 会话，且 mode 匹配（当前 mode 的当前对话）。
        // 优先于 session_backup 回退——新建项目产生的草稿才是最新的「当前对话」，
        // 此时 backup 里仍是切换前那一条。
        // 归属路径用**创建时快照**回填（下方经 project_paths 覆盖表统一写入），
        // 不读库、不按当前目录推断——它尚未落任何持久化。
        active_id = Some(d.id.clone());
        draft_origin = Some((d.id.clone(), d.project_path.clone()));
        created_fallback.insert(d.id.clone(), d.created_at);
        candidates.push((
            d.id.clone(),
            serde_json::json!({
                "id": d.id, "mode": d.mode, "title": "",
                "preview": "",
                "message_count": 0, "updated_at": d.created_at,
                "is_active": true,
                // 前端据 draft 渲染「新建对话」并隐藏重命名/归档（改名会写 sessions 行）
                "draft": true,
            }),
            true,
        ));
    } else if let Some(sess) = read_backup_session(state) {
        // chat_history 在当前 mode 的 runtime 槽为空时也使用 session_backup；
        // 列表必须复用同一回退源，否则 Workflow 会话切换成功后看不到“当前”。
        push_active_candidate(
            state,
            kind,
            &sess,
            &mut candidates,
            &mut active_id,
            &mut created_fallback,
        );
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

    // ── 分组数据（Phase 1）：条目补归属路径；projects[] / archived_projects[] /
    // collapsed_limit 供前端直接建组，无需二次拼装。
    // 无归属会话（历史遗留、未配置项目目录时创建）project_path = null → 前端归入
    // 「未分组」；后端不做任何按当前目录的推断。
    let prefs = nuphus::config::UserPreferences::load();
    let ids: Vec<String> = candidates.iter().map(|c| c.0.clone()).collect();
    let mut project_paths = nuphus::store::session::session_project_paths(&ids).unwrap_or_default();
    // 草稿对话的归属来自**内存快照**（它没有、也不会有 session_meta 行）：覆盖表是唯一
    // 回填点，item.project_path 与分组用的 session_paths 因此天然一致。
    if let Some((draft_id, draft_path)) = &draft_origin {
        project_paths.insert(draft_id.clone(), draft_path.clone());
    }
    let session_paths: Vec<String> = candidates
        .iter()
        .filter_map(|c| project_paths.get(&c.0).cloned())
        .collect();
    let (projects, archived_projects) =
        build_project_groups(&prefs.project_bookmarks, &prefs.project_dir, &session_paths);
    let collapsed_limit = prefs.session_group_limit();

    Ok(serde_json::json!({
        "can_switch": can_switch,
        "stage": stage.as_str(),
        "items": candidates
            .into_iter()
            .map(|(id, mut v, _)| {
                if let Some(obj) = v.as_object_mut() {
                    let path = match project_paths.get(&id) {
                        Some(p) => serde_json::Value::String(p.clone()),
                        None => serde_json::Value::Null,
                    };
                    obj.insert("project_path".to_string(), path);
                    // created_at（Unix 毫秒）：会话创建时刻，供「按时间顺序 → 创建时间」组内排序。
                    // 来源优先级：① SQLite sessions.created_at（rfc3339 → ms，upsert_meta_row /
                    // upsert_snapshot 维护，覆盖全部已落盘会话）；② 兜底表（尚无行的 active
                    // 会话：首条消息时间戳，空会话退化为 updated_at）；③ 最后退化为 updated_at。
                    // 三级都是会话自身的真实时间，不造值。
                    let updated = obj.get("updated_at").and_then(|x| x.as_u64()).unwrap_or(0);
                    let created = created_at_map
                        .get(&id)
                        .and_then(|s| rfc3339_to_millis(s))
                        .or_else(|| created_fallback.get(&id).copied())
                        .unwrap_or(updated);
                    obj.insert("created_at".to_string(), serde_json::json!(created));
                }
                v
            })
            .collect::<Vec<_>>(),
        "projects": projects,
        "archived_projects": archived_projects,
        "collapsed_limit": collapsed_limit,
        // 排序偏好（组序维度 + 组内排序键）：桌面 SessionRail 与移动端 NavBar 共用读数，
        // 已归一（非法配置值不会漏到前端）。
        "sort_prefs": {
            "group_order": prefs.session_group_order(),
            "sort_key": prefs.session_sort_key(),
        },
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
        // 切走了：草稿对话（若有）不再是当前对话
        clear_draft_session(state);
        broadcast_session_changed_mobile(state, &sid);
        return Ok(());
    };
    *slot = target_session;

    tracing::info!(
        "[Shelf] 切换到会话 {} ({current_kind} -> {target_kind})",
        entry.id
    );
    // 切走了：草稿对话（若有）不再是当前对话
    clear_draft_session(state);
    broadcast_session_changed_mobile(state, &entry.id);
    Ok(())
}

/// 新建对话 = 后端真转场（单一权威状态）：归档当前（有内容才占槽）→ 当前 mode 槽置
/// None（**不创建任何空会话**——「新建对话」只回到无会话的欢迎页，新会话仅在欢迎页
/// 直发消息时由 process.rs 空态判据创建）→ 记录弹窗标题（`title`，诞生点消费）→
/// 清 session_backup/去重键/重试现场。
///
/// `title` = 弹窗里填的会话标题：**确认只记录，不创建会话**——记录进
/// `SessionState.pending_new_chat_title`，由 [`register_session_birth`] 在会话诞生点
/// 取出写成该会话的标题（展示台覆盖表 + sessions.summary）。None（Ctrl+N / TitleBar /
/// 手机遥控）同时承担「清掉上一次残留记录」的语义。
///
/// 广播：CompoundEmitter 双推（桌面 Tauri IPC + 手机 WS）——手机「新建对话」遥控桌面
/// 走同一入口，变更经 SessionChanged 事件回传，双端跟随显示（单一路径，手机跟随）。
#[tauri::command]
pub fn new_chat_session_cmd(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<String, String> {
    new_chat_session_with_event(&app, state.inner(), title)
}

/// 内部实现（&AppState 直取）：mobile_server 的 /new-chat 端点复用（避免构造 tauri State）
pub(crate) fn new_chat_session_with_event<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    title: Option<String>,
) -> Result<String, String> {
    guard_switch(state).map_err(|c| c.to_string())?;
    // 标题校验先于任何状态变更：非法标题直接拒绝，不留「已归档但没记录标题」的半截状态
    let title = normalize_new_chat_title(title)?;

    let current_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let kind = normalize_mode(&current_mode);

    let mut ctx = state.runtime.lock().map_err(|e| e.to_string())?;
    archive_active(state, &mut ctx, kind);

    // 离开 active：当前 mode 槽整体置 None（无 agent = 欢迎页/无会话）。不装空会话。
    // 新会话只在下一次欢迎页直发消息时由 process.rs 空态判据创建。
    let new_id = uuid::Uuid::new_v4().to_string(); // SessionChanged 事件 token，非真实会话 id
    match kind {
        "workflow" => ctx.workflow_agent = None,
        _ => ctx.leader_agent = None,
    }
    drop(ctx);

    // 会话边界必须同步清理恢复快照/去重键/重试现场。否则：新会话第一条消息若恰好
    // 与上一会话末条相同，会被 completion dedup 当成重复提交而静默丢弃；或旧失败回合
    // 仍可被「重试」复活进新会话。
    if let Ok(mut sb) = state.session.lock() {
        sb.session_backup = None;
        sb.last_message.clear();
        sb.last_send_id = None;
        sb.last_message_images.clear();
        // 记录本次弹窗标题（None = 清空旧记录）：与清 backup 同一把锁同一次边界动作，
        // 避免两次加锁之间被其它会话边界动作插进来
        sb.pending_new_chat_title = title;
        // 回到欢迎页 = 当前对话不再存在：草稿对话（若有）一并清掉，否则它会被当成
        // 「当前对话」继续挂在 rail 上，与用户刚刚表达的「新建」意图冲突
        sb.draft_session = None;
    }
    if let Ok(mut ex) = state.execution.lock() {
        ex.pending_retry = None;
    }
    tracing::info!(
        "[Shelf] 新建对话：归档并清空当前 {kind} 槽（回到欢迎页，标题已记录于诞生点消费）"
    );
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
        // 当前对话变成刚恢复的这条：草稿对话（若有）不再是当前对话
        sb.draft_session = None;
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
        fn provider_name(&self) -> &str {
            ""
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

    // ── 项目文件夹分组 ──

    fn project_bookmark(name: &str, path: &str, archived: bool) -> nuphus::config::ProjectBookmark {
        nuphus::config::ProjectBookmark {
            name: name.to_string(),
            path: path.to_string(),
            archived,
        }
    }

    /// projects[] 组装：书签保序 + 当前目录标记 + 未收藏自动组排书签之后 + 归档过滤。
    #[test]
    fn build_project_groups_orders_bookmarks_then_autos() {
        let bookmarks = vec![
            project_bookmark("A", "E:\\work\\A", false),
            project_bookmark("已归档目录", "E:\\work\\Old", true),
        ];
        let session_paths = vec![
            "E:\\work\\B".to_string(),
            "E:\\work\\A".to_string(),
            "E:\\work\\B".to_string(),
        ];

        let (visible, archived) = build_project_groups(&bookmarks, "E:\\work\\B", &session_paths);

        let paths: Vec<&str> = visible.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["E:\\work\\A", "E:\\work\\B"],
            "书签在前，自动组在后"
        );
        assert_eq!(visible[0].name, "A", "书签自定义名优先");
        assert!(!visible[0].auto && !visible[0].is_current);
        assert!(visible[1].auto, "未收藏但有会话的目录 → 只读自动组");
        assert!(visible[1].is_current, "当前工作目录标记（仅高亮）");
        assert_eq!(visible[1].name, "B", "自动组展示名取目录末段");

        // 归档书签：可见组里被过滤（隐藏），单独返回供「已归档文件夹」恢复入口
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].path, "E:\\work\\Old");
        assert!(archived.iter().all(|e| !e.auto));
        assert!(visible.iter().all(|e| e.path != "E:\\work\\Old"));
    }

    /// 同目录多会话 → 只生成一个组；归属路径与书签重合 → 由书签组代表（不重复）。
    /// 尾斜杠差异与平台无关（归一化时去掉尾部分隔符），故三平台共用同一断言。
    #[test]
    fn build_project_groups_dedups_paths_and_prefers_bookmarks() {
        let bookmarks = vec![project_bookmark("A", "E:\\work\\A\\", false)];
        let session_paths = vec!["E:\\work\\A".to_string(), "E:\\work\\A".to_string()];

        let (visible, _) = build_project_groups(&bookmarks, "", &session_paths);
        assert_eq!(visible.len(), 1, "尾斜杠/重复路径不得裂成两个组");
        assert_eq!(visible[0].path, "E:\\work\\A", "以书签为准则保序保名");
        assert!(!visible[0].auto);
    }

    /// 大小写差异是否算同一目录，由 `same_project_path` 里的 `cfg!(windows)` 决定：
    /// Windows 忽略大小写，非 Windows 大小写敏感（`/work/A` 与 `/work/a` 是两个目录）。
    /// 因此该断言只在 Windows 成立；非 Windows 的分支由
    /// `build_project_groups_treats_case_as_distinct_on_unix` 覆盖。
    #[cfg(windows)]
    #[test]
    fn build_project_groups_ignores_case_on_windows() {
        let bookmarks = vec![project_bookmark("A", "E:\\work\\A\\", false)];
        let session_paths = vec!["E:\\work\\a".to_string(), "e:\\work\\a".to_string()];

        let (visible, _) = build_project_groups(&bookmarks, "", &session_paths);
        assert_eq!(
            visible.len(),
            1,
            "Windows 下尾斜杠/大小写差异不得裂成两个组"
        );
        assert_eq!(visible[0].path, "E:\\work\\A", "以书签为准则保序保名");
        assert!(!visible[0].auto);
    }

    /// 非 Windows（Linux/macOS）大小写敏感：`E:\work\A` 与 `E:\work\a` 是两个目录，
    /// 归属路径不得被书签吞并，两条大小写不同的会话路径也各自成组 → 共 3 组。
    #[cfg(not(windows))]
    #[test]
    fn build_project_groups_treats_case_as_distinct_on_unix() {
        let bookmarks = vec![project_bookmark("A", "E:\\work\\A\\", false)];
        let session_paths = vec!["E:\\work\\a".to_string(), "e:\\work\\a".to_string()];

        let (visible, _) = build_project_groups(&bookmarks, "", &session_paths);
        assert_eq!(
            visible.len(),
            3,
            "非 Windows 大小写敏感：大小写不同的目录不得合并成一组"
        );
    }

    /// 无归属会话（未出现在 session_paths）不产生任何组：不猜测、不伪造。
    #[test]
    fn build_project_groups_skips_ungrouped_sessions() {
        let (visible, archived) = build_project_groups(&[], "", &[]);
        assert!(visible.is_empty(), "无归属会话不得凭空生成组");
        assert!(archived.is_empty());
    }

    // ── 历史会话归属回填（启动一次性迁移）──
    //
    // 被测函数在 `nuphus::store::session`（lib 侧，session_meta 的唯一归属写入层）：
    // 这里用内存连接驱动它，覆盖「tag 精确匹配 → 回填 / 不匹配 → 保持无归属 / 幂等 /
    // 不覆盖已有路径 / 异常 tag 不 panic」，并附真实库副本的手动探针。

    /// 与生产表结构一致的内存 session_meta（db.rs DDL）——回填不必碰真实 DB。
    fn backfill_conn() -> rusqlite::Connection {
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

    fn insert_meta(conn: &rusqlite::Connection, id: &str, tag: &str, path: Option<&str>) {
        conn.execute(
            "INSERT INTO session_meta (session_id, project_tag, project_path, created_at)
             VALUES (?1, ?2, ?3, '2026-01-01T00:00:00Z')",
            rusqlite::params![id, tag, path],
        )
        .unwrap();
    }

    fn stored_path(conn: &rusqlite::Connection, id: &str) -> Option<String> {
        conn.query_row(
            "SELECT project_path FROM session_meta WHERE session_id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn missing_path_count(conn: &rusqlite::Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM session_meta WHERE project_path IS NULL OR project_path = ''",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// ① tag 命中候选目录 → 回填成功；同 tag 多行共用一次解析结果（自愈复用）。
    ///
    /// 两个候选目录的 tag **都用 `derive_project_tag_from_dir` 现算**，不硬编码 Windows 上
    /// 派生的字面值：该函数 name 段取 `Path::file_name()`，而 `E:\NUS\1` 这类字符串在非
    /// Windows 上「整串即文件名」（分隔符不是分隔符），硬编码值会让本用例在 ubuntu 上失配。
    /// 生产逻辑本身三平台一致（写入的是命中候选目录的原串，见 `dir_for_project_tag`），
    /// Windows 上派生值的等价关系另由 `derive_tag_matches_legacy_windows_value` 守住。
    #[test]
    fn backfill_writes_path_when_tag_matches_candidate_dir() {
        let conn = backfill_conn();
        let dir = "E:\\NUS\\Nuphus";
        let other_dir = "E:\\NUS\\1";
        let tag = nuphus::utils::derive_project_tag_from_dir(dir).unwrap();
        let other_tag = nuphus::utils::derive_project_tag_from_dir(other_dir).unwrap();
        assert_ne!(
            tag, other_tag,
            "两个不同目录的 tag 必须可区分，否则本用例退化"
        );
        insert_meta(&conn, "s1", &tag, None);
        insert_meta(&conn, "s2", &tag, None);
        insert_meta(&conn, "s3", &other_tag, None);

        let candidates = vec![
            other_dir.to_string(), // 书签顺序无关：按 tag 精确匹配
            dir.to_string(),
        ];
        let n =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();

        assert_eq!(n, 3, "三个同/异 tag 行都应命中候选目录并回填");
        assert_eq!(stored_path(&conn, "s1").as_deref(), Some(dir));
        assert_eq!(stored_path(&conn, "s2").as_deref(), Some(dir));
        assert_eq!(
            stored_path(&conn, "s3").as_deref(),
            Some(other_dir),
            "命中书签目录"
        );
        assert_eq!(missing_path_count(&conn), 0);
    }

    /// 真实库里的历史 tag 是 **Windows 上**派生的（`1-228c2201` ⇒ `E:\NUS\1`）：
    /// 该等价关系依赖 Windows 的路径语义，只在 Windows 成立，故单独 cfg 断言——
    /// 它是「回填能把真实库的旧 tag 还原成目录」这一事实的直接证据。
    #[cfg(windows)]
    #[test]
    fn derive_tag_matches_legacy_windows_value() {
        assert_eq!(
            nuphus::utils::derive_project_tag_from_dir("E:\\NUS\\1").as_deref(),
            Some("1-228c2201"),
            "真实库 tag 1-228c2201 ⇒ E:\\NUS\\1（Windows 派生）"
        );
        assert_eq!(
            nuphus::utils::derive_project_tag_from_dir("E:\\NUS\\Nuphus").as_deref(),
            Some("Nuphus-9102132f"),
            "真实库 tag Nuphus-9102132f ⇒ E:\\NUS\\Nuphus（Windows 派生）"
        );
    }

    /// ② tag 无匹配 → 不写：保持无归属，前端仍归「未分组」（不猜测、不伪造）。
    #[test]
    fn backfill_leaves_unmatched_tag_without_path() {
        let conn = backfill_conn();
        insert_meta(&conn, "s1", "已删除的项目-deadbeef", None);
        insert_meta(&conn, "s2", "Nuphus-00000000", None); // 名字对但哈希不匹配

        let candidates = vec!["E:\\NUS\\Nuphus".to_string()];
        let n =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();

        assert_eq!(n, 0, "无候选命中不得写入任何路径");
        assert_eq!(stored_path(&conn, "s1"), None);
        assert_eq!(stored_path(&conn, "s2"), None);
        assert_eq!(
            missing_path_count(&conn),
            2,
            "无归属行必须留在待填清单（未分组兜底）"
        );
    }

    /// ③ 重复运行幂等：第二次零变更，且已填值稳定不变。
    #[test]
    fn backfill_is_idempotent() {
        let conn = backfill_conn();
        let dir = "E:\\NUS\\1";
        let tag = nuphus::utils::derive_project_tag_from_dir(dir).unwrap();
        insert_meta(&conn, "s1", &tag, None);
        insert_meta(&conn, "s2", "未知-0badf00d", None);
        let candidates = vec![dir.to_string()];

        let first =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();
        let second =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();

        assert_eq!(first, 1, "首次只回填命中行");
        assert_eq!(second, 0, "第二次不得产生任何变更");
        assert_eq!(stored_path(&conn, "s1").as_deref(), Some(dir));
        assert_eq!(missing_path_count(&conn), 1);
    }

    /// ④ 已有 project_path 不被覆盖：即使该行 tag 指向另一个候选目录。
    #[test]
    fn backfill_never_overwrites_existing_path() {
        let conn = backfill_conn();
        let old_dir = "E:\\work\\Old";
        let new_dir = "E:\\work\\New";
        // tag 由 new_dir 派生（人为构造「路径与 tag 不同源」的行），回填不得改写它
        let tag = nuphus::utils::derive_project_tag_from_dir(new_dir).unwrap();
        insert_meta(&conn, "s1", &tag, Some(old_dir));
        insert_meta(&conn, "s2", &tag, Some("")); // 空串属于无归属：允许回填

        let candidates = vec![new_dir.to_string()];
        let n =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();

        assert_eq!(n, 1, "只有空值行可写");
        assert_eq!(
            stored_path(&conn, "s1").as_deref(),
            Some(old_dir),
            "既有归属不得被改写"
        );
        assert_eq!(
            stored_path(&conn, "s2").as_deref(),
            Some(new_dir),
            "空串视为无归属，可回填"
        );
    }

    /// ⑤ 空 / 异常 tag 不 panic、不写入（标签不可逆 → 只认精确匹配，不猜测）。
    #[test]
    fn backfill_tolerates_empty_and_odd_tags() {
        let conn = backfill_conn();
        let long_tag = format!("{}-{:08x}", "超长项目名".repeat(10), 1u32);
        for (i, tag) in [
            "",
            "   ",
            "default",
            "Nuphus",                // 缺哈希段
            "Nuphus-12345678-extra", // 多段
            "🚀项目-1234abcd",       // 非 ASCII
            long_tag.as_str(),
        ]
        .iter()
        .enumerate()
        {
            insert_meta(&conn, &format!("s{i}"), tag, None);
        }
        let candidates = vec!["E:\\NUS\\Nuphus".to_string(), "".to_string()];

        let n =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();

        assert_eq!(n, 0, "异常 tag 一律不匹配，不得写入");
        assert_eq!(missing_path_count(&conn), 7);

        // 候选目录为空 → 直接短路，不查询、不 panic
        let conn2 = backfill_conn();
        insert_meta(&conn2, "s1", "Nuphus-9102132f", None);
        assert_eq!(
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn2, &[]).unwrap(),
            0
        );
        assert_eq!(
            nuphus::store::session::sessions_missing_project_path(&conn2)
                .unwrap()
                .len(),
            1
        );
    }

    /// 回填 → rail 分组闭环：候选目录取自书签 + 当前目录，回填后的归属路径直接喂给
    /// 分组逻辑（`build_project_groups`），组必须按书签顺序出现、无归属行不产生组。
    #[test]
    fn backfill_output_feeds_rail_grouping() {
        let conn = backfill_conn();
        let dir_a = "E:\\NUS\\Nuphus";
        let dir_b = "E:\\NUS\\1";
        let tag_a = nuphus::utils::derive_project_tag_from_dir(dir_a).unwrap();
        let tag_b = nuphus::utils::derive_project_tag_from_dir(dir_b).unwrap();
        for i in 0..3 {
            insert_meta(&conn, &format!("a{i}"), &tag_a, None);
        }
        for i in 0..2 {
            insert_meta(&conn, &format!("b{i}"), &tag_b, None);
        }
        insert_meta(&conn, "u0", "已删除的项目-deadbeef", None);

        let bookmarks = vec![
            project_bookmark("Nuphus", dir_a, false),
            project_bookmark("1", dir_b, false),
        ];
        let candidates =
            nuphus::store::session::backfill_candidate_dirs(&conn, &bookmarks, dir_a).unwrap();
        assert_eq!(
            candidates,
            vec![dir_a.to_string(), dir_b.to_string()],
            "候选 = 书签序 + 当前目录（去重）"
        );

        let n =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();
        assert_eq!(n, 5, "命中的 5 行回填，无归属行保持 NULL");

        // rail 侧：条目 project_path → 会话数（前端按此建组）
        let mut stmt = conn
            .prepare(
                "SELECT project_path, COUNT(*) FROM session_meta
                 WHERE project_path IS NOT NULL AND project_path != '' GROUP BY project_path",
            )
            .unwrap();
        let counts: HashMap<String, i64> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let mut paths: Vec<String> = counts.keys().cloned().collect();
        paths.sort();

        let (visible, archived) = build_project_groups(&bookmarks, dir_a, &paths);
        assert!(archived.is_empty());
        assert_eq!(
            visible.len(),
            2,
            "回填后两个书签目录各成一组，无归属行不产生组"
        );
        assert_eq!(
            (
                visible[0].path.as_str(),
                visible[0].is_current,
                visible[0].auto
            ),
            (dir_a, true, false)
        );
        assert_eq!(visible[1].path, dir_b);
        assert_eq!(
            counts.get(dir_a).copied(),
            Some(3),
            "E:\\NUS\\Nuphus 组 3 条"
        );
        assert_eq!(counts.get(dir_b).copied(), Some(2), "E:\\NUS\\1 组 2 条");
    }

    /// 真实库副本探针（手动，默认忽略）——把 `%APPDATA%\\nuphus\\nuphus.db`（含
    /// `-wal`/`-shm`）**复制到临时目录**后，用副本路径运行：
    ///   `cargo test -p nuphus-desktop --bin nuphus backfill_probe -- --ignored --nocapture`
    /// 候选目录取自真实 preferences（只读）。真实库路径被显式拒绝：探针只写副本。
    #[test]
    #[ignore]
    fn backfill_probe_on_real_db_copy() {
        let Some(copy) = std::env::var("NUPHUS_BACKFILL_PROBE_DB")
            .ok()
            .filter(|p| !p.trim().is_empty())
        else {
            println!("[probe] 未设置 NUPHUS_BACKFILL_PROBE_DB（数据库副本路径），跳过");
            return;
        };
        if let (Ok(a), Some(Ok(b))) = (
            std::fs::canonicalize(&copy),
            dirs::data_dir().map(|d| std::fs::canonicalize(d.join("nuphus").join("nuphus.db"))),
        ) {
            assert_ne!(a, b, "拒绝对真实库运行：请先复制副本再传副本路径");
        }

        let conn = rusqlite::Connection::open(&copy).expect("打开数据库副本失败");
        let prefs = nuphus::config::UserPreferences::load();
        let candidates = nuphus::store::session::backfill_candidate_dirs(
            &conn,
            &prefs.project_bookmarks,
            &prefs.project_dir,
        )
        .unwrap();
        println!("[probe] 候选目录 = {candidates:?}");
        println!("[probe] 回填前 NULL 行数 = {}", missing_path_count(&conn));
        let n =
            nuphus::store::session::backfill_session_project_paths_with_conn(&conn, &candidates)
                .unwrap();
        println!("[probe] 本次回填 {n} 行");
        println!("[probe] 回填后 NULL 行数 = {}", missing_path_count(&conn));
        let mut stmt = conn
            .prepare(
                "SELECT project_tag, project_path, COUNT(*) FROM session_meta
                 GROUP BY project_tag, project_path ORDER BY project_tag",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (tag, path, count) = row.unwrap();
            println!("[probe]   {tag} → {path:?}（{count} 行）");
        }
    }

    /// 会话台返回体形状：条目带 project_path（无归属 = null），顶层带
    /// projects / archived_projects / collapsed_limit，前端可直接分组。
    #[test]
    fn list_shelf_sessions_carries_grouping_payload() {
        let state = AppState::default();
        let sess = session_with_user(&["历史会话（无归属）"]);
        let id = sess.id.clone();
        {
            let mut shelf = state.shelf.lock().unwrap();
            let entry = build_entry(id.clone(), "leader", &sess, Some("历史会话（无归属）"));
            shelf.put(entry, sess);
        }

        let payload = list_shelf_sessions_inner(&state).unwrap();
        let items = payload["items"].as_array().expect("items 必须是数组");
        let item = items
            .iter()
            .find(|i| i["id"] == serde_json::json!(id))
            .expect("返回体应含驻留会话");
        assert!(
            item.get("project_path").is_some(),
            "条目必须带 project_path 键（无归属为 null）"
        );
        assert_eq!(
            item["project_path"],
            serde_json::Value::Null,
            "无归属会话不得用当前目录回填"
        );
        assert!(payload["projects"].is_array(), "projects 必须是数组");
        assert!(
            payload["archived_projects"].is_array(),
            "archived_projects 必须是数组"
        );
        let limit = payload["collapsed_limit"]
            .as_u64()
            .expect("collapsed_limit 必须是数字");
        assert!(limit >= 1, "折叠上限必须为正数");
        assert!(
            payload["can_switch"].as_bool().is_some(),
            "既有字段不得丢失"
        );
        // 排序偏好 + 创建时间（排序 UI 的两个新数据源）
        let created = item["created_at"]
            .as_u64()
            .expect("条目必须带 created_at（Unix 毫秒）");
        assert!(created > 0, "created_at 必须是真实时间戳");
        assert_eq!(
            created,
            item["updated_at"].as_u64().unwrap(),
            "无 SQLite 行时 created_at 退化为 updated_at（本用例只 put 内存、未落元数据行）"
        );
        let group_order = payload["sort_prefs"]["group_order"]
            .as_str()
            .expect("sort_prefs.group_order 必须是字符串");
        let sort_key = payload["sort_prefs"]["sort_key"]
            .as_str()
            .expect("sort_prefs.sort_key 必须是字符串");
        assert!(
            ["bookmark", "recent"].contains(&group_order),
            "组序维度必须是归一后的合法值，实际: {group_order}"
        );
        assert!(
            ["updated", "created"].contains(&sort_key),
            "组内键必须是归一后的合法值，实际: {sort_key}"
        );
    }

    /// created_at 数据来源：已落盘会话取 SQLite `sessions.created_at`（rfc3339 → ms），
    /// 而不是列表排序用的 updated_at ——「按时间顺序 → 创建时间」组内序唯一权威。
    #[test]
    fn list_shelf_sessions_created_at_comes_from_sessions_row() {
        let state = AppState::default();
        let sess = session_with_user(&["创建时间来源校验"]);
        let id = sess.id.clone();
        // ShelfState::put 只动内存（与生产一致：元数据行由调用方落盘，见 archive_active）
        upsert_meta_row(&sess, "创建时间来源校验");
        {
            let mut shelf = state.shelf.lock().unwrap();
            shelf.put(
                build_entry(id.clone(), "leader", &sess, Some("创建时间来源校验")),
                sess,
            );
        }

        let payload = list_shelf_sessions_inner(&state).unwrap();
        let item = payload["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == serde_json::json!(id))
            .expect("返回体应含驻留会话")
            .clone();
        let created_ms = item["created_at"]
            .as_u64()
            .expect("created_at 必须是毫秒数");

        let row_created = nuphus::store::session::get_session(&id)
            .unwrap()
            .expect("put 必须落元数据行")
            .created_at;
        assert_eq!(
            created_ms,
            rfc3339_to_millis(&row_created).expect("sessions.created_at 应为 rfc3339"),
            "created_at 必须等于 sessions 行的 created_at（不造值）"
        );

        // 自清理（store 测试同款约定）
        let conn = nuphus::store::db::acquire().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
            .unwrap();
    }

    /// 已登记归属的会话：条目带真实路径，且该路径作为自动组（未收藏）出现在
    /// projects[] 末尾；is_current 与该路径是否为当前项目目录一致。
    /// 归属行写真实 DB 后自清理（store 测试同款约定）。
    #[test]
    fn list_shelf_sessions_groups_owned_session_into_auto_project() {
        let state = AppState::default();
        let sess = session_with_user(&["有归属会话"]);
        let id = sess.id.clone();
        {
            let mut shelf = state.shelf.lock().unwrap();
            shelf.put(
                build_entry(id.clone(), "leader", &sess, Some("有归属会话")),
                sess,
            );
        }
        let owned_dir = "E:\\__nuphus_test_proj__\\group";
        {
            let conn = nuphus::store::db::acquire().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO session_meta (session_id, project_tag, project_path, created_at)
                 VALUES (?1, 'group-00000000', ?2, 't')",
                rusqlite::params![id, owned_dir],
            )
            .unwrap();
        }

        let payload = list_shelf_sessions_inner(&state).unwrap();
        let item = payload["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == serde_json::json!(id))
            .expect("返回体应含驻留会话");
        assert_eq!(item["project_path"], serde_json::json!(owned_dir));

        let projects = payload["projects"].as_array().unwrap();
        let group = projects
            .iter()
            .find(|p| p["path"] == serde_json::json!(owned_dir))
            .expect("归属路径应生成自动组");
        assert_eq!(group["auto"], serde_json::json!(true));
        assert_eq!(group["name"], serde_json::json!("group"));
        let current = nuphus::config::UserPreferences::load().project_dir;
        assert_eq!(
            group["is_current"],
            serde_json::json!(same_project_path(owned_dir, &current)),
            "is_current 必须与当前项目目录一致"
        );

        let conn = nuphus::store::db::acquire().unwrap();
        conn.execute(
            "DELETE FROM session_meta WHERE session_id = ?1",
            rusqlite::params![id],
        )
        .unwrap();
    }

    /// 回归（任务链 87f4fc7a）：保护名单必须包含 shelf.order 全量驻留成员——
    /// prune 白名单漏掉任一在台成员都会导致其快照被误清、重启后从 rail 消失。
    #[test]
    fn protected_snapshot_ids_contains_all_shelf_order_entries() {
        let state = AppState::default();
        let want: Vec<String> = {
            let mut shelf = state.shelf.lock().unwrap();
            // 模拟「已从磁盘预热完成」：本用例校验的是可信内存态下的名单完整性
            shelf.warmed = true;
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

    /// 回归（2026-09-21 用户会话丢失）：展示台尚未预热时内存名单不可信 ——
    /// 保护名单必须为空，使上游 `prune_snapshots` 走「空名单短路」零裁剪。
    /// 这是「白名单非空但残缺」在生产路径上的第一道闸：预热未完成 → 名单为空。
    #[test]
    fn protected_snapshot_ids_is_empty_before_warmup() {
        let state = AppState::default();
        {
            let mut shelf = state.shelf.lock().unwrap();
            let s = session_with_user(&["未预热成员"]);
            shelf.put(
                build_entry(s.id.clone(), "leader", &s, Some("未预热成员")),
                s,
            );
        }
        assert!(
            !state.shelf.lock().unwrap().order.is_empty(),
            "前置：展示台应有成员"
        );
        assert!(
            protected_snapshot_ids(&state).is_empty(),
            "未预热时名单必须为空（触发零裁剪），禁止用残缺内存态反推删除快照"
        );

        // 预热完成后同一展示台应恢复产出名单
        state.shelf.lock().unwrap().warmed = true;
        assert!(
            !protected_snapshot_ids(&state).is_empty(),
            "预热完成后名单应正常产出"
        );
    }

    /// 容量语义：超上限淘汰最旧时，`entries` / `sessions` 不得留下幽灵成员
    /// （旧实现只 pop `order`，另两张表仍持有该 id → `len()` 与 `order` 口径漂移）。
    #[test]
    fn shelf_put_eviction_clears_entry_and_session_residue() {
        let mut shelf = ShelfState::default();
        let mut ids = Vec::new();
        for i in 0..SHELF_CAPACITY + 2 {
            let s = session_with_user(&[&format!("成员{i}")]);
            let id = s.id.clone();
            let evicted = shelf.put(build_entry(id.clone(), "leader", &s, None), s);
            if i < SHELF_CAPACITY {
                assert!(evicted.is_none(), "未超容量不应淘汰");
            } else {
                assert!(evicted.is_some(), "超容量应淘汰最旧");
            }
            ids.push(id);
        }
        assert_eq!(shelf.order.len(), SHELF_CAPACITY);
        assert_eq!(
            shelf.entries.len(),
            SHELF_CAPACITY,
            "entries 不应留幽灵成员"
        );
        assert_eq!(
            shelf.sessions.len(),
            SHELF_CAPACITY,
            "sessions 不应留幽灵成员"
        );
        for id in &ids[..2] {
            assert!(!shelf.contains(id), "被淘汰会话不应仍可查询: {id}");
        }
        for id in &ids[2..] {
            assert!(shelf.contains(id), "在台会话应可查询: {id}");
        }
    }

    /// 老镜像退场：改名后扩展名不再是 `.json` → 不再被迁移扫描命中（杜绝僵尸复活）。
    #[test]
    fn migrated_mirror_file_is_renamed_out_of_scan_scope() {
        let dir =
            std::env::temp_dir().join(format!("nuphus-mirror-retire-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("legacy-session.json");
        std::fs::write(&src, b"{}").unwrap();

        let retired = retire_migrated_file(&src).expect("退场改名应成功");
        assert!(!src.exists(), "原 .json 文件应已不在原位");
        assert!(retired.exists(), "退场文件应保留（可回滚）");
        assert_eq!(
            retired.extension().and_then(|x| x.to_str()),
            Some("migrated"),
            "退场文件扩展名应为 migrated（≠ json，扫描不再命中）"
        );
        // 幂等：源已不存在 → rename 失败返回 None，不 panic、不覆盖
        assert!(
            retire_migrated_file(&src).is_none(),
            "源已退场时应返回 None"
        );

        std::fs::remove_dir_all(&dir).ok();
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

    /// Workflow agent 尚未初始化时，切换会话会把目标放入 session_backup；列表的
    /// 当前标记必须与 chat_history 使用同一回退源，而不能因为 workflow_agent=None
    /// 把所有 Workflow 条目都显示为非当前。
    #[test]
    fn list_shelf_marks_workflow_backup_as_active_without_agent() {
        let state = AppState::default();
        {
            let mut cm = state.current_mode.write().unwrap();
            *cm = "workflow".to_string();
        }

        let target = session_with_user(&["备份中的工作流会话"]);
        let target_id = target.id.clone();
        let entry = build_entry(target_id.clone(), "workflow", &target, Some("工作流当前"));
        {
            let mut shelf = state.shelf.lock().unwrap();
            shelf.put(entry, target.clone());
        }
        {
            let mut session = state.session.lock().unwrap();
            session.session_backup = Some(serde_json::to_string(&target).unwrap());
        }

        let payload = list_shelf_sessions_inner(&state).unwrap();
        let items = payload["items"].as_array().unwrap();
        let active: Vec<&serde_json::Value> = items
            .iter()
            .filter(|item| item["is_active"] == true)
            .collect();
        assert_eq!(active.len(), 1, "backup 当前会话必须是唯一 active");
        assert_eq!(active[0]["id"].as_str(), Some(target_id.as_str()));
        assert_eq!(active[0]["mode"].as_str(), Some("workflow"));
    }

    /// runtime agent 已恢复时优先使用 runtime 会话，不能被旧 backup 覆盖。
    #[test]
    fn list_shelf_runtime_active_takes_precedence_over_backup() {
        let state = AppState::default();
        {
            let mut cm = state.current_mode.write().unwrap();
            *cm = "workflow".to_string();
        }

        let runtime_session = session_with_user(&["运行中的工作流会话"]);
        let backup_session = session_with_user(&["旧的备份会话"]);
        {
            let mut guard = state.runtime.lock().unwrap();
            guard.workflow_agent = Some(workflow_agent_with(runtime_session));
        }
        let runtime_id = {
            let guard = state.runtime.lock().unwrap();
            guard.workflow_agent.as_ref().unwrap().session().id.clone()
        };
        {
            let mut shelf = state.shelf.lock().unwrap();
            let entry = build_entry(
                backup_session.id.clone(),
                "workflow",
                &backup_session,
                Some("旧备份"),
            );
            shelf.put(entry, backup_session.clone());
        }
        {
            let mut session = state.session.lock().unwrap();
            session.session_backup = Some(serde_json::to_string(&backup_session).unwrap());
        }

        let payload = list_shelf_sessions_inner(&state).unwrap();
        let items = payload["items"].as_array().unwrap();
        let active: Vec<&serde_json::Value> = items
            .iter()
            .filter(|item| item["is_active"] == true)
            .collect();
        assert_eq!(active.len(), 1, "runtime 会话必须是唯一 active");
        assert_eq!(active[0]["id"].as_str(), Some(runtime_id.as_str()));
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

    // ── 「新建对话」弹窗：确认只记录标题，会话仍在发消息时诞生 ──

    /// 标题规范化：空白 → 未填（None，不报错）；超长按稳定错误码拒绝
    /// （与 rename_session_cmd 同一 60 字上限）。
    #[test]
    fn normalize_new_chat_title_trims_and_validates() {
        assert_eq!(normalize_new_chat_title(None).unwrap(), None);
        assert_eq!(
            normalize_new_chat_title(Some("   ".into())).unwrap(),
            None,
            "纯空白 = 未填：不记录，也不阻断新建"
        );
        assert_eq!(
            normalize_new_chat_title(Some("  接口联调复盘 ".into())).unwrap(),
            Some("接口联调复盘".to_string()),
            "首尾空白应裁掉（与 rename_session_cmd 同一规范化）"
        );
        assert_eq!(
            normalize_new_chat_title(Some("标".repeat(60))).unwrap(),
            Some("标".repeat(60)),
            "60 字为上限本身，应放行"
        );
        assert_eq!(
            normalize_new_chat_title(Some("标".repeat(61))).unwrap_err(),
            "invalid_title"
        );
    }

    /// 诞生点消费记录标题：写展示台覆盖表 + sessions.summary，且**只消费一次**——
    /// 下一个诞生（无记录）不得继承上一次弹窗的标题。
    #[test]
    fn recorded_new_chat_title_applies_once_at_birth() {
        let state = AppState::default();
        let first = Session::new(); // 诞生点状态：新 uuid + 空 session
        {
            let mut sb = state.session.lock().unwrap();
            sb.pending_new_chat_title = Some("接口联调复盘".to_string());
        }

        register_session_birth(&state, &first);

        assert_eq!(
            state.shelf.lock().unwrap().titles.get(&first.id).cloned(),
            Some("接口联调复盘".to_string()),
            "覆盖表必须记下弹窗标题（rail 显示 / 归档入台 / 完成回填都以它为准）"
        );
        let row = nuphus::store::session::get_session(&first.id)
            .unwrap()
            .expect("诞生点应落 sessions 行");
        assert_eq!(row.summary, "接口联调复盘", "sessions.summary = 弹窗标题");
        assert!(
            state
                .session
                .lock()
                .unwrap()
                .pending_new_chat_title
                .is_none(),
            "记录取走即清空（不会泄漏给之后的会话）"
        );

        // 第二个诞生（无记录）：不得继承上一次的标题
        let second = Session::new();
        register_session_birth(&state, &second);
        assert!(
            !state.shelf.lock().unwrap().titles.contains_key(&second.id),
            "无记录会话不得拿到上一次弹窗的标题"
        );
        assert!(
            nuphus::store::session::get_session(&second.id)
                .unwrap()
                .is_none(),
            "无记录时诞生点不写元数据行（保持既有派生标题语义）"
        );

        // 自清理（store 测试同款约定）：sessions 行 + 本测试经 register_session_origin
        // 顺带写下的 session_meta 行
        let conn = nuphus::store::db::acquire().unwrap();
        for id in [&first.id, &second.id] {
            let _ = nuphus::store::session::delete_session(id);
            conn.execute(
                "DELETE FROM session_meta WHERE session_id = ?1",
                rusqlite::params![id],
            )
            .unwrap();
        }
    }

    /// 记录标题一旦落库，后续「空标题」元数据回填（每轮完成/归档都写一次空串）
    /// 必须保住它——这是「标题打回派生默认」那类回归的守卫。
    #[test]
    fn recorded_title_survives_empty_title_meta_refresh() {
        let state = AppState::default();
        let born = Session::new();
        {
            let mut sb = state.session.lock().unwrap();
            sb.pending_new_chat_title = Some("接口联调复盘".to_string());
        }
        register_session_birth(&state, &born);

        // 首条消息进入后（会话不再为空）按既有回填语义写一次空标题
        let mut after_turn = born.clone();
        after_turn.push_user("帮我看下这个接口".to_string());
        upsert_meta_row(&after_turn, "");

        let row = nuphus::store::session::get_session(&born.id)
            .unwrap()
            .expect("回填后行仍在");
        assert_eq!(
            row.summary, "接口联调复盘",
            "空标题回填必须保留既有 summary，不得打回派生标题"
        );

        let conn = nuphus::store::db::acquire().unwrap();
        let _ = nuphus::store::session::delete_session(&born.id);
        conn.execute(
            "DELETE FROM session_meta WHERE session_id = ?1",
            rusqlite::params![born.id],
        )
        .unwrap();
    }

    // ── 草稿对话（「新建项目文件夹」→ 尚未开说的空对话，内存态）──

    /// 草稿对话**不落任何持久化**：无 sessions 行、无 session_meta 归属行、无 mirror、
    /// 无 snapshot。这是「未发消息就退出 → 不留痕迹 / 重启不出现」的机制证据。
    /// 无项目目录时拒绝创建（宁可缺失不可错记），且不留半截状态。
    #[test]
    fn draft_session_leaves_no_persistence_trace() {
        let state = AppState::default();
        let dir = "E:\\__nuphus_test_proj__\\draft";
        let draft = create_draft_session(&state, "leader", dir).unwrap();

        assert_eq!(
            draft_session(&state).map(|d| d.id),
            Some(draft.id.clone()),
            "草稿登记在内存态（SessionState.draft_session）"
        );
        assert_eq!(draft.project_path, dir, "归属 = 创建时的项目目录快照");
        assert!(
            draft.created_at > 0,
            "启动时刻是真实时间戳（条目排序用），不造值"
        );

        assert!(
            nuphus::store::session::get_session(&draft.id)
                .unwrap()
                .is_none(),
            "草稿不得落 sessions 行"
        );
        assert!(
            nuphus::store::session::get_snapshot(&draft.id)
                .unwrap()
                .is_none(),
            "草稿不得写 SQLite 快照（= 内存镜像落盘点）"
        );
        assert!(read_mirror(&draft.id).is_none(), "草稿不得写展示台镜像");
        assert!(
            nuphus::store::session::session_project_paths(std::slice::from_ref(&draft.id))
                .unwrap()
                .is_empty(),
            "草稿不得登记 session_meta 归属行"
        );

        let blank = AppState::default();
        assert_eq!(
            create_draft_session(&blank, "leader", "   ").unwrap_err(),
            "no_project_dir",
            "未配置项目目录 → 拒绝，不猜路径"
        );
        assert!(draft_session(&blank).is_none(), "拒绝时不得留下半截草稿");
    }

    /// 草稿作为**当前对话**出现在 rail：`is_active` + `draft` 标记 + 归属路径来自内存
    /// 快照；该路径同时进入分组数据源（它没有任何 session_meta 行，前端据此不会把它
    /// 归入「未分组」）。
    #[test]
    fn list_shelf_shows_draft_as_active_in_its_project_group() {
        let state = AppState::default();
        let dir = "E:\\__nuphus_test_proj__\\draft-group";
        let draft = create_draft_session(&state, "leader", dir).unwrap();

        let payload = list_shelf_sessions_inner(&state).unwrap();
        let item = payload["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == serde_json::json!(draft.id))
            .expect("rail 应含草稿条目");
        assert_eq!(
            item["is_active"],
            serde_json::json!(true),
            "草稿就是当前对话（带着色高亮）"
        );
        assert_eq!(
            item["draft"],
            serde_json::json!(true),
            "前端据 draft 渲染「新建对话」并隐藏重命名/归档"
        );
        assert_eq!(
            item["project_path"],
            serde_json::json!(dir),
            "归属取创建时快照，不读库、不按当前目录推断"
        );
        assert_eq!(item["message_count"], serde_json::json!(0));
        assert_eq!(item["created_at"], serde_json::json!(draft.created_at));

        assert!(
            payload["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["path"] == serde_json::json!(dir)),
            "快照路径必须生成分组条目，否则前端会把它归入「未分组」"
        );
    }

    /// mode 不匹配的草稿不得冒充当前对话（切到 workflow 后 leader 草稿不再 active）。
    #[test]
    fn draft_session_only_active_in_its_own_mode() {
        let state = AppState::default();
        let draft =
            create_draft_session(&state, "workflow", "E:\\__nuphus_test_proj__\\wf").unwrap();
        {
            let mut cm = state.current_mode.write().unwrap();
            *cm = "leader".to_string();
        }

        let payload = list_shelf_sessions_inner(&state).unwrap();
        assert!(
            !payload["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == serde_json::json!(draft.id)),
            "当前 mode 是 leader 时不得展示 workflow 草稿"
        );
    }

    /// 未发消息就切走：切换到另一条会话（成功路径）后草稿被丢弃，且不再出现在列表里。
    #[test]
    fn draft_session_disappears_after_switching_to_another_session() {
        let state = AppState::default();
        let draft =
            create_draft_session(&state, "leader", "E:\\__nuphus_test_proj__\\switch").unwrap();

        let other = session_with_user(&["别的会话"]);
        let other_id = other.id.clone();
        {
            let mut shelf = state.shelf.lock().unwrap();
            shelf.put(
                build_entry(other_id.clone(), "leader", &other, Some("别的会话")),
                other,
            );
        }

        switch_session_inner_mode(&state, other_id, Some("leader".to_string())).unwrap();

        assert!(draft_session(&state).is_none(), "切换成功即丢弃草稿");
        let payload = list_shelf_sessions_inner(&state).unwrap();
        assert!(
            !payload["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == serde_json::json!(draft.id)),
            "切换后草稿不得继续留在 rail 上"
        );
    }

    /// 首条消息 → 真实会话在诞生点落成：草稿被清掉（归属/标题由诞生点既有逻辑负责），
    /// 不会出现「真实会话 + 草稿」两条当前对话。
    #[test]
    fn draft_session_cleared_when_real_session_is_born() {
        let state = AppState::default();
        let draft =
            create_draft_session(&state, "leader", "E:\\__nuphus_test_proj__\\birth").unwrap();

        let born = Session::new(); // 诞生点状态：全新 uuid + 空 session
        register_session_birth(&state, &born);

        assert!(draft_session(&state).is_none(), "真实会话诞生 → 草稿结束");
        let payload = list_shelf_sessions_inner(&state).unwrap();
        assert!(
            !payload["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == serde_json::json!(draft.id)),
            "诞生后 rail 只应剩真实会话"
        );

        // 自清理（诞生点按真实 prefs.project_dir 登记了归属行；未配置则无行）
        let conn = nuphus::store::db::acquire().unwrap();
        conn.execute(
            "DELETE FROM session_meta WHERE session_id = ?1",
            rusqlite::params![born.id],
        )
        .unwrap();
        let _ = nuphus::store::session::delete_session(&born.id);
    }

    /// 冷启动：草稿只活在 AppState 内存里——「新进程」（新 AppState）读不到它，
    /// 且它没有任何持久化可读（见 draft_session_leaves_no_persistence_trace），
    /// 因此重启后不会出现该空对话。
    #[test]
    fn draft_session_does_not_survive_cold_start() {
        let old = AppState::default();
        let draft =
            create_draft_session(&old, "leader", "E:\\__nuphus_test_proj__\\restart").unwrap();
        drop(old); // 等价于进程退出：内存态随实例消失

        let restarted = AppState::default();
        assert!(draft_session(&restarted).is_none(), "重启后不得恢复草稿");
        let payload = list_shelf_sessions_inner(&restarted).unwrap();
        assert!(
            !payload["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["id"] == serde_json::json!(draft.id)),
            "重启后的列表不得含草稿条目"
        );
    }
}
