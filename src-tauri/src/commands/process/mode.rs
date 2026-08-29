//! Mode 命令 — 获取/切换运行模式

use crate::emitter::CompoundEmitter;
use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use std::str::FromStr;
use tauri::State;

/// 设置运行模式（"leader" / "workflow"）
///
/// 无效模式（含旧版残留的 "free"/"plan"）fallback 到 Leader 默认值，
/// 不报错——旧持久化数据不得导致功能不可用。
///
/// 模式切换：
/// - 切到 Workflow: 保留现有 workflow_agent（若存在），否则懒初始化在 send_message_cmd 中完成
/// - 切到 Leader: 保留 workflow_agent（session 不丢失），同步更新 leader_agent mode
///
/// 泛型核心：桌面 IPC（具体 Wry thin wrapper `set_mode`）与手机端 mobile_server
/// （泛型 Runtime）共用同一实现；`#[tauri::command]` 由下方 thin wrapper 提供。
pub async fn set_mode_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    let parsed = nuphus::runtime::Mode::from_str(&mode).unwrap_or_else(|_| {
        tracing::warn!("[MODE] Unknown mode '{}', falling back to leader", mode);
        nuphus::runtime::Mode::default()
    });
    let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
    match parsed {
        nuphus::runtime::Mode::Workflow => {
            // WorkflowAgent lazy init in send_message_cmd
            tracing::info!("[MODE] Switched to: workflow");
        }
        nuphus::runtime::Mode::Leader => {
            if let Some(ref mut runtime) = guard.leader_agent {
                runtime.set_mode(parsed);
            }
            tracing::info!("[MODE] Switched to: leader");
        }
        nuphus::runtime::Mode::Custom => {
            // Custom runs on the Leader main loop; L2 comes from active custom card.
            // set_mode triggers prompt-cache invalidation so L2 rebuilds from the card.
            if let Some(ref mut runtime) = guard.leader_agent {
                runtime.set_mode(parsed);
            }
            tracing::info!("[MODE] Switched to: custom");
        }
    }
    drop(guard);

    // 后端权威 current_mode：chat_history 按此选择 agent 会话（替换旧「最近活跃」猜测）。
    // current_mode 是独立 RwLock，与已释放的 runtime 锁无冲突。
    // 切换语义（2026-08-30 解耦后）：手动切换只更新 current_mode，不设任何 pending 状态。
    // 会话归属判定完全由 submit_user_message 实时比较「发送 mode vs session 绑定 mode」
    // 决定（规则2）：不一致 → 新建该 mode 会话；一致 → 续聊当前 session。
    {
        if let Ok(mut cm) = state.current_mode.write() {
            *cm = parsed.as_str().to_string();
        }
    }

    // 广播 mode 变更：双推桌面 Tauri + 手机 WS（mobile_server 未启动时 CompoundEmitter
    // 退化为纯 Tauri 推送，桌面端零回归）。
    // 后端是 mode 的唯一权威源：桌面 set_mode 与手机 /switch-mode 共用此命令，
    // 切换后双端（手机自身 + 桌面端实时一致）同步「当前模式」。
    let emitter = CompoundEmitter::new(app, &state);
    emitter.emit(NuphusEvent::ModeChanged {
        mode: parsed.as_str().to_string(),
    });

    Ok(())
}

/// 桌面 IPC 命令入口（thin wrapper）：委托泛型核心 `set_mode_impl`。
/// 桌面端切换运行模式时由前端 invoke；事件双推（桌面 Tauri + 手机 WS）。
#[tauri::command]
pub async fn set_mode(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    set_mode_impl(app, state, mode).await
}

/// 获取当前权威 mode：前端启动时调用，使 mode state 与后端镜像恢复结果一致
/// （启动恢复 current_mode from 镜像——leader/workflow/custom 三态）。
#[tauri::command]
pub fn get_current_mode(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string()))
}
