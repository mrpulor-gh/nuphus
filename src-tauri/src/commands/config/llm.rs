//! LLM configuration commands.
//!
//! Contains all tauri::command handlers related to LLM provider/model
//! configuration plus the LLM-specific helpers used only by `configure_llm`
//! (context-window probing, vision probing, etc.).

use super::toml_ops::{
    clear_provider_api_key_in_config_toml, get_config_path, list_configured_providers,
    read_provider_api_key_from_config_toml, read_provider_reasoning_effort_from_config_toml,
    update_config_toml, update_model_context_window, update_model_reasoning_efforts,
    update_model_supports_vision, update_reasoning_effort, upsert_provider_models,
};
use crate::emitter::CompoundEmitter;
use crate::state::{AppState, LlamaConfig};
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use nuphus::config::registry::ProviderRegistry;
use tauri::State;
/// Load provider/model selection from providers.toml (TOML) at app startup.
/// Reads top-level `model` field (system default), falls back to first provider with an api_key.
pub fn load_llm_config_from_disk(state: &crate::state::AppState) {
    let config_path = state.llm_config_path.clone();
    if !config_path.exists() {
        return;
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[STARTUP] Failed to read providers.toml: {}", e);
            return;
        }
    };
    let doc: toml::Value = match toml::from_str(&content) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("[STARTUP] Failed to parse providers.toml: {}", e);
            return;
        }
    };

    // 1. 读顶部 model 字段（系统默认模型），定位其 provider（拿 api_key/base_url）。
    //    已废弃 [last_used]：模型选择由 agent_models + 顶部 model 字段承载，无跨启动记忆。
    let top_model = doc.get("model").and_then(|v| v.as_str()).unwrap_or("");

    let find_by_model = |model: &str| -> Option<(String, String, String, String)> {
        if model.is_empty() {
            return None;
        }
        let providers = doc.get("providers").and_then(|p| p.as_array())?;
        for entry in providers {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let k_raw = entry.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
            let k = nuphus::cookies::decrypt_secret(k_raw).unwrap_or_default();
            let u = entry.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
            let has_model = entry
                .get("models")
                .and_then(|arr| arr.as_array())
                .map(|ms| {
                    ms.iter()
                        .any(|m| m.get("id").and_then(|i| i.as_str()) == Some(model))
                })
                .unwrap_or(false);
            if has_model && !k.is_empty() {
                return Some((
                    name.to_string(),
                    model.to_string(),
                    u.to_string(),
                    k.to_string(),
                ));
            }
        }
        None
    };

    let (provider_name, model_id, base_url, api_key) = match find_by_model(top_model) {
        Some(v) => v,
        None => {
            // 2. Fallback: first provider with a non-empty api_key
            let providers = match doc.get("providers").and_then(|p| p.as_array()) {
                Some(arr) => arr,
                None => return,
            };
            let mut p = String::new();
            let mut m = String::new();
            let mut bu = String::new();
            let mut key = String::new();
            for entry in providers {
                let name = entry
                    .get("provider_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or(entry.get("name").and_then(|v| v.as_str()).unwrap_or(""));
                let k_raw = entry.get("api_key").and_then(|v| v.as_str()).unwrap_or("");
                let k = nuphus::cookies::decrypt_secret(k_raw).unwrap_or_default();
                let u = entry.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
                if !k.is_empty() && !name.is_empty() {
                    let model = entry
                        .get("models")
                        .and_then(|arr| arr.as_array())
                        .and_then(|models| models.first())
                        .and_then(|m| m.get("id").and_then(|id| id.as_str()))
                        .unwrap_or("");
                    if !model.is_empty() {
                        p = name.to_string();
                        m = model.to_string();
                        bu = u.to_string();
                        key = k.to_string();
                        break;
                    }
                }
            }
            if p.is_empty() || m.is_empty() {
                return;
            }
            (p, m, bu, key)
        }
    };

    if api_key.is_empty() || model_id.is_empty() {
        return;
    }

    let cfg = LlamaConfig {
        api_key,
        model: model_id.clone(),
        provider: provider_name.clone(),
        base_url,
        parameters: None,
        // Preserve any reasoning-effort configured in config.toml for this provider.
        reasoning_effort: read_provider_reasoning_effort_from_config_toml(&provider_name),
    };

    if let Ok(mut guard) = state.runtime.lock() {
        guard.llm_config = Some(cfg.clone());
        guard.model_context_window = nuphus::agent::goal_types::get_context_window(&model_id);
    }

    tracing::info!(
        "[STARTUP] Loaded LLM config from providers.toml: provider={}, model={}",
        provider_name,
        model_id
    );
}

/// (废弃) 模型选择已由 agent_models + 顶部 model 字段承载，此函数保留仅供兼容。
#[allow(dead_code)]
fn update_last_used(
    providers_path: &std::path::Path,
    provider: &str,
    model: &str,
    base_url: &str,
) -> Result<(), String> {
    let content = std::fs::read_to_string(providers_path).unwrap_or_default();
    let mut doc: toml::Value = content.parse().unwrap_or_else(|_| {
        let table = toml::value::Table::new();
        toml::Value::Table(table)
    });

    let table = doc
        .as_table_mut()
        .ok_or_else(|| "providers.toml is not a table".to_string())?;

    let mut last_used = toml::value::Table::new();
    last_used.insert(
        "provider".to_string(),
        toml::Value::String(provider.to_string()),
    );
    last_used.insert("model".to_string(), toml::Value::String(model.to_string()));
    last_used.insert(
        "base_url".to_string(),
        toml::Value::String(base_url.to_string()),
    );
    table.insert("last_used".to_string(), toml::Value::Table(last_used));

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content = toml::to_string_pretty(&doc)
        .map_err(|e| format!("serialize providers.toml failed: {}", e))?;
    std::fs::write(providers_path, new_content)
        .map_err(|e| format!("write providers.toml failed: {}", e))?;

    tracing::info!(
        "[last_used] updated: provider={}, model={}",
        provider,
        model
    );
    Ok(())
}

// ════════════════════════════════════════════════════════════════════
// Agent 级模型配置（高级设置）：leader / workflow / exec / custom 各自模型，
// 空 = 跟随默认模型（default），default 空 = 跟随 leader（锚点）。
// 持久化于 providers.toml `[agent_models]` section。
// ════════════════════════════════════════════════════════════════════

