// rec.rs — 工作流录制命令层（rec_*）+ 录制会话状态机
//
// 职责（仅 src-tauri 壳，不动 nuphus crate）：
//   1. rec_set_workflow: 初始化录制会话（idle → active），创建 {workflow}/screenshots/
//   2. rec_start: 把 action_kind 映射为 rec_hook::CaptureKind 并阻塞捕获一次真实操作
//   3. rec_cancel: 取消进行中的捕获（幂等，透传 rec_hook::rec_cancel_current）
//   4. rec_abort: 放弃整个录制会话（取消捕获 + 回 idle）
//   5. rec_complete: 收前端 steps 数组落盘 record-draft.{ts}.json，会话回 idle
//   6. rec_session_status: 只读查询会话状态
//   7. rec_save_pending: 进度持久化——drafts 落盘 record-draft.pending.json，会话回 idle
//   8. rec_load_pending: 读取上次保存的待恢复进度（begin 自动恢复用）
//   9. rec_discard_pending: 幂等删除当前会话 workflow 的 pending 文件（草稿清空用）
//
// 录制会话状态机：
//   idle（None） ←→ active（Some(RecSession)）
//   - rec_set_workflow  → active（覆盖重置）
//   - rec_abort/rec_complete/rec_save_pending → idle
//   - 捕获期间置 REC_CAPTURING=true（AtomicBool 防重入，rec_hook 内部共享全局
//     statics，并发 capture_once 会互相污染结果通道，必须串行化）
//
// 进度持久化：plugin/workflows/{workflow_id}/record-draft.pending.json（按 workflow_id
// 天然隔离）。rec_complete 终稿写盘后自动删除 pending——终稿已交付，旧进度失效。
//
// overlay 联动：toolbar.rs 的 overlay_capture_confirm(mode="rec_region"/"rec_template")
// 通过 rec_active_screenshots_dir() 取当前会话截图目录保存 ROI 证据 / find_image 模板。
// 录制截图一律走 PRE_SCREENSHOT 预截图裁剪（toolbar.rs 已保证），禁止 live capture。

use nuphus::desktop::rec_hook;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::Manager;

/// 录制会话 active 状态
struct RecSession {
    workflow_id: String,
    workflow_dir: PathBuf,
    screenshots_dir: PathBuf,
    started_at_ms: u64,
}

/// 会话状态：None = idle；Some = active
static REC_SESSION: Mutex<Option<RecSession>> = Mutex::new(None);
/// 捕获防重入守卫（true = 有一个 capture_once 在跑）
static REC_CAPTURING: AtomicBool = AtomicBool::new(false);

/// RAII：无论成功/取消/超时/join 异常都复位捕获标志
struct CaptureGuard;

impl CaptureGuard {
    /// 尝试获取捕获权；已在捕获中则返回明确错误
    fn try_acquire() -> Result<Self, String> {
        if REC_CAPTURING.swap(true, Ordering::SeqCst) {
            return Err("已有录制捕获正在进行中，请先 rec_cancel 或等待完成".to_string());
        }
        Ok(CaptureGuard)
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        REC_CAPTURING.store(false, Ordering::SeqCst);
    }
}

/// 会话信息（rec_set_workflow / rec_session_status 返回）
#[derive(Debug, Clone, Serialize)]
pub struct RecSessionInfo {
    pub status: String, // "idle" | "active"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshots_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// workflow 根目录（与 workflow store / canvas_layout_path 同源：项目根/plugin/workflows）
fn workflows_root() -> PathBuf {
    nuphus::utils::workspace_root()
        .join("plugin")
        .join("workflows")
}

/// 校验 workflow_id：非空、不含路径分隔符 / ".."（防目录穿越）
fn validate_workflow_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("workflow_id 不能为空".to_string());
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(format!("workflow_id 含非法字符: {id}"));
    }
    Ok(())
}

/// 进度持久化 checkpoint 文件名（位于 {workflow_dir}/ 下，按 workflow 隔离）
const PENDING_FILE: &str = "record-draft.pending.json";

