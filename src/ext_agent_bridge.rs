//! ext_agent_bridge — agent_dispatch 工具执行桥（注入点）
//!
//! nuphus lib 不依赖 Tauri；agent_dispatch 的完整编排（上板/进程捕获/SeqRunner/await
//! 门铃/超时自检）全部在桌面壳（src-tauri/src/ext_agent/），本模块只提供 fn 指针
//! 注入桥 —— 模式对齐 video_bridge / render_bridge（单进程直调，无 IPC）。
//!
//! 契约：注册的 fn 接收工具原始参数 JSON，返回工具结果 JSON 字符串：
//!   ok:      {"ok":true,"brief_path":"...","confirmed":"ready|progress","summary":"..."}
//!   timeout: {"timeout":true,"self_check":"..."}
//! 错误经 Err 返回（工具层渲染为 ToolResult::failure）。

use std::sync::OnceLock;

/// 桥实现签名：工具参数 JSON → 工具结果 JSON。
pub type AgentDispatchImpl = fn(&serde_json::Value) -> Result<String, String>;

static IMPL: OnceLock<AgentDispatchImpl> = OnceLock::new();

/// 桌面壳启动时注册（幂等：首次生效）。
pub fn register_agent_dispatch_impl(f: AgentDispatchImpl) {
    let _ = IMPL.set(f);
}

/// 桌面壳是否已注册桥实现。
pub fn is_available() -> bool {
    IMPL.get().is_some()
}

/// 调用 agent_dispatch 编排。未注册（如 headless 构建）→ 诚实报错，绝不伪造结果。
pub fn dispatch(params: &serde_json::Value) -> Result<String, String> {
    match IMPL.get() {
        Some(f) => f(params),
        None => Err("agent_dispatch 不可用（桌面壳未注册 ext_agent bridge）".to_string()),
    }
}