/// Agent 级模型配置（空字符串 = 未设置 → 跟随 default → 跟随 leader）
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AgentModels {
    pub default: String,
    pub leader: String,
    pub workflow: String,
    pub exec: String,
    pub custom: String,
}

impl AgentModels {
    pub const AGENTS: [&'static str; 5] = ["default", "leader", "workflow", "exec", "custom"];

    pub fn set(&mut self, agent: &str, model: String) {
        match agent {
            "leader" => self.leader = model,
            "workflow" => self.workflow = model,
            "exec" => self.exec = model,
            "custom" => self.custom = model,
            _ => self.default = model,
        }
    }
}

/// Read `[agent_models]` from providers.toml (empty when absent / parse failure).
pub fn load_agent_models(providers_path: &std::path::Path) -> AgentModels {
    let mut out = AgentModels::default();
    let Ok(content) = std::fs::read_to_string(providers_path) else {
        return out;
    };
    let Ok(doc) = content.parse::<toml::Value>() else {
        return out;
    };
    let Some(section) = doc.get("agent_models").and_then(|v| v.as_table()) else {
        return out;
    };
    for agent in AgentModels::AGENTS {
        if let Some(v) = section.get(agent).and_then(|v| v.as_str()) {
            out.set(agent, v.to_string());
        }
    }
    // 兼容旧字段：`global`（旧版全局默认）→ `default`
    if out.default.is_empty() {
        if let Some(v) = section.get("global").and_then(|v| v.as_str()) {
            out.default = v.to_string();
        }
    }
    out
}

/// Write one `[agent_models]` entry to providers.toml.
fn save_agent_model(
    providers_path: &std::path::Path,
    agent: &str,
    model: &str,
) -> Result<(), String> {
    if !AgentModels::AGENTS.contains(&agent) {
        return Err(format!("未知 agent: {agent}"));
    }
    let content = std::fs::read_to_string(providers_path).unwrap_or_default();
    let mut doc: toml::Value = content
        .parse()
        .unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()));
    let table = doc
        .as_table_mut()
        .ok_or_else(|| "providers.toml is not a table".to_string())?;
    let section = table
        .entry("agent_models")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "agent_models is not a table".to_string())?;
    section.insert(agent.to_string(), toml::Value::String(model.to_string()));

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content = toml::to_string_pretty(&doc)
        .map_err(|e| format!("serialize providers.toml failed: {e}"))?;
    std::fs::write(providers_path, new_content)
        .map_err(|e| format!("write providers.toml failed: {e}"))?;
    tracing::info!("[agent_models] {agent} = {model}");
    Ok(())
}

/// 单一模型解析入口：计算某 agent 的生效模型（唯一解析点，process/retry 共用）。
///
/// 解析链（「可用」= 非空且 `registry.find_model` 命中）：
///   leader_eff   = leader 可用 ? leader : registry.model（锚点，providers.toml 顶部 model 字段）
///   default_eff  = default 可用 ? default : leader_eff
///   workflow/custom/exec_eff = 各自可用 ? 各自 : default_eff
/// `mode` 为 "leader" 或未知 → leader_eff。
pub fn effective_model(
    providers_path: &std::path::Path,
    registry: &nuphus::config::ModelRegistry,
    mode: &str,
) -> String {
    let am = load_agent_models(providers_path);
    let avail = |m: &str| !m.is_empty() && registry.find_model(m).is_some();

    let leader = if avail(&am.leader) {
        am.leader.clone()
    } else {
        registry.model.clone()
    };
    let default = if avail(&am.default) {
        am.default.clone()
    } else {
        leader.clone()
    };

    match mode {
        "workflow" => {
            if avail(&am.workflow) {
                am.workflow.clone()
            } else {
                default
            }
        }
        "custom" => {
            if avail(&am.custom) {
                am.custom.clone()
            } else {
                default
            }
        }
        "exec" => {
            if avail(&am.exec) {
                am.exec.clone()
            } else {
                default
            }
        }
        _ => leader,
    }
}

/// Get current agent-level model configuration (advanced settings).
#[tauri::command]
pub fn get_agent_models(state: State<'_, AppState>) -> Result<AgentModels, String> {
    Ok(load_agent_models(&state.llm_config_path))
}

/// 计算某 mode 的生效模型（前端输入框显示用）。
#[tauri::command]
pub fn get_effective_model(state: State<'_, AppState>, mode: String) -> Result<String, String> {
    let registry = nuphus::config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
    Ok(effective_model(&state.llm_config_path, &registry, &mode))
}

/// Set one agent's model. `model` empty string = clear (follow default fallback).
#[tauri::command]
pub fn set_agent_model(
    state: State<'_, AppState>,
    agent: String,
    model: String,
) -> Result<String, String> {
    save_agent_model(&state.llm_config_path, &agent, &model)?;
    Ok(format!(
        "{agent} 模型已设置为 {}",
        if model.is_empty() {
            "跟随默认模型"
        } else {
            &model
        }
    ))
}