fn pending_path(workflow_dir: &Path) -> PathBuf {
    workflow_dir.join(PENDING_FILE)
}

fn snapshot() -> RecSessionInfo {
    let guard = REC_SESSION.lock().expect("rec session lock poisoned");
    match guard.as_ref() {
        None => RecSessionInfo {
            status: "idle".to_string(),
            workflow_id: None,
            workflow_dir: None,
            screenshots_dir: None,
            started_at_ms: None,
        },
        Some(s) => RecSessionInfo {
            status: "active".to_string(),
            workflow_id: Some(s.workflow_id.clone()),
            workflow_dir: Some(s.workflow_dir.display().to_string()),
            screenshots_dir: Some(s.screenshots_dir.display().to_string()),
            started_at_ms: Some(s.started_at_ms),
        },
    }
}

/// toolbar.rs overlay 联动入口：当前 active 会话的截图目录（无会话返回明确错误）
pub(crate) fn rec_active_screenshots_dir() -> Result<PathBuf, String> {
    let guard = REC_SESSION.lock().expect("rec session lock poisoned");
    guard
        .as_ref()
        .map(|s| s.screenshots_dir.clone())
        .ok_or_else(|| "录制会话未初始化，请先调用 rec_set_workflow".to_string())
}

/// rec_browser 等扩展命令的会话检查：当前必须处于 active 录制会话。
/// 只读探测（不消费会话），失败给出与 rec_* 一致的中文提示。
pub(crate) fn rec_session_ensure_active() -> Result<(), String> {
    let guard = REC_SESSION.lock().expect("rec session lock poisoned");
    if guard.is_none() {
        return Err("录制会话未初始化，请先调用 rec_set_workflow".to_string());
    }
    Ok(())
}

// ═════════════════════════════════════════════════════════════
// 命令
// ═════════════════════════════════════════════════════════════

/// 初始化录制会话：创建 {workflow}/screenshots/，会话进入 active。
/// 重复调用 = 切换到新 workflow 并重置会话（幂等语义）。
///
/// 全局执行闸门：录制 = 系统操作（低层 hook 捕获真实桌面事件），与 Agent 任务 /
/// 进行中的 workflow 禁并行 —— busy 或 active workflow 存在时拒绝进入。
/// 只挡入口：rec_start 之前必经过 rec_set_workflow，会话建立后系统捕获归录制会话独占。
#[tauri::command]
pub async fn rec_set_workflow(
    state: tauri::State<'_, crate::state::AppState>,
    workflow_id: String,
) -> Result<RecSessionInfo, String> {
    // ── 全局执行闸门（后端兜底；前端锁定态已先拦截）──
    let busy = state.busy.load(Ordering::SeqCst);
    let engine_active = {
        let engine = state.workflow_engine.read().await;
        engine.active_run_info().is_some()
    };
    if busy || engine_active {
        return Err("当前有任务执行中，暂不可用！请等待完成后再录制".to_string());
    }

    validate_workflow_id(&workflow_id)?;

    let workflow_dir = workflows_root().join(&workflow_id);
    let screenshots_dir = workflow_dir.join("screenshots");
    std::fs::create_dir_all(&screenshots_dir).map_err(|e| format!("创建录制截图目录失败: {e}"))?;

    let session = RecSession {
        workflow_id: workflow_id.clone(),
        workflow_dir,
        screenshots_dir,
        started_at_ms: now_ms(),
    };
    {
        // 锁必须在块作用域内释放——snapshot() 会再次 lock 同一 Mutex，
        // std Mutex 非重入，若 guard 存活到函数尾则同线程二次 lock 死锁（曾致点击录制整窗卡死）。
        let mut guard = REC_SESSION.lock().expect("rec session lock poisoned");
        *guard = Some(session);
    }

    tracing::info!("[rec] rec_set_workflow: active for '{}'", workflow_id);
    Ok(snapshot())
}

/// 只读查询录制会话状态
#[tauri::command]
pub fn rec_session_status() -> Result<RecSessionInfo, String> {
    Ok(snapshot())
}

