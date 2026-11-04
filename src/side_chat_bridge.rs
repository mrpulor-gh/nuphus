//! side_chat_bridge — side_chat_dispatch 工具执行桥（注入点）
//!
//! nuphus lib 不依赖 Tauri；Leader 派发临时伙伴的完整闭环（选壳 / 取壳 /
//! build/复用 / run_free / 交付门禁）全部在桌面壳（src-tauri/src/commands/
//! side_chat.rs 的 `side_chat_dispatch`），本模块只提供 fn 指针注入桥——
//! 模式对齐 ext_agent_bridge / render_bridge / video_bridge（单进程直调，
//! 无 IPC）。
//!
//! 契约：注册的 fn 接收工具原始参数 JSON，返回工具结果 JSON 字符串：
//!   ok:      {"reply":"...","steps":[...],"ctx_used":N,"turn_id":"...","busy":false}
//!   busy:    {"busy":true} —— 用户正在用伙伴面板，Leader 据此降级（正常回执，非错误）
//! 错误经 Err 返回（工具层渲染为 ToolResult::failure）。

use std::sync::OnceLock;

/// 桥实现签名：工具参数 JSON → 工具结果 JSON。
pub type SideChatDispatchImpl = fn(&serde_json::Value) -> Result<String, String>;

static IMPL: OnceLock<SideChatDispatchImpl> = OnceLock::new();

/// 桌面壳启动时注册（幂等：首次生效）。
pub fn register_side_chat_dispatch_impl(f: SideChatDispatchImpl) {
    let _ = IMPL.set(f);
}

/// 桌面壳是否已注册桥实现。
pub fn is_available() -> bool {
    IMPL.get().is_some()
}

/// 调用派发闭环。未注册（如 headless 构建）→ 诚实报错，绝不伪造结果。
pub fn dispatch(params: &serde_json::Value) -> Result<String, String> {
    match IMPL.get() {
        Some(f) => f(params),
        None => Err("side_chat_dispatch 不可用（桌面壳未注册 side_chat bridge）".to_string()),
    }
}