/// Switch active model (provider-driven: reads target provider's API key from config.toml,
/// never takes a key from the frontend). For initial setup / key changes, use configure_llm.
///
/// 泛型核心：桌面 IPC（具体 Wry thin wrapper `switch_model`）与手机端
/// mobile_server（泛型 Runtime）共用同一实现；`#[tauri::command]` 由下方
/// thin wrapper 提供（避免宏生成函数重复）。
pub async fn switch_model_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    model: String,
    provider: String,
    base_url: Option<String>,
    context_window: Option<usize>,
    mode: Option<String>,
) -> Result<String, String> {
    // Read the target provider's stored API key from config.toml
    let api_key = read_provider_api_key_from_config_toml(&provider).ok_or_else(|| {
        format!(
            "Provider '{}' 尚未配置 API Key，请先在模型配置页面输入密钥",
            provider
        )
    })?;

    // Resolve base_url from provider metadata
    let registry = ProviderRegistry::builtin();
    let pmeta = registry.get(&provider);
    let resolved_base_url = base_url.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        pmeta
            .as_ref()
            .map(|p| p.default_base_url().to_string())
            .unwrap_or_default()
    });

    let resolved_model = model.clone();
    let resolved_provider = provider.clone();

    tracing::info!(
        "switch_model: provider={}, model={}, base_url={}",
        resolved_provider,
        resolved_model,
        resolved_base_url
    );

    // Check if model actually changed
    let prev_model = state
        .runtime
        .lock()
        .ok()
        .and_then(|g| g.llm_config.as_ref().map(|c| c.model.clone()));

    // Carry the reasoning-effort configured for this provider (config.toml
    // [[providers]] reasoning_effort) into runtime so the next client build
    // (from_single → transport) picks it up.
    let reasoning_effort = read_provider_reasoning_effort_from_config_toml(&resolved_provider);

    // Store config in runtime state
    let cfg = LlamaConfig {
        api_key: api_key.clone(),
        model: resolved_model.clone(),
        provider: resolved_provider.clone(),
        base_url: resolved_base_url.clone(),
        parameters: None,
        reasoning_effort,
    };
    {
        let mut config = state.runtime.lock().map_err(|e| e.to_string())?;
        config.llm_config = Some(cfg.clone());
    }

    // Agent 级模型：前端按当前 mode 切换 → 落盘写对应 agent（leader/workflow/custom）。
    // mode 缺省/未知 → 默认写 leader（锚点）。default/exec 由高级设置页配置。
    let agent_key = mode
        .as_deref()
        .filter(|m| AgentModels::AGENTS.contains(m))
        .unwrap_or("leader");
    let _ = save_agent_model(&state.llm_config_path, agent_key, &resolved_model);

    // Push notification if model changed
    if let Some(prev) = prev_model {
        if prev != resolved_model {
            let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
            if let Some(agent) = guard.leader_agent.as_mut() {
                agent
                    .session_mut()
                    .push_system(format!("当前模型已切换至 {}", resolved_model));
            }
        }
    }

    // 广播模型变更：双推桌面 Tauri + 手机 WS（mobile_server 未启动时 CompoundEmitter
    // 退化为纯 Tauri 推送，桌面端零回归）。
    // 后端是模型选择的唯一权威源：桌面 switch_model 与手机 /switch-model 共用此命令，
    // 切换后双端（手机自身 + 桌面端实时一致）同步「当前模型」。
    let emitter = CompoundEmitter::new(app, &state);
    emitter.emit(NuphusEvent::SessionInfo {
        session_id: uuid::Uuid::new_v4().to_string(),
        model: resolved_model.clone(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });

    // Post-config: context window, vision probe
    post_configure(
        &state,
        &resolved_provider,
        &resolved_model,
        &api_key,
        &resolved_base_url,
        context_window,
    )
    .await;

    Ok(format!(
        "Switched to: provider={}, model={}",
        resolved_provider, resolved_model
    ))
}

/// 桌面 IPC 命令入口（thin wrapper）：委托泛型核心 `switch_model_impl`。
/// 桌面端切换模型时由前端 invoke；事件双推（桌面 Tauri + 手机 WS）。
#[tauri::command]
pub async fn switch_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    model: String,
    provider: String,
    base_url: Option<String>,
    context_window: Option<usize>,
    mode: Option<String>,
) -> Result<String, String> {
    switch_model_impl(app, state, model, provider, base_url, context_window, mode).await
}

/// Configure LLM: set API key, model, provider. Persists to key file (plaintext) + config.toml.
/// For model switching, frontend passes the existing API key from its config state.
#[tauri::command]
pub async fn configure_llm(
    state: State<'_, AppState>,
    api_key: String,
    model: Option<String>,
    provider: Option<String>,
    base_url: Option<String>,
    context_window: Option<usize>,
) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }

    let resolved_provider = provider
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "minimax".to_string());

    // Check if this is a model switch (re-config vs initial setup)
    let prev_model = state
        .runtime
        .lock()
        .ok()
        .and_then(|g| g.llm_config.as_ref().map(|c| c.model.clone()));

    let registry = ProviderRegistry::builtin();
    let provider = registry.get(&resolved_provider);
    let default_model = provider
        .as_ref()
        .map(|p| p.default_model())
        .unwrap_or("MiniMax-M2.7");
    let resolved_model = model
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_model.to_string());
    let resolved_base_url = base_url.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        provider
            .as_ref()
            .map(|p| p.default_base_url().to_string())
            .unwrap_or_default()
    });

    tracing::info!(
        "configure_llm: provider={}, model={}, base_url={}",
        resolved_provider,
        resolved_model,
        resolved_base_url
    );

    let toml_config_path: Option<std::path::PathBuf> = get_config_path().or_else(|| {
        let fallback = state.llm_config_path.with_file_name("providers.toml");
        if let Some(parent) = fallback.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Some(fallback)
    });

    // Store config in runtime state
    let cfg = LlamaConfig {
        api_key: api_key.clone(),
        model: resolved_model.clone(),
        provider: resolved_provider.clone(),
        base_url: resolved_base_url.clone(),
        parameters: None,
        // Preserve any reasoning-effort already configured for this provider.
        reasoning_effort: read_provider_reasoning_effort_from_config_toml(&resolved_provider),
    };
    {
        let mut config = state.runtime.lock().map_err(|e| e.to_string())?;
        config.llm_config = Some(cfg.clone());
    }

    // Write API key to config.toml
    if let Some(ref config_path) = toml_config_path {
        if let Err(e) = update_config_toml(
            config_path,
            &resolved_provider,
            &cfg.api_key,
            &resolved_model,
            Some(&resolved_base_url),
        ) {
            tracing::error!("[configure_llm] Failed to update config.toml: {}", e);
            return Err(format!("保存 API Key 到配置文件失败: {}", e));
        }
    }

    // Push system notification if model actually changed (re-config, not initial setup)
    if let Some(prev) = prev_model {
        if prev != resolved_model || resolved_provider != cfg.provider {
            let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
            if let Some(agent) = guard.leader_agent.as_mut() {
                agent
                    .session_mut()
                    .push_system(format!("当前模型已切换至 {}", resolved_model));
            }
        }
    }

    // Post-config: context window, vision probe
    post_configure(
        &state,
        &resolved_provider,
        &resolved_model,
        &api_key,
        &resolved_base_url,
        context_window,
    )
    .await;

    Ok(format!(
        "LLM configured: provider={}, model={}",
        resolved_provider, resolved_model
    ))
}

