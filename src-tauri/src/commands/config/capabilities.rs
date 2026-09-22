//! Capabilities, refinement, and tool permission commands.

use super::toml_ops::get_config_path;
use crate::state::AppState;
use tauri::State;
#[tauri::command]
pub fn get_tool_permissions(state: State<'_, AppState>) -> Result<String, String> {
    let perms = state.runtime.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::to_string(&perms.tool_permissions).unwrap_or_default())
}

#[tauri::command]
pub fn set_capability(
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Use providers.toml as canonical location (same as get_capabilities)
    let config_path = {
        let providers_path = state.llm_config_path.with_file_name("providers.toml");
        if providers_path.exists() {
            providers_path
        } else {
            get_config_path().ok_or_else(|| "Unable to locate config file".to_string())?
        }
    };

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config.toml: {}", e))?;

    let mut doc: toml::Value = content
        .parse()
        .unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()));

    // Get or create [capabilities] table
    if doc.get("capabilities").is_none() {
        let table = doc
            .as_table_mut()
            .ok_or_else(|| "config.toml root is not a table".to_string())?;
        table.insert(
            "capabilities".to_string(),
            toml::Value::Table(toml::value::Table::new()),
        );
    }
    let caps = doc
        .get_mut("capabilities")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| "Cannot create [capabilities] table".to_string())?;

    caps.insert(
        key.clone(),
        if let Ok(n) = value.parse::<i64>() {
            toml::Value::Integer(n)
        } else {
            toml::Value::String(value)
        },
    );

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(&config_path, new_content)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;

    Ok(())
}

/// 原子设置能力模型绑定：`{kind}` 与 `{kind}_provider` 必须一起落盘。
///
/// 前端原先分两次调用 `set_capability(kind, …)` / `set_capability(kind_provider, …)`；
/// 第二次失败就会留下「新 model + 旧 provider」的中间态 —— 后端按
/// provider + model 精确解析时找不到该组合，能力请求直接失败，而 UI 已经提示成功。
/// 这里收敛为一次读写：要么两个字段都更新，要么都不动。
///
/// `kind` ∈ { vision, stt, tts, voice, image_generation }。
#[tauri::command]
pub fn set_capability_binding(
    kind: String,
    model: String,
    provider: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    const KINDS: [&str; 5] = ["vision", "stt", "tts", "voice", "image_generation"];
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("未知能力类型: {kind}"));
    }

    let config_path = {
        let providers_path = state.llm_config_path.with_file_name("providers.toml");
        if providers_path.exists() {
            providers_path
        } else {
            get_config_path().ok_or_else(|| "Unable to locate config file".to_string())?
        }
    };

    super::toml_ops::set_capability_in_config_toml(&config_path, &kind, &model, &provider)?;

    tracing::info!(
        "[set_capability_binding] kind={} model={} provider={}",
        kind,
        model,
        provider
    );
    Ok(())
}

#[tauri::command]
pub fn get_session_refine_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let threshold = state.runtime.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "threshold": threshold.refine_threshold,
    }))
}

/// Set refinement config
#[tauri::command]
pub fn set_session_refine_config(
    state: State<'_, AppState>,
    threshold: Option<f64>,
) -> Result<String, String> {
    if let Some(th) = threshold {
        if !(0.0..=1.0).contains(&th) {
            return Err("threshold must be between 0.0 ~ 1.0".to_string());
        }
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.refine_threshold = th;
        tracing::info!("set_session_refine_config: threshold={}", th);
    }
    Ok("Refinement config updated".to_string())
}
