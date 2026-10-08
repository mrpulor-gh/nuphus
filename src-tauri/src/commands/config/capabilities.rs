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
    let _config_write = nuphus::config::lock_provider_config();
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
    nuphus::config::write_provider_config(&config_path, &new_content)
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
/// `kind` ∈ { vision, stt, tts, voice, image_generation, video_generation }。
#[tauri::command]
pub fn set_capability_binding(
    kind: String,
    model: String,
    provider: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    const KINDS: [&str; 6] = [
        "vision",
        "stt",
        "tts",
        "voice",
        "image_generation",
        "video_generation",
    ];
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
    let rt = state.runtime.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        // 大窗口强制线（仅 RefineTier::Large 生效）；附带可调范围供前端渲染 slider
        "force_threshold": rt.large_force_refine_threshold,
        "force_min": nuphus::agent::distill::LARGE_FORCE_MIN,
        "force_max": nuphus::agent::distill::LARGE_FORCE_MAX,
    }))
}

/// Set the large-window force-refine ratio.
///
/// Only meaningful for `RefineTier::Large` (context_window > 600K); Small/Medium
/// tiers use their fixed ratios and ignore this value. Rejects out-of-range input
/// rather than silently clamping — a bad value here means the caller's UI and the
/// backend disagree, which is worth surfacing instead of hiding.
#[tauri::command]
pub fn set_session_refine_config(
    state: State<'_, AppState>,
    force_threshold: Option<f64>,
) -> Result<String, String> {
    if let Some(th) = force_threshold {
        let min = nuphus::agent::distill::LARGE_FORCE_MIN;
        let max = nuphus::agent::distill::LARGE_FORCE_MAX;
        if !(min..=max).contains(&th) {
            return Err(format!("force_threshold must be between {} ~ {}", min, max));
        }
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.large_force_refine_threshold = th;
        tracing::info!("set_session_refine_config: force_threshold={}", th);
        // 落盘：内存 guard 随进程消失，不持久化等于「用户调了但下次启动悄悄退回
        // 默认 50%」。先落盘再返回——写失败要让调用方看到（滑杆会弹错误提示），
        // 否则又变成「显示已生效、实际没保存」。
        crate::commands::config::toml_ops::write_refine_force_threshold_to_config_toml(th)?;
    }
    Ok("Refinement config updated".to_string())
}