// ═════════════════════════════════════════════════════════
// 主窗窗口生命周期（录制捕获期间让路 / 结束后复原）
// ═════════════════════════════════════════════════════════

/// 主窗最小化让路（捕获开始前调用）：minimize 而非 hide——任务栏可见，用户可点回主窗
/// 查看/取消；webview 最小化后 JS 运行与 invoke 链不受影响。best-effort：主窗不存在/
/// 已最小化则幂等跳过，失败不阻断捕获本身。
fn minimize_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.minimize();
    }
}

/// 主窗复原激活（捕获结束全路径收尾调用）：unminimize + show + set_focus。
/// best-effort 且幂等；主窗不存在/已关闭则跳过不报错（与 toolbar.rs overlay 复原同模式）。
fn restore_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 捕获一次真实桌面操作。action_kind ∈ click/scroll/hotkey/any。
/// 阻塞直到目标事件/取消/超时（默认 60s）；期间可并发调用 rec_cancel。
/// 返回 rec_hook::CaptureEvent 的 serde 直序列化 JSON。
#[tauri::command]
pub async fn rec_start(
    app: tauri::AppHandle,
    action_kind: String,
    timeout_secs: Option<u64>,
) -> Result<rec_hook::CaptureEvent, String> {
    let _guard = CaptureGuard::try_acquire()?;
    let kind = rec_hook::CaptureKind::from_str(&action_kind)?;
    let timeout = timeout_secs.unwrap_or(60);

    // 1) 主窗最小化让路：hook 捕获期间把前台让给目标窗口，用户在 Nuphus 内的点击不再被
    //    hook 当捕获事件返回（设计意图「主窗收起让路」此前从未实现——自窗口误捕获根因）。
    minimize_main_window(&app);

    // 2) capture_once 是同步阻塞（最长 timeout 秒），放入阻塞线程池避免卡 UI 线程。
    //    第三个参数 ignore_self_window=true：rec_hook 后端过滤 Nuphus 自身窗口事件。
    //    join 结果先落变量、收尾（恢复窗口）再返回——保证 Ok / Err(取消) / Err(超时) /
    //    join 异常 全路径都执行窗口复原（Rust async 无 try-finally，用统一收尾表达 finally）。
    let joined =
        tauri::async_runtime::spawn_blocking(move || rec_hook::capture_once(kind, timeout, true))
            .await;

    // 3) finally 收尾：无论捕获成功/取消/超时/线程异常，主窗复原并激活。
    restore_main_window(&app);

    joined.map_err(|e| format!("录制捕获线程异常退出: {e}"))?
}

/// 取消进行中的录制捕获（幂等，不改变会话 active/idle）
#[tauri::command]
pub fn rec_cancel() -> Result<(), String> {
    rec_hook::rec_cancel_current();
    Ok(())
}

/// 放弃整个录制会话：取消捕获 + 回 idle（丢弃当前 workflow 关联）
#[tauri::command]
pub fn rec_abort() -> Result<(), String> {
    rec_hook::rec_cancel_current();
    let mut guard = REC_SESSION.lock().expect("rec session lock poisoned");
    *guard = None;
    Ok(())
}