/// Clear a provider's API key from config.toml.
///
/// Preserves the provider entry (name / base_url / models) — only `api_key`
/// is emptied. Idempotent: unknown providers return `Ok(())` without changes.
#[tauri::command]
pub fn clear_provider_api_key(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), String> {
    let toml_config_path = get_config_path().or_else(|| {
        let fallback = state.llm_config_path.with_file_name("providers.toml");
        if let Some(parent) = fallback.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Some(fallback)
    });

    if let Some(ref config_path) = toml_config_path {
        if let Err(e) = clear_provider_api_key_in_config_toml(config_path, &provider) {
            tracing::error!("[clear_provider_api_key] Failed to clear config.toml: {}", e);
            return Err(format!("清除 API Key 失败: {}", e));
        }
    }
    Ok(())
}

/// Post-config steps: query context window from API + probe vision support.
async fn post_configure(
    state: &State<'_, AppState>,
    resolved_provider: &str,
    resolved_model: &str,
    api_key: &str,
    resolved_base_url: &str,
    context_window: Option<usize>,
) {
    let (auth_header, auth_prefix) = ProviderRegistry::builtin()
        .get(resolved_provider)
        .map(|p| (p.auth_header(), p.auth_prefix()))
        .unwrap_or(("authorization", "Bearer "));

    let toml_config_path =
        get_config_path().unwrap_or_else(|| state.llm_config_path.with_file_name("providers.toml"));

    // Context window + reasoning-effort capability (single /models fetch)
    let ctx = if resolved_provider == "local" {
        context_window.unwrap_or(128_000)
    } else {
        let base_url = resolved_base_url.to_string();
        let model = resolved_model.to_string();
        let key = api_key.to_string();
        let hdr = auth_header.to_string();
        let prefix = auth_prefix.to_string();
        match tokio::task::spawn_blocking(move || {
            query_model_metadata_from_api(&base_url, &model, &key, &hdr, &prefix)
        })
        .await
        {
            Ok(Some(meta)) => {
                // Persist discovered effort capability into the model entry so
                // list_models can serve it without a builtin-registry hit.
                if !meta.reasoning_efforts.is_empty() {
                    let _ = update_model_reasoning_efforts(
                        &toml_config_path,
                        resolved_provider,
                        resolved_model,
                        &meta.reasoning_efforts,
                        meta.default_effort.as_deref(),
                    );
                }
                meta.context_length.unwrap_or_else(|| {
                    nuphus::agent::goal_types::get_context_window(resolved_model)
                })
            }
            _ => nuphus::agent::goal_types::get_context_window(resolved_model),
        }
    };
    {
        let mut cw = state.runtime.lock().ok();
        if let Some(ref mut cw) = cw {
            cw.model_context_window = ctx;
        }
    }

    let _ = update_model_context_window(&toml_config_path, resolved_provider, resolved_model, ctx);

    // Vision probe — provider-driven: prefer metadata from ProviderRegistry over HTTP probing.
    // Only HTTP-probe when metadata doesn't have a definitive answer (e.g. custom / new models).
    let metadata_vision = ProviderRegistry::builtin()
        .get(resolved_provider)
        .and_then(|p| p.models().iter().find(|m| m.id == resolved_model))
        .map(|m| m.supports_vision);

    match metadata_vision {
        Some(true) => {
            tracing::info!(
                "[vision-probe] metadata: model={} supports vision ✓",
                resolved_model
            );
            let _ = update_model_supports_vision(
                &toml_config_path,
                resolved_provider,
                resolved_model,
                true,
            );
        }
        Some(false) => {
            tracing::info!(
                "[vision-probe] metadata: model={} does NOT support vision",
                resolved_model
            );
            let _ = update_model_supports_vision(
                &toml_config_path,
                resolved_provider,
                resolved_model,
                false,
            );
        }
        None if resolved_provider != "local" => {
            // No metadata — fall back to HTTP probe for custom models
            let base_url = resolved_base_url.to_string();
            let model = resolved_model.to_string();
            let key = api_key.to_string();
            let hdr = auth_header.to_string();
            let prefix = auth_prefix.to_string();
            if let Ok(Some(supports_vision)) = tokio::task::spawn_blocking(move || {
                probe_vision(&base_url, &model, &key, &hdr, &prefix)
            })
            .await
            {
                let _ = update_model_supports_vision(
                    &toml_config_path,
                    resolved_provider,
                    resolved_model,
                    supports_vision,
                );
            }
        }
        _ => {} // local or unknown provider — skip
    }
}

#[tauri::command]
pub fn get_current_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let configured_providers = list_configured_providers();

    // 优先从 config.toml 读取
    if let Some(config_path) = get_config_path() {
        use nuphus::config::ModelRegistry;
        if let Ok(registry) =
            ModelRegistry::from_toml(config_path.to_str().unwrap_or("config.toml"))
        {
            let current_model = registry.model.clone();
            for provider in &registry.providers {
                for model in &provider.models {
                    if model.id == current_model {
                        return Ok(serde_json::json!({
                            "api_key": &provider.api_key,
                            "has_key": !provider.api_key.is_empty(),
                            "model": current_model,
                            "provider": provider.name,
                            "base_url": provider.base_url,
                            "configured_providers": configured_providers,
                        }));
                    }
                }
            }
            return Ok(serde_json::json!({
                "api_key": "",
                "has_key": false,
                "model": current_model,
                "provider": "",
                "base_url": "",
                "configured_providers": configured_providers,
            }));
        }
    }

    // 后备：从内存读取（首次启动时 config.toml 可能还未创建）
    let guard = state.runtime.lock().map_err(|e| e.to_string())?;
    match guard.llm_config.as_ref() {
        Some(cfg) => Ok(serde_json::json!({
            "api_key": &cfg.api_key,
            "has_key": !cfg.api_key.is_empty(),
            "model": cfg.model,
            "provider": cfg.provider,
            "base_url": cfg.base_url,
            "configured_providers": configured_providers,
        })),
        None => Ok(serde_json::json!(null)),
    }
}

