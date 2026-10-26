use nuphus::agent::events::{EventEmitter, NuphusEvent};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// 窗口置前但【不抢键盘焦点】。
///
/// 大王要求：执行完成窗口自动置前（能看到结果），但输入框不自动聚焦
/// （不打断用户在其他窗口打字）。Tauri 的 set_focus() 会同时置前+激活抢焦点，
/// 无 NOACTIVATE 变体——这里用原生 SetWindowPos(HWND_TOP, SWP_NOACTIVATE) 实现
/// 「z-order 提升到最前，但不改变键盘焦点」。
#[cfg(target_os = "windows")]
fn bring_to_front_no_activate<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let _ = win.show();
    let _ = win.unminimize();
    if let Ok(hwnd) = win.hwnd() {
        // tauri hwnd() 返回 windows crate 的 HWND(pub *mut c_void)，取 .0 原始指针
        let raw: *mut core::ffi::c_void = hwnd.0;
        unsafe {
            let _ = SetWindowPos(
                raw,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn bring_to_front_no_activate<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    // 非 Windows 平台无 NOACTIVATE 语义，退化为 show（不抢焦点即可）
    let _ = win.show();
    let _ = win.unminimize();
}

#[derive(Debug, Clone, Serialize)]
pub struct FramedEvent {
    pub seq: u64,
    #[serde(rename = "event")]
    pub inner: NuphusEvent,
}

// Runtime 泛化（默认 Wry）：生产路径零改动；测试可用 MockRuntime 构造完整事件链。
// Clone 手写实现——derive 会额外要求 R: Clone（MockRuntime 不满足）。
pub struct TauriEventEmitter<R: tauri::Runtime = tauri::Wry> {
    pub app: tauri::AppHandle<R>,
    pub seq: Arc<AtomicU64>,
}

impl<R: tauri::Runtime> Clone for TauriEventEmitter<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            seq: self.seq.clone(),
        }
    }
}

impl<R: tauri::Runtime> EventEmitter for TauriEventEmitter<R> {
    fn emit(&self, event: NuphusEvent) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);

        // ── 窗口可见性管理 ──
        // HudUpdate: 恢复 HUD 窗口尺寸+位置，确保可见（创建时 visible=false）
        // UserInputRequest: 确保主窗口在前台（用户能看到输入弹窗）
        // ExecutionCompleted: Leader/工作流完成时主窗口置顶
        match &event {
            NuphusEvent::HudUpdate { text, phase, .. } => {
                // 并行工具计时仲裁：running 文本始终跟随活跃集 top（system 族 >
                // desktop/browser），终态经过时若留任工具则切回其 running。
                let (text, phase) = if phase == "running" {
                    match nuphus::hud_arbiter::on_running_hint() {
                        Some(rewritten) => (rewritten, phase.clone()),
                        None => (text.clone(), phase.clone()),
                    }
                } else if phase != "workflow" && phase != "workflow_wait" {
                    match nuphus::hud_arbiter::on_hud_terminal() {
                        Some(remain_text) => {
                            // 留任工具仍在跑：本终态帧之外补发其 running（fresh seq）
                            let seq2 = self.seq.fetch_add(1, Ordering::SeqCst);
                            crate::commands::hud::show(&self.app, &remain_text, "running", seq2);
                            (text.clone(), phase.clone())
                        }
                        None => (text.clone(), phase.clone()),
                    }
                } else {
                    (text.clone(), phase.clone())
                };
                // Delegate to shared show/hide logic (off-screen positioning,
                // force-close fallback, etc.)
                if phase == "hidden" || phase == "hide" {
                    crate::commands::hud::hide(&self.app);
                } else {
                    // HUD 消费端按全局 seq 单调排序：迟到/乱序的 running 不会再
                    // 覆盖已落地的终态 done（计时器完成不退表的根源）。直调
                    // show 的旁路（main.rs / llm.rs）传 seq=0，前端恒生效。
                    crate::commands::hud::show(&self.app, &text, &phase, seq);
                }
            }
            NuphusEvent::ToolCallStart {
                call_id,
                tool_name,
                params,
                ..
            } => {
                // 并行工具注册：更高优先级（system 族 > desktop/browser）进场即切换。
                // params 用于 system_shell 的 `title`（调用方填的短标题 → HUD 直显）。
                if let Some(text) =
                    nuphus::hud_arbiter::on_tool_call_start_with(call_id, tool_name, Some(params))
                {
                    let seq2 = self.seq.fetch_add(1, Ordering::SeqCst);
                    crate::commands::hud::show(&self.app, &text, "running", seq2);
                }
            }
            NuphusEvent::ToolCallEnd {
                call_id,
                tool_name,
                success,
                ..
            } => {
                use nuphus::hud_arbiter::{on_tool_call_end, ToolEndAction};
                match on_tool_call_end(call_id, tool_name, *success) {
                    ToolEndAction::None => {}
                    ToolEndAction::SwitchRunning(text) => {
                        let seq2 = self.seq.fetch_add(1, Ordering::SeqCst);
                        crate::commands::hud::show(&self.app, &text, "running", seq2);
                    }
                    ToolEndAction::SynthTerminal { phase, text } => {
                        // 活跃集清空且仍处 running 显示：合成终态收尾计时
                        // （覆盖 sub_task 提示工具/shell 前台超时等无终态路径）
                        let seq2 = self.seq.fetch_add(1, Ordering::SeqCst);
                        crate::commands::hud::show(&self.app, &text, phase, seq2);
                    }
                }
            }
            // 任务级终局/中断：中断路径 ToolCallEnd 缺席，清场防泄漏
            NuphusEvent::ExecutionError { .. } => {
                nuphus::hud_arbiter::flush_all();
            }
            NuphusEvent::ExecutionCompleted { .. } => {
                // 不做窗口激活——子任务完成（sub_task_loop）和全局完成
                // 都会发此事件。不做 show/focus，避免每次工具调用都抢前台。
                nuphus::hud_arbiter::flush_all();
            }
            NuphusEvent::LeaderDone { .. } => {
                // 窗口置前但【不抢键盘焦点】：执行完成窗口自动跑到最前
                // （大王要求保留置前），但输入框不自动聚焦、不打断其他窗口打字
                // （2026-08-08 大王要求移除执行完自动聚焦）。
                nuphus::hud_arbiter::flush_all();
                if let Some(main) = self.app.get_webview_window("main") {
                    bring_to_front_no_activate(&main);
                }
            }
            NuphusEvent::UserInputRequest { .. } => {
                if let Some(main) = self.app.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            }
            NuphusEvent::SecurityCheck { tool, .. }
                if tool == nuphus::tools::desktop_approval::EVENT_TOOL =>
            {
                if let Some(main) = self.app.get_webview_window("main") {
                    bring_to_front_no_activate(&main);
                }
            }
            _ => {}
        }

        let _ = self
            .app
            .emit("nuphus-event", FramedEvent { seq, inner: event });
    }
}
// ============================================================================
// CompoundEmitter — 事件双推（桌面 Tauri + 手机 WS）
// ============================================================================