/// 完成录制：收前端组装好的 steps 数组（serde_json）+ 可选 user_notes，
/// 落盘 plugin/workflows/{workflow_id}/record-draft.{ts}.json，会话回 idle。
/// 返回 { path, workflow_id, created_at, step_count }。
#[tauri::command]
pub fn rec_complete(
    steps: serde_json::Value,
    user_notes: Option<String>,
) -> Result<serde_json::Value, String> {
    if !steps.is_array() {
        return Err("steps 必须是数组".to_string());
    }
    let step_count = steps.as_array().map(|a| a.len()).unwrap_or(0);

    // 克隆会话信息（不消费）；写盘成功后才置 idle，失败保留可重试
    let (workflow_id, workflow_dir) = {
        let guard = REC_SESSION.lock().expect("rec session lock poisoned");
        let session = guard
            .as_ref()
            .ok_or_else(|| "录制会话未初始化，请先调用 rec_set_workflow".to_string())?;
        (session.workflow_id.clone(), session.workflow_dir.clone())
    };

    std::fs::create_dir_all(&workflow_dir).map_err(|e| format!("创建工作流目录失败: {e}"))?;

    let now = chrono::Utc::now();
    let ts = now.format("%Y%m%d_%H%M%S_%3f");
    let created_at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let file_name = format!("record-draft.{ts}.json");
    let draft_path = workflow_dir.join(&file_name);

    let payload = serde_json::json!({
        "workflow_id": workflow_id,
        "created_at": created_at,
        "steps": steps,
        "user_notes": user_notes,
    });
    let json_text = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("序列化 record-draft 失败: {e}"))?;
    std::fs::write(&draft_path, json_text).map_err(|e| format!("写入 record-draft 失败: {e}"))?;

    // 终稿已交付 → 旧待恢复进度失效，删除 pending（尽力而为：失败仅告警，不阻断完成——
    // 终稿已成功写盘，若此处报错会让前端误以为整个完成流程失败）
    let pending = pending_path(&workflow_dir);
    if pending.exists() {
        if let Err(e) = std::fs::remove_file(&pending) {
            tracing::warn!("[rec] rec_complete: 清理待恢复进度失败: {e}");
        }
    }

    // 写盘成功 → 会话回 idle
    {
        let mut guard = REC_SESSION.lock().expect("rec session lock poisoned");
        *guard = None;
    }

    tracing::info!(
        "[rec] rec_complete: draft '{}' ({} steps) for '{}'",
        draft_path.display(),
        step_count,
        workflow_id
    );

    Ok(serde_json::json!({
        "path": draft_path.display().to_string(),
        "workflow_id": workflow_id,
        "created_at": created_at,
        "step_count": step_count,
    }))
}

/// 保存录制进度：收前端 drafts 数组（step_draft + canvas_step_id）落盘
/// plugin/workflows/{workflow_id}/record-draft.pending.json，写盘成功后会话回 idle。
/// 下次 begin 自动恢复该文件可继续录制（完成终稿后此文件会被删除）。
/// 返回 { path, workflow_id, step_count }。
#[tauri::command]
pub fn rec_save_pending(
    steps: serde_json::Value,
    user_notes: Option<String>,
) -> Result<serde_json::Value, String> {
    if !steps.is_array() {
        return Err("steps 必须是数组".to_string());
    }
    let step_count = steps.as_array().map(|a| a.len()).unwrap_or(0);

    // 克隆会话信息；写盘成功后才置 idle，失败保留会话可继续录制/重试
    let (workflow_id, workflow_dir) = {
        let guard = REC_SESSION.lock().expect("rec session lock poisoned");
        let session = guard
            .as_ref()
            .ok_or_else(|| "录制会话未初始化，请先调用 rec_set_workflow".to_string())?;
        (session.workflow_id.clone(), session.workflow_dir.clone())
    };

    std::fs::create_dir_all(&workflow_dir).map_err(|e| format!("创建工作流目录失败: {e}"))?;

    let saved_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let path = pending_path(&workflow_dir);

    let payload = serde_json::json!({
        "workflow_id": workflow_id,
        "saved_at": saved_at,
        "steps": steps,
        "user_notes": user_notes,
    });
    let json_text =
        serde_json::to_string_pretty(&payload).map_err(|e| format!("序列化录制进度失败: {e}"))?;
    std::fs::write(&path, json_text).map_err(|e| format!("写入录制进度失败: {e}"))?;

    // 写盘成功 → 会话回 idle（块内释放 guard，同 rec_complete 模式）
    {
        let mut guard = REC_SESSION.lock().expect("rec session lock poisoned");
        *guard = None;
    }

    tracing::info!(
        "[rec] rec_save_pending: '{}' ({} steps) for '{}'",
        path.display(),
        step_count,
        workflow_id
    );

    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "workflow_id": workflow_id,
        "step_count": step_count,
    }))
}