#[tauri::command]
pub fn is_llm_configured(state: State<'_, AppState>) -> Result<bool, String> {
    let guard = state.runtime.lock().map_err(|e| e.to_string())?;

    // ── Eager-load from providers.toml on first call ──
    if guard.llm_config.is_none() {
        let path = state.llm_config_path.clone();
        drop(guard); // release lock before file I/O

        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<toml::Value>(&content) {
                    Ok(doc) => {
                        if let Some(providers) = doc.get("providers").and_then(|p| p.as_array()) {
                            for entry in providers {
                                let name = entry.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                let provider_type = entry
                                    .get("provider_type")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or(name);
                                let api_key =
                                    entry.get("api_key").and_then(|k| k.as_str()).unwrap_or("");
                                let base_url =
                                    entry.get("base_url").and_then(|u| u.as_str()).unwrap_or("");
                                if !api_key.is_empty() && !name.is_empty() {
                                    let model_id = entry
                                        .get("models")
                                        .and_then(|m| m.as_array())
                                        .and_then(|models| models.first())
                                        .and_then(|m| m.get("id"))
                                        .and_then(|id| id.as_str())
                                        .unwrap_or("");
                                    if !model_id.is_empty() {
                                        let mut guard =
                                            state.runtime.lock().map_err(|e| e.to_string())?;
                                        guard.llm_config = Some(LlamaConfig {
                                            api_key: api_key.to_string(),
                                            model: model_id.to_string(),
                                            provider: provider_type.to_string(),
                                            base_url: base_url.to_string(),
                                            parameters: None,
                                            reasoning_effort: None,
                                        });
                                        guard.model_context_window =
                                            nuphus::agent::goal_types::get_context_window(model_id);
                                        tracing::info!("[is_llm_configured] Loaded from providers.toml: provider={}, model={}, context_window={}",
                                                provider_type, model_id, guard.model_context_window);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[is_llm_configured] Failed to parse providers.toml: {}", e);
                    }
                },
                Err(e) => {
                    tracing::warn!("[is_llm_configured] Failed to read providers.toml: {}", e);
                }
            }
        }

        // Re-acquire lock
        let guard = state.runtime.lock().map_err(|e| e.to_string())?;
        match guard.llm_config.as_ref() {
            Some(cfg) => Ok(!cfg.api_key.is_empty() && cfg.api_key.len() >= 10),
            None => Ok(false),
        }
    } else {
        match guard.llm_config.as_ref() {
            Some(cfg) => Ok(!cfg.api_key.is_empty() && cfg.api_key.len() >= 10),
            None => Ok(false),
        }
    }
}

#[tauri::command]
pub fn list_models(_state: State<'_, AppState>) -> Result<Vec<nuphus::api::ModelInfo>, String> {
    use nuphus::config::ModelRegistry;

    let registry = if let Some(path) = get_config_path() {
        tracing::info!("loading config from: {}", path.display());
        ModelRegistry::from_toml(path.to_str().unwrap_or("config.toml"))
            .map_err(|e| format!("load config failed: {}", e))?
    } else {
        tracing::info!("no config file found, falling back to environment variables");
        ModelRegistry::from_env().map_err(|e| format!("load from env failed: {}", e))?
    };

    let mut models = Vec::new();
    let builtin = nuphus::config::registry::ProviderRegistry::builtin();
    for provider in &registry.providers {
        for model in &provider.models {
            // Reasoning-effort options: prefer per-model metadata persisted at
            // configure time (discovered from the provider's /models response,
            // e.g. Kimi think_efforts); fall back to the built-in ModelDef
            // (e.g. deepseek-v4-flash = [high, max]) for providers whose
            // /models returns bare id lists. Unknown models → no effort knob.
            let reasoning_efforts = if !model.reasoning_efforts.is_empty() {
                model.reasoning_efforts.clone()
            } else {
                builtin
                    .find_model(&model.id)
                    .map(|(_, m)| m.reasoning_efforts.iter().map(|s| s.to_string()).collect())
                    .unwrap_or_default()
            };
            let default_effort = model.default_effort.clone().or_else(|| {
                builtin
                    .find_model(&model.id)
                    .and_then(|(_, m)| m.default_effort.map(|s| s.to_string()))
            });
            models.push(nuphus::api::ModelInfo {
                id: model.id.clone(),
                provider: provider.name.clone(),
                alias: model.alias.clone(),
                supports_streaming: model.supports_streaming,
                supports_vision: model.supports_vision,
                supports_audio: model.supports_audio,
                supports_image_generation: model.supports_image_generation,
                reasoning_efforts,
                default_effort,
            });
        }
    }

    Ok(models)
}

#[tauri::command]
pub fn get_default_model(_state: State<'_, AppState>) -> Result<String, String> {
    use nuphus::config::ModelRegistry;

    let registry = if let Some(path) = get_config_path() {
        ModelRegistry::from_toml(path.to_str().unwrap_or("config.toml"))
            .map_err(|e| format!("load config failed: {}", e))?
    } else {
        ModelRegistry::from_env().map_err(|e| format!("load from env failed: {}", e))?
    };

    Ok(registry.model.clone())
}

/// Read the reasoning-effort value configured for a provider
/// (config.toml `[[providers]] reasoning_effort`). `null` = not configured /
/// provider default (transport sends no `reasoning_effort`).
#[tauri::command]
pub fn get_reasoning_effort(
    _state: State<'_, AppState>,
    provider: String,
) -> Result<Option<String>, String> {
    Ok(read_provider_reasoning_effort_from_config_toml(&provider))
}

/// Persist a reasoning-effort value for a provider into config.toml and update
/// the in-memory LlamaConfig so the next client build picks it up.
/// `effort: null` or empty clears the setting (provider default).
#[tauri::command]
pub fn set_reasoning_effort(
    state: State<'_, AppState>,
    provider: String,
    effort: Option<String>,
) -> Result<String, String> {
    let path = get_config_path().ok_or_else(|| "config.toml 未找到".to_string())?;
    update_reasoning_effort(&path, &provider, effort.as_deref())?;
    {
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        if let Some(cfg) = guard.llm_config.as_mut() {
            if cfg.provider == provider {
                cfg.reasoning_effort = effort.clone();
            }
        }
    }
    match effort {
        Some(e) if !e.is_empty() => Ok(format!("reasoning_effort set to {} for {}", e, provider)),
        _ => Ok(format!("reasoning_effort cleared for {}", provider)),
    }
}

#[tauri::command]
pub async fn test_llm_connection(
    _state: State<'_, AppState>,
    api_key: String,
    model: String,
    provider: String,
    base_url: String,
) -> Result<String, String> {
    // ProviderKind is now the canonical type (merged from KnownProvider).
    // 函数体内不需要直接命名该类型。
    use nuphus::api::MessageRequest;
    use nuphus::llm::LlmClient;
    use std::time::Duration;

    tracing::info!(
        "test_llm_connection: provider={}, model={}, base_url={}",
        provider,
        model,
        base_url
    );

    // 1. Get provider metadata and defaults from ProviderRegistry
    let registry = ProviderRegistry::builtin();
    let provider_meta = registry.get(&provider);
    let resolved_base_url = if base_url.is_empty() {
        provider_meta
            .as_ref()
            .map(|p| p.default_base_url().to_string())
            .unwrap_or_default()
    } else {
        base_url
    };
    let auth_header = provider_meta
        .as_ref()
        .map(|p| p.auth_header().to_string())
        .unwrap_or_else(|| "authorization".to_string());
    let auth_prefix = provider_meta
        .as_ref()
        .map(|p| p.auth_prefix().to_string())
        .unwrap_or_else(|| "Bearer ".to_string());

    // 2. Create client — unified provider-driven path for all Providers.
    //    Every Provider's transport() method selects the correct Transport
    //    (ChatCompletions or Anthropic) with quirks embedded. No per-Provider
    //    branching needed.
    let client = match &provider_meta {
        Some(pmeta) => {
            let provider_cfg = nuphus::config::ProviderConfig {
                name: provider.clone(),
                provider_type: nuphus::config::KnownProvider::from_id(&provider)
                    .unwrap_or(nuphus::config::KnownProvider::Custom),
                api_key: api_key.clone(),
                base_url: resolved_base_url.clone(),
                auth_header: auth_header.clone(),
                auth_prefix: auth_prefix.clone(),
                timeout_secs: 15,
                models: vec![],
                reasoning_effort: None,
            };
            LlmClient::with_transport_arc(pmeta.transport(&provider_cfg, &model))
        }
        None => return Ok(format!("error: unknown provider '{}'", provider)),
    };

    // 3. Build test message
    let request = MessageRequest::new(
        model.clone(),
        vec![serde_json::json!({
            "role": "user",
            "content": "Hello, respond with just 'ok'."
        })],
    )
    .with_max_tokens(10)
    .with_stream(true);

    // 4. Send request (with 15s timeout)
    let result = tokio::time::timeout(Duration::from_secs(15), client.stream_async(request)).await;

    match result {
        Ok(Ok(events)) => {
            // Collect response text
            let mut response_text = String::new();
            for event in events {
                match event {
                    nuphus::api::AssistantEvent::TextDelta(text) => {
                        response_text.push_str(&text);
                    }
                    nuphus::api::AssistantEvent::MessageStop => break,
                    _ => {}
                }
            }

            let preview = if response_text.len() > 40 {
                format!("{}...", response_text.chars().take(40).collect::<String>())
            } else {
                response_text
            };

            tracing::info!(
                "test_llm_connection success: provider={}, model={}, response={}",
                provider,
                model,
                preview
            );

            Ok(format!(
                "ok: provider={}, model={}, response='{}'",
                provider, model, preview
            ))
        }
        Ok(Err(e)) => {
            tracing::warn!(
                "test_llm_connection API error: provider={}, model={}, error={}",
                provider,
                model,
                e
            );
            Ok(format!("error: API request failed: {}", e))
        }
        Err(_) => {
            tracing::warn!(
                "test_llm_connection timeout: provider={}, model={}",
                provider,
                model
            );
            Ok("error: request timed out after 15 seconds".to_string())
        }
    }
}

/// 从服务商 /v1/models 拉取最新模型列表（list_provider_models 与 refresh_provider_models 共用核心）。
async fn fetch_provider_models(
    api_key: &str,
    provider: &str,
    base_url: Option<&str>,
) -> Result<Vec<String>, String> {
    use std::time::Duration;

    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }

    let registry = ProviderRegistry::builtin();
    let pmeta = registry
        .get(provider)
        .ok_or_else(|| format!("Unknown provider: {}", provider))?;

    let resolved_base_url = base_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| pmeta.default_base_url());

    let url = format!("{}/models", resolved_base_url.trim_end_matches('/'));
    let auth_header = pmeta.auth_header();
    let auth_prefix = pmeta.auth_prefix();
    let auth_value = format!("{}{}", auth_prefix, api_key);

    tracing::info!(
        "[list-provider-models] GET {} for provider={}",
        url,
        provider
    );

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Create HTTP client failed: {}", e))?;

    let response = client
        .get(&url)
        .header(auth_header, &auth_value)
        .header("User-Agent", "Nuphus/1.0")
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("API 错误 ({}): {}", status, msg));
    }

    // Extract model IDs from common response formats
    let mut models = Vec::new();
    for array_key in &["data", "models", "model_list"] {
        if let Some(arr) = body.get(array_key).and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    models.push(id.to_string());
                }
            }
            if !models.is_empty() {
                break;
            }
        }
    }

    if models.is_empty() {
        return Err("API 未返回任何可用模型".to_string());
    }

    models.sort();
    models.dedup();
    tracing::info!(
        "[list-provider-models] Found {} models for provider={}",
        models.len(),
        provider
    );
    Ok(models)
}

