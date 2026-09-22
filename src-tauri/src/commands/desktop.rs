// desktop.rs — Desktop tool independent commands
// Bypasses ToolRegistry / execute_tool / mock middleware
// Directly calls nuphus::desktop::DesktopClient
//
// ⚠ DesktopClient returns { success: true, result: data } wrapper format
// All commands use unwrap_result() to extract clean data for frontend
//
// ── 资源门（2026-09 加固）──
// 这些命令**绕过 ToolRegistry** 直连桌面，工具页（DesktopToolbar）与输入框粘贴
// 都会打到同一台物理桌面（剪贴板 / 屏幕 / 输入）。Agent 轮次或录制会话进行中时
// 与之并行会污染 agent 的自动化步骤（例如 agent 正在 ctrl+V 时用户改了剪贴板），
// 因此统一走资源门：拿不到即明确拒绝（稳定码 automation_busy），不排队不降级。

use crate::state::AppState;
use nuphus::automation_gate::{HoldKind, ResourceClass};
use tauri::State;

/// Extract raw data from DesktopClient's { success, result/error } wrapper
fn unwrap_result(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        Ok(value.get("result").cloned().unwrap_or(value))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// 手动桌面命令统一取锁（单次调用期间持有）。
///
/// 日志用 debug 级：`desktop_mouse_position` 在工具页是 60ms 轮询（被拒期间
/// 每帧一条 warn 会淹没日志），拒绝本身由调用方按需呈现给用户。
fn acquire_manual(
    state: &State<'_, AppState>,
    label: &str,
) -> Result<nuphus::automation_gate::AutomationLease, String> {
    state
        .automation_gate
        .try_acquire(ResourceClass::Desktop, HoldKind::ManualTool, label)
        .map_err(|busy| {
            tracing::debug!("[resource-gate] 拒绝手动桌面命令 {label}: {busy}");
            busy.to_string()
        })
}

/// Mouse current position — returns { x, y }
#[tauri::command]
pub async fn desktop_mouse_position(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _lease = acquire_manual(&state, "desktop_mouse_position")?;
    let client = nuphus::desktop::DesktopClient::new();
    let raw = client.mouse_position().await.map_err(|e| e.to_string())?;
    unwrap_result(raw)
}

/// 读取剪贴板中的文件路径（Windows CF_HDROP）。
///
/// **豁免资源门（2026-09-22 决定）**：本命令只做只读的剪贴板查询，不触碰鼠标 /
/// 键盘 / 截图等独占外设，也不经浏览器 CDP 单例 —— 不存在并发死锁路径。而执行中
/// 往输入框粘贴文件路径是常规操作，取锁会让它在任务运行期间静默退化，故刻意不取
/// `HoldKind::ManualTool` 租约。若日后本命令改为写操作或开始触碰独占资源，
/// **必须补回取锁**（同文件另两个桌面命令即为取锁范例）。
#[tauri::command]
pub async fn desktop_clipboard_read_file_paths() -> Result<serde_json::Value, String> {
    let client = nuphus::desktop::DesktopClient::new();
    let raw = client
        .clipboard_read_file_paths()
        .await
        .map_err(|e| e.to_string())?;
    unwrap_result(raw)
}

/// 写入剪贴板
#[tauri::command]
pub async fn desktop_clipboard_write(
    state: State<'_, AppState>,
    text: String,
) -> Result<serde_json::Value, String> {
    let _lease = acquire_manual(&state, "desktop_clipboard_write")?;
    let client = nuphus::desktop::DesktopClient::new();
    let raw = client
        .clipboard_write(&text)
        .await
        .map_err(|e| e.to_string())?;
    unwrap_result(raw)
}