/// 读取上次保存的录制进度（会话必须 active）。
/// 文件存在 → { exists:true, workflow_id, saved_at, steps, user_notes }；
/// 不存在 → { exists:false }。
#[tauri::command]
pub fn rec_load_pending() -> Result<serde_json::Value, String> {
    let (workflow_id, workflow_dir) = {
        let guard = REC_SESSION.lock().expect("rec session lock poisoned");
        let session = guard
            .as_ref()
            .ok_or_else(|| "录制会话未初始化，请先调用 rec_set_workflow".to_string())?;
        (session.workflow_id.clone(), session.workflow_dir.clone())
    };

    let path = pending_path(&workflow_dir);
    if !path.exists() {
        return Ok(serde_json::json!({ "exists": false }));
    }

    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取录制进度失败: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("录制进度文件损坏: {e}（可放弃后重新录制覆盖）"))?;

    Ok(serde_json::json!({
        "exists": true,
        "workflow_id": workflow_id,
        "saved_at": value.get("saved_at").cloned().unwrap_or(serde_json::Value::Null),
        "steps": value.get("steps").cloned().unwrap_or_else(|| serde_json::json!([])),
        "user_notes": value.get("user_notes").cloned().unwrap_or(serde_json::Value::Null),
    }))
}

/// 幂等删除当前会话 workflow 的待恢复进度文件（文件不存在也 Ok）；不改会话状态。
/// 会话不在 active（无 workflow 上下文）时同样 Ok——无处可删即视为已清理。
#[tauri::command]
pub fn rec_discard_pending() -> Result<(), String> {
    let workflow_dir = {
        let guard = REC_SESSION.lock().expect("rec session lock poisoned");
        guard.as_ref().map(|s| s.workflow_dir.clone())
    };
    if let Some(dir) = workflow_dir {
        let path = pending_path(&dir);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删除录制进度失败: {e}"))?;
            tracing::info!("[rec] rec_discard_pending: removed '{}'", path.display());
        }
    }
    Ok(())
}

// ── 单元测试（纯逻辑，不触碰 hook/窗口）──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_workflow_id_ok() {
        assert!(validate_workflow_id("wf-2026-09-02_abc").is_ok());
        assert!(validate_workflow_id("a1").is_ok());
    }

    #[test]
    fn validate_workflow_id_rejects() {
        assert!(validate_workflow_id("").is_err());
        assert!(validate_workflow_id("  ").is_err());
        assert!(validate_workflow_id("a/b").is_err());
        assert!(validate_workflow_id("a\\b").is_err());
        assert!(validate_workflow_id("../x").is_err());
        assert!(validate_workflow_id("x/../y").is_err());
    }

    #[test]
    fn capture_kind_roundtrip() {
        assert_eq!(
            rec_hook::CaptureKind::from_str("click"),
            Ok(rec_hook::CaptureKind::Click)
        );
        assert_eq!(
            rec_hook::CaptureKind::from_str("Scroll"),
            Ok(rec_hook::CaptureKind::Scroll)
        );
        assert_eq!(
            rec_hook::CaptureKind::from_str("hotkey"),
            Ok(rec_hook::CaptureKind::Hotkey)
        );
        assert_eq!(
            rec_hook::CaptureKind::from_str("ANY"),
            Ok(rec_hook::CaptureKind::Any)
        );
        assert!(rec_hook::CaptureKind::from_str("typing").is_err());
    }

    #[test]
    fn workflows_root_layout() {
        // workflows_root 应以 workspace_root/plugin/workflows 结尾（路径可判定而非硬编码盘符）
        let root = workflows_root();
        assert_eq!(root.file_name().and_then(|s| s.to_str()), Some("workflows"));
        assert_eq!(
            root.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str()),
            Some("plugin")
        );
    }

    #[test]
    fn pending_path_layout() {
        // pending checkpoint 落在 {workflow_dir}/record-draft.pending.json（相对路径防盘符差异）
        let dir = PathBuf::from("plugin/workflows/wf-2026-09-02_abc");
        assert_eq!(
            pending_path(&dir),
            PathBuf::from("plugin/workflows/wf-2026-09-02_abc/record-draft.pending.json")
        );
        assert_eq!(PENDING_FILE, "record-draft.pending.json");
    }
}