/// 移动端 WS 推送端：序列化事件后经 broadcast channel 发给所有已连接的手机客户端。
/// 复活的旧 GatewayEventEmitter 模式（2fd603e 删除），单 mpsc 升级为 broadcast（多客户端）。
#[derive(Clone)]
pub struct MobileWsEmitter {
    tx: tokio::sync::broadcast::Sender<String>,
}

impl MobileWsEmitter {
    pub fn new(tx: tokio::sync::broadcast::Sender<String>) -> Self {
        Self { tx }
    }
}

impl EventEmitter for MobileWsEmitter {
    fn emit(&self, event: NuphusEvent) {
        let json = match serde_json::to_string(&event) {
            Ok(json) => json,
            Err(e) => {
                tracing::error!("[MobileWsEmitter] serialize error: {}", e);
                return;
            }
        };
        // 无接收者（手机未连接）是常态，静默丢弃；channel 满时 lagged 由订阅侧处理
        let _ = self.tx.send(json);
    }
}

/// 复合 emitter：事件同时推桌面（Tauri IPC）与手机（WebSocket broadcast）。
/// `mobile` 为 None（mobile_server 未启动）时退化为纯 Tauri 推送，行为与
/// 单独使用 TauriEventEmitter 完全等价（桌面零回归保证）。
///
/// 注意：构造时 seq 只取一次快照——Tauri 端 seq 由 TauriEventEmitter 内部
/// 维护共享计数，WS 端推送的是裸 NuphusEvent JSON（不含 seq 帧头），
/// 手机客户端如需排序可在 P2 自行包装。
pub struct CompoundEmitter<R: tauri::Runtime = tauri::Wry> {
    pub tauri: TauriEventEmitter<R>,
    pub mobile: Option<MobileWsEmitter>,
}

impl<R: tauri::Runtime> Clone for CompoundEmitter<R> {
    fn clone(&self) -> Self {
        Self {
            tauri: self.tauri.clone(),
            mobile: self.mobile.clone(),
        }
    }
}

impl<R: tauri::Runtime> CompoundEmitter<R> {
    /// 从 AppState 当前状态构造：server 运行（mobile_ws_tx = Some）→ 双推；否则纯 Tauri。
    pub fn new(app: tauri::AppHandle<R>, state: &crate::state::AppState) -> Self {
        let mobile = state
            .mobile_ws_tx
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(MobileWsEmitter::new);
        Self {
            tauri: TauriEventEmitter {
                app,
                seq: state.event_seq.clone(),
            },
            mobile,
        }
    }
}

impl<R: tauri::Runtime> EventEmitter for CompoundEmitter<R> {
    fn emit(&self, event: NuphusEvent) {
        self.tauri.emit(event.clone());
        if let Some(ref m) = self.mobile {
            m.emit(event);
        }
    }
}