/// 通过 /v1/models 检测 API key 并列出可用模型（连接检测：key 必须由前端显式传入）。
#[tauri::command]
pub async fn list_provider_models(
    api_key: String,
    provider: String,
    base_url: Option<String>,
) -> Result<Vec<String>, String> {
    fetch_provider_models(&api_key, &provider, base_url.as_deref()).await
}

/// 刷新某服务商最新模型列表：读取 config.toml 已存 API key（不暴露 key 本身），
/// 拉取 /v1/models 返回并集排序后的模型 ID。未配置 key 时报错引导先连接。
#[tauri::command]
pub async fn refresh_provider_models(
    provider: String,
    base_url: Option<String>,
) -> Result<Vec<String>, String> {
    let api_key = read_provider_api_key_from_config_toml(&provider)
        .ok_or_else(|| "该服务商尚未配置 API Key，请先在连接区域输入并保存".to_string())?;
    let models = fetch_provider_models(&api_key, &provider, base_url.as_deref()).await?;

    // 持久化：把 API 返回的最新模型 ID 合并进 config.toml，使 list_models
    // （图像理解 / STT / TTS 选择器数据源）能看到刷新发现的新模型。
    if let Some(config_path) = get_config_path() {
        let _ = upsert_provider_models(&config_path, &provider, &models);
    }

    Ok(models)
}

/// Supported Provider info
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub auth_header: String,
    pub auth_prefix: String,
}

