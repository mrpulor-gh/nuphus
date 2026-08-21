//! custom_agent — Tauri Custom Agent CRUD + 激活命令
//!
//! 对应 nuphus::custom_agents::CustomAgentStore 的静态方法。
//! set_active_custom_agent 在当前已是 Custom 模式时，同步刷新
//! 工具白名单 / 记忆归属全局状态 / prompt 缓存（换卡即生效，无需重启会话）。

use nuphus::custom_agents::{CustomAgentConfig, CustomAgentStore};
use tauri::State;

use crate::state::AppState;

/// 列出所有 Custom Agent 卡片
#[tauri::command]
pub async fn list_custom_agents() -> Result<Vec<CustomAgentConfig>, String> {
    Ok(CustomAgentStore::list())
}

/// 保存（新增或更新）Custom Agent 卡片
///
/// 缓存纪律（与 set_leader_context 的「缓存第一」一致）：编辑激活卡片的 l2_prompt
/// 同 session 不生效——此处仅失效文件列表缓存（AGENTS_CACHE），不触碰
/// agent prompt 缓存，保证 provider 侧 prompt cache 前缀在整个 session 内稳定命中。
/// 新 L2 的生效时机：mode 切换 / 换卡（set_active_custom_agent 会 live 刷新）/ 新 session。
/// 注意与「换卡」语义不对称是有意为之：换卡是用户显式切换身份，立即生效；
/// 编辑是静默保存，不得中途改写正在运行的会话前缀。
#[tauri::command]
pub async fn save_custom_agent(config: CustomAgentConfig) -> Result<CustomAgentConfig, String> {
    CustomAgentStore::save(&config)
}

/// 按 id 删除 Custom Agent 卡片
#[tauri::command]
pub async fn delete_custom_agent(id: String) -> Result<(), String> {
    CustomAgentStore::delete_by_id(&id)
}

/// 获取当前激活的 Custom Agent 卡片
#[tauri::command]
pub async fn get_active_custom_agent() -> Result<Option<CustomAgentConfig>, String> {
    Ok(CustomAgentStore::get_active())
}

/// 设置激活卡片。若当前正处于 Custom 模式，立即刷新白名单/记忆归属/缓存使换卡生效。
#[tauri::command]
pub async fn set_active_custom_agent(
    state: State<'_, AppState>,
    id: String,
) -> Result<CustomAgentConfig, String> {
    let config = CustomAgentStore::set_active(&id)?;

    // 若当前已是 Custom 模式，换卡需立即生效（刷新白名单 + 记忆归属 + prompt 缓存）
    let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
    if let Some(ref mut runtime) = guard.leader_agent {
        if runtime.mode() == nuphus::runtime::Mode::Custom {
            runtime.apply_custom_card_state(Some(&config));
            runtime.agent_mut().invalidate_prompt_cache();
            tracing::info!(
                "[CUSTOM] Active card switched to '{}' (applied live)",
                config.name
            );
        }
    }
    Ok(config)
}
