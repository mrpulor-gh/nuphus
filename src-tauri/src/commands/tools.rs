// tools.rs — agent 工具调用命令（get_tools/execute_tool/get_desktop_status）
//
// 注意：本文件是 agent 工具列表的对外入口，内部机制命令（PDF/图片/视频
// 处理工具页）作为子模块挂载，绝不注册进 get_tools / execute_tool 的
// agent 工具列表——两者职责隔离：这里暴露给 LLM 编排，tools/* 由用户在
// 工具页手动调用。
pub mod doc;
pub mod image;
pub mod pdf;
pub mod video;
pub mod voice;

use crate::state::{AppState, DesktopStatus, HookScriptInfo, HooksConfigStatus, ToolSchema};
use tauri::State;

#[tauri::command]
pub fn get_tools(state: State<'_, AppState>) -> Result<Vec<ToolSchema>, String> {
    let schemas = state.tools.get_schemas();
    Ok(schemas
        .into_iter()
        .map(|s| ToolSchema {
            name: s.function.name,
            description: s
                .function
                .description
                .as_deref()
                .unwrap_or_default()
                .to_string(),
            input_schema: s.function.parameters,
            group: None,
        })
        .collect())
}

#[tauri::command]
pub async fn execute_tool(
    state: State<'_, AppState>,
    tool_name: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let perms = state
        .runtime
        .lock()
        .map_err(|e| e.to_string())?
        .tool_permissions;
    let policy = nuphus::PermissionPolicy::new(perms);
    let result = state
        .tools
        .execute_with_permission(&tool_name, &params, &policy)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "success": result.success,
        "output": result.output.unwrap_or_default(),
        "error": result.error.unwrap_or_default(),
    }))
}

#[tauri::command]
pub fn get_desktop_status(state: State<'_, AppState>) -> Result<DesktopStatus, String> {
    let has_desktop = state
        .tools
        .tool_names()
        .iter()
        .any(|n| n.starts_with("mouse_") || n == "screenshot" || n == "keyboard_type");
    let tools = state.tools.len();
    Ok(DesktopStatus {
        connected: has_desktop,
        tools_count: if has_desktop { tools - 7 } else { 0 },
    })
}

#[tauri::command]
pub fn get_hooks_status() -> Result<HooksConfigStatus, String> {
    let exe_dir = std::env::current_exe()
        .map(|p| {
            p.parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf()
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let hooks_dir = exe_dir
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("hooks");
    let yaml_path = hooks_dir.join("hooks.yaml");

    let read_script = |name: &str| -> Option<HookScriptInfo> {
        let script_path = hooks_dir.join(format!("{}.ps1", name));
        let exists = script_path.exists();
        let size = if exists {
            std::fs::metadata(&script_path)
                .map(|m| m.len())
                .unwrap_or(0)
        } else {
            0
        };
        Some(HookScriptInfo {
            path: script_path.display().to_string(),
            exists,
            size_bytes: size,
        })
    };

    Ok(HooksConfigStatus {
        pre_tool_call: read_script("pre_tool_call"),
        post_tool_call: read_script("post_tool_call"),
        on_session_start: read_script("session_start"),
        on_session_end: read_script("session_end"),
        config_path: yaml_path.display().to_string(),
    })
}