#[tauri::command]
pub fn get_supported_providers() -> Result<Vec<ProviderInfo>, String> {
    let providers = ProviderRegistry::builtin()
        .list_info()
        .iter()
        .map(|p| ProviderInfo {
            id: p.id.to_string(),
            name: p.name.to_string(),
            base_url: p.base_url.to_string(),
            default_model: p.default_model.to_string(),
            auth_header: p.auth_header.to_string(),
            auth_prefix: p.auth_prefix.to_string(),
        })
        .collect::<Vec<_>>();

    tracing::info!(
        "get_supported_providers: returning {} providers",
        providers.len()
    );
    Ok(providers)
}

#[tauri::command]
pub fn get_capabilities(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    use nuphus::config::ModelRegistry;

    // Use providers.toml as canonical location for capabilities
    // (capabilities belong to the model registry, same as providers)
    let providers_path = state.llm_config_path.with_file_name("providers.toml");
    let registry = if providers_path.exists() {
        ModelRegistry::from_toml(providers_path.to_str().unwrap_or("providers.toml"))
            .map_err(|e| format!("load providers.toml failed: {}", e))?
    } else if let Some(path) = get_config_path() {
        ModelRegistry::from_toml(path.to_str().unwrap_or("config.toml"))
            .map_err(|e| format!("load config failed: {}", e))?
    } else {
        ModelRegistry::from_env().map_err(|e| format!("load from env failed: {}", e))?
    };

    let caps = &registry.capabilities;
    let result = serde_json::json!({
        "model": registry.model,
        "vision": caps.vision,
        "stt": caps.stt,
        "tts": caps.tts,
        "chat_agent_max_iterations": caps.chat_agent_max_iterations,
    });

    tracing::info!("get_capabilities: {:?}", result);
    Ok(result)
}

/// Model capability metadata discovered from the provider's /models endpoint.
/// Kimi additionally exposes per-model reasoning-effort capability
/// (`think_efforts { valid_efforts, default_effort }`); providers that return
/// bare id lists (DeepSeek/MiniMax) simply yield empty efforts.
struct ModelApiMetadata {
    context_length: Option<usize>,
    reasoning_efforts: Vec<String>,
    default_effort: Option<String>,
}

/// Extract reasoning-effort capability from a /models model entry.
/// Known shape: Kimi `think_efforts { valid_efforts[], default_effort }`.
fn extract_reasoning_efforts(m: &serde_json::Value) -> (Vec<String>, Option<String>) {
    if let Some(te) = m.get("think_efforts") {
        let efforts = te
            .get("valid_efforts")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let default = te
            .get("default_effort")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (efforts, default);
    }
    (Vec::new(), None)
}

/// Locate the model entry (by id/model field, case-insensitive) in a /models array.
fn find_model_entry<'a>(
    models: &'a [serde_json::Value],
    target_model: &str,
) -> Option<&'a serde_json::Value> {
    let target = target_model.to_lowercase();
    models.iter().find(|m| {
        m.get("id")
            .or_else(|| m.get("model"))
            .and_then(|v| v.as_str())
            .map(|id| id.to_lowercase() == target)
            .unwrap_or(false)
    })
}

fn query_model_metadata_from_api(
    base_url: &str,
    model: &str,
    api_key: &str,
    auth_header: &str,
    auth_prefix: &str,
) -> Option<ModelApiMetadata> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let auth_value = format!("{}{}", auth_prefix, api_key);
    tracing::info!("[model-meta] GET {} for model={}", url, model);

    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[model-meta] create client failed: {}", e);
            return None;
        }
    };

    let response = match client
        .get(&url)
        .header(auth_header, &auth_value)
        .header("User-Agent", "Nuphus/1.0")
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "[model-meta] GET {} failed: {} (auth_header={}, has_key={})",
                url,
                e,
                auth_header,
                !api_key.is_empty()
            );
            return None;
        }
    };

    let status = response.status();
    let body_text = match response.text() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("[model-meta] read body failed: {}", e);
            return None;
        }
    };

    if status.as_u16() != 200 {
        tracing::warn!(
            "[model-meta] HTTP {} from {}: {}",
            status,
            url,
            body_text.chars().take(200).collect::<String>()
        );
        return None;
    }

    let body: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "[model-meta] JSON parse failed from {}: {} — body preview: {}",
                url,
                e,
                body_text.chars().take(100).collect::<String>()
            );
            return None;
        }
    };

    // Context window field names (provider-agnostic)
    const CTX_KEYS: &[&str] = &[
        "context_length",
        "max_context_length",
        "context_window",
        "max_tokens",
        "max_input_tokens",
    ];

    let build_meta = |m: &serde_json::Value| {
        let (reasoning_efforts, default_effort) = extract_reasoning_efforts(m);
        let meta = ModelApiMetadata {
            context_length: extract_context(m, CTX_KEYS),
            reasoning_efforts,
            default_effort,
        };
        tracing::info!(
            "[model-meta] LIVE: model={} context={:?} efforts={:?} default={:?}",
            model,
            meta.context_length,
            meta.reasoning_efforts,
            meta.default_effort
        );
        meta
    };

    // Try to find model entry in arrays: data[], models[], model_list[]
    for array_key in &["data", "models", "model_list"] {
        if let Some(arr) = body.get(array_key).and_then(|d| d.as_array()) {
            if let Some(entry) = find_model_entry(arr, model) {
                return Some(build_meta(entry));
            }
        }
    }

    // Some APIs return a single model object directly at top level
    if let Some(id) = body
        .get("id")
        .or_else(|| body.get("model"))
        .and_then(|v| v.as_str())
    {
        if id.to_lowercase() == model.to_lowercase() {
            return Some(build_meta(&body));
        }
    }

    // Some APIs return model metadata at body.{model_name}
    if let Some(model_obj) = body.get(model.to_lowercase()) {
        return Some(build_meta(model_obj));
    }

    tracing::warn!(
        "[model-meta] model={} not found in /models response. Available keys: {:?}, has data={}, has models={}",
        model,
        body.as_object().map(|o| o.keys().take(10).collect::<Vec<_>>()).unwrap_or_default(),
        body.get("data").is_some(),
        body.get("models").is_some(),
    );
    None
}

/// Probe whether a model supports vision by sending a 1x1 PNG as image_url.
///
/// Returns:
/// - `Some(true)` — API returned 200, model accepts image input
/// - `Some(false)` — API returned 400+ (likely doesn't support vision)
/// - `None` — network/auth error, indeterminate — don't touch config
fn probe_vision(
    base_url: &str,
    model: &str,
    api_key: &str,
    auth_header: &str,
    auth_prefix: &str,
) -> Option<bool> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let auth_value = format!("{}{}", auth_prefix, api_key);

    // 1x1 blue pixel PNG, ~67 bytes → ~90 chars base64
    let tiny_png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    let body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": "ok" },
                { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", tiny_png) } }
            ]
        }],
        "max_tokens": 5,
    });

    tracing::info!("[vision-probe] POST {} for model={}", url, model);

    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[vision-probe] create client failed: {}", e);
            return None;
        }
    };

    let response = match client
        .post(&url)
        .header(auth_header, &auth_value)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[vision-probe] POST failed: {}", e);
            return None;
        }
    };

    let status = response.status();
    if status.is_success() {
        tracing::info!("[vision-probe] model={} supports vision ✓", model);
        Some(true)
    } else {
        tracing::info!(
            "[vision-probe] model={} returned HTTP {}, likely no vision support",
            model,
            status
        );
        Some(false)
    }
}

fn extract_context(obj: &serde_json::Value, ctx_keys: &[&str]) -> Option<usize> {
    for key in ctx_keys {
        if let Some(ctx) = obj.get(key).and_then(|v| v.as_u64()) {
            return Some(ctx as usize);
        }
    }
    None
}

#[tauri::command]
pub fn get_context_limit(state: State<'_, AppState>) -> Result<usize, String> {
    // 1. Prefer backend cached value (real model window set during configure_llm)
    {
        let cw = state.runtime.lock().map_err(|e| e.to_string())?;
        if cw.model_context_window > 0 {
            return Ok(cw.model_context_window);
        }
    }

    // 2. When cache is empty, infer from current LLM config's model name
    //    (scoped so the lock drops before re-locking for write-back)
    let ctx = {
        let config = state.runtime.lock().map_err(|e| e.to_string())?;
        config
            .llm_config
            .as_ref()
            .map(|cfg| nuphus::agent::goal_types::get_context_window(&cfg.model))
    };
    if let Some(ctx) = ctx {
        // Write back to cache
        let mut cw = state.runtime.lock().map_err(|e| e.to_string())?;
        cw.model_context_window = ctx;
        return Ok(ctx);
    }

    // 3. Not configured yet → return default, frontend will refresh later
    Ok(128000)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写临时配置：agent_models 内容 + registry config（providers 列表）。
    /// 返回 (providers_path, config_path)，测试结束由调用方清理目录。
    fn write_fixtures(
        agent_models_toml: &str,
        model_ids: &[&str],
        fallback: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("nuphus-llm-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();

        let am_path = dir.join("providers.toml");
        std::fs::write(&am_path, agent_models_toml).unwrap();

        let cfg_path = dir.join("config.toml");
        let mut cfg = format!("model = \"{fallback}\"\n\n[[providers]]\nname = \"deepseek\"\nprovider_type = \"deepseek\"\napi_key = \"sk-x\"\nbase_url = \"https://api.deepseek.com\"\n");
        for m in model_ids {
            cfg += &format!("\n[[providers.models]]\nid = \"{m}\"\n");
        }
        std::fs::write(&cfg_path, cfg).unwrap();
        (am_path, cfg_path)
    }

    #[test]
    fn effective_model_full_resolution() {
        let am = "[agent_models]\nleader = \"leader-model\"\ndefault = \"default-model\"\nworkflow = \"wf-model\"\nexec = \"exec-model\"\ncustom = \"custom-model\"\n";
        let ids = [
            "leader-model",
            "default-model",
            "wf-model",
            "exec-model",
            "custom-model",
            "fallback-model",
        ];
        let (am_path, cfg_path) = write_fixtures(am, &ids, "fallback-model");
        let registry =
            nuphus::config::ModelRegistry::from_toml(cfg_path.to_str().unwrap()).unwrap();

        assert_eq!(
            effective_model(&am_path, &registry, "leader"),
            "leader-model"
        );
        assert_eq!(effective_model(&am_path, &registry, "workflow"), "wf-model");
        assert_eq!(
            effective_model(&am_path, &registry, "custom"),
            "custom-model"
        );
        assert_eq!(effective_model(&am_path, &registry, "exec"), "exec-model");
        assert_eq!(
            effective_model(&am_path, &registry, "unknown-mode"),
            "leader-model"
        );
        std::fs::remove_dir_all(am_path.parent().unwrap()).ok();
    }

    #[test]
    fn effective_model_fallback_chain() {
        // leader 配置了但 registry 无此模型（不可用）→ 回退顶部 model
        let am = "[agent_models]\nleader = \"missing-leader\"\ndefault = \"missing-default\"\n";
        let ids = ["wf-model", "fallback-model"];
        let (am_path, cfg_path) = write_fixtures(am, &ids, "fallback-model");
        let registry =
            nuphus::config::ModelRegistry::from_toml(cfg_path.to_str().unwrap()).unwrap();

        // leader 不可用 → 回退 registry.model
        assert_eq!(
            effective_model(&am_path, &registry, "leader"),
            "fallback-model"
        );
        // default 不可用 → 回退 leader（=fallback-model）
        assert_eq!(
            effective_model(&am_path, &registry, "default"),
            "fallback-model"
        );
        // workflow 未配置（空）→ 回退 default → leader → fallback-model
        // （注意：registry 里有 wf-model，但 agent_models 未显式配置 workflow 字段，
        //   effective_model 不会「发现」它——只有显式配置才生效）
        assert_eq!(
            effective_model(&am_path, &registry, "workflow"),
            "fallback-model"
        );
        std::fs::remove_dir_all(am_path.parent().unwrap()).ok();
    }

    #[test]
    fn effective_model_missing_agent_models_file() {
        // providers.toml 不存在 → AgentModels 全空 → 全部回退顶部 model
        let dir =
            std::env::temp_dir().join(format!("nuphus-llm-test-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let am_path = dir.join("does-not-exist.toml");
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "model = \"fallback-model\"\n\n[[providers]]\nname = \"deepseek\"\nprovider_type = \"deepseek\"\napi_key = \"sk-x\"\n\n[[providers.models]]\nid = \"fallback-model\"\n").unwrap();
        let registry =
            nuphus::config::ModelRegistry::from_toml(cfg_path.to_str().unwrap()).unwrap();

        assert_eq!(
            effective_model(&am_path, &registry, "leader"),
            "fallback-model"
        );
        assert_eq!(
            effective_model(&am_path, &registry, "workflow"),
            "fallback-model"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}