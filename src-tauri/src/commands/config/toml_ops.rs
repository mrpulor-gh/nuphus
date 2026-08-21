//! TOML config.toml CRUD helpers.
//!
//! Lowest layer of the config module — reads/writes `config.toml` (a.k.a.
//! `providers.toml` / `nuphus.toml`) and provides the canonical `get_config_path`
//! used by every other module in `config::` as well as by sibling modules
//! (`attachment`, `process`, etc.).

// ============================================================================
// context_window & supports_vision model fields
// ============================================================================

/// Update model context_window in config.toml model entry
pub fn update_model_context_window(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
    context_window: usize,
) -> Result<(), String> {
    // If file doesn't exist yet, silently skip — creating it is update_config_toml's job
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };

    let providers = match doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        Some(p) => p,
        None => return Ok(()),
    };

    for provider in providers.iter_mut() {
        if let Some(name) = provider.get("name").and_then(|n| n.as_str()) {
            if name == provider_name {
                if let Some(map) = provider.as_table_mut() {
                    if let Some(models) = map.get_mut("models").and_then(|m| m.as_array_mut()) {
                        for model in models.iter_mut() {
                            if let Some(id) = model.get("id").and_then(|i| i.as_str()) {
                                if id == model_id {
                                    if let Some(map) = model.as_table_mut() {
                                        map.insert(
                                            "context_window".to_string(),
                                            toml::Value::Integer(context_window as i64),
                                        );
                                        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
                                        let new_content =
                                            toml::to_string_pretty(&doc).map_err(|e| {
                                                format!("serialize config.toml failed: {}", e)
                                            })?;
                                        std::fs::write(config_path, new_content).map_err(|e| {
                                            format!("write config.toml failed: {}", e)
                                        })?;
                                        tracing::info!(
                                            "Updated context_window for {}/{}: {}",
                                            provider_name,
                                            model_id,
                                            context_window
                                        );
                                    }
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Update model reasoning-effort metadata in config.toml model entry
/// (discovered from the provider's /models response at configure time).
pub fn update_model_reasoning_efforts(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
    efforts: &[String],
    default_effort: Option<&str>,
) -> Result<(), String> {
    if efforts.is_empty() {
        return Ok(());
    }
    // If file doesn't exist yet, silently skip — creating it is update_config_toml's job
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };

    let providers = match doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        Some(p) => p,
        None => return Ok(()),
    };

    for provider in providers.iter_mut() {
        if let Some(name) = provider.get("name").and_then(|n| n.as_str()) {
            if name == provider_name {
                if let Some(map) = provider.as_table_mut() {
                    if let Some(models) = map.get_mut("models").and_then(|m| m.as_array_mut()) {
                        for model in models.iter_mut() {
                            if let Some(id) = model.get("id").and_then(|i| i.as_str()) {
                                if id == model_id {
                                    if let Some(map) = model.as_table_mut() {
                                        map.insert(
                                            "reasoning_efforts".to_string(),
                                            toml::Value::Array(
                                                efforts
                                                    .iter()
                                                    .map(|e| toml::Value::String(e.clone()))
                                                    .collect(),
                                            ),
                                        );
                                        if let Some(d) = default_effort {
                                            map.insert(
                                                "default_effort".to_string(),
                                                toml::Value::String(d.to_string()),
                                            );
                                        }
                                        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
                                        let new_content =
                                            toml::to_string_pretty(&doc).map_err(|e| {
                                                format!("serialize config.toml failed: {}", e)
                                            })?;
                                        std::fs::write(config_path, new_content).map_err(|e| {
                                            format!("write config.toml failed: {}", e)
                                        })?;
                                        tracing::info!(
                                            "Updated reasoning_efforts for {}/{}: {:?} (default {:?})",
                                            provider_name, model_id, efforts, default_effort
                                        );
                                    }
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Update model supports_vision in config.toml model entry
pub fn update_model_supports_vision(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
    supports_vision: bool,
) -> Result<(), String> {
    // If file doesn't exist yet, silently skip
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };

    let providers = match doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        Some(p) => p,
        None => return Ok(()),
    };

    for provider in providers.iter_mut() {
        if let Some(name) = provider.get("name").and_then(|n| n.as_str()) {
            if name == provider_name {
                if let Some(map) = provider.as_table_mut() {
                    if let Some(models) = map.get_mut("models").and_then(|m| m.as_array_mut()) {
                        for model in models.iter_mut() {
                            if let Some(id) = model.get("id").and_then(|i| i.as_str()) {
                                if id == model_id {
                                    if let Some(map) = model.as_table_mut() {
                                        map.insert(
                                            "supports_vision".to_string(),
                                            toml::Value::Boolean(supports_vision),
                                        );
                                        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
                                        let new_content =
                                            toml::to_string_pretty(&doc).map_err(|e| {
                                                format!("serialize config.toml failed: {}", e)
                                            })?;
                                        std::fs::write(config_path, new_content).map_err(|e| {
                                            format!("write config.toml failed: {}", e)
                                        })?;
                                        tracing::info!(
                                            "Updated supports_vision for {}/{}: {}",
                                            provider_name,
                                            model_id,
                                            supports_vision
                                        );
                                    }
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ============================================================================
// Provider config + model registration
// ============================================================================

/// Upsert fetched model IDs into config.toml `[[providers]].models`.
///
/// 「刷新模型列表」时调用：把 API /v1/models 返回的最新模型 ID 合并进配置，
/// 已存在的模型保留其元数据（context_window / supports_vision / reasoning_efforts 等），
/// 新增的模型写入最小条目（supports_streaming 默认 true，与 ModelEntry serde 一致）。
/// 这样 list_models（图像理解/STT/TTS 选择器数据源）能立刻看到刷新发现的新模型。
pub fn upsert_provider_models(
    config_path: &std::path::Path,
    provider_name: &str,
    model_ids: &[String],
) -> Result<(), String> {
    if model_ids.is_empty() {
        return Ok(());
    }
    // If file doesn't exist yet, silently skip — creating it is update_config_toml's job
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };

    let providers = match doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        Some(p) => p,
        None => return Ok(()),
    };

    for provider in providers.iter_mut() {
        if let Some(name) = provider.get("name").and_then(|n| n.as_str()) {
            if name != provider_name {
                continue;
            }
            if let Some(map) = provider.as_table_mut() {
                // Collect existing model ids
                let mut existing: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                if let Some(models) = map.get("models").and_then(|m| m.as_array()) {
                    for model in models {
                        if let Some(id) = model.get("id").and_then(|i| i.as_str()) {
                            existing.insert(id.to_string());
                        }
                    }
                }
                // Append missing models
                let mut added = 0usize;
                for id in model_ids {
                    if existing.contains(id) {
                        continue;
                    }
                    let mut entry = toml::map::Map::new();
                    entry.insert("id".to_string(), toml::Value::String(id.clone()));
                    // 与 ModelEntry 的 serde 默认一致：supports_streaming 默认 true
                    entry.insert("supports_streaming".to_string(), toml::Value::Boolean(true));
                    map.entry("models".to_string())
                        .or_insert_with(|| toml::Value::Array(vec![]));
                    if let Some(models_arr) = map.get_mut("models").and_then(|m| m.as_array_mut()) {
                        models_arr.push(toml::Value::Table(entry));
                        existing.insert(id.clone());
                        added += 1;
                    }
                }
                if added > 0 {
                    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
                    let new_content = toml::to_string_pretty(&doc)
                        .map_err(|e| format!("serialize config.toml failed: {}", e))?;
                    std::fs::write(config_path, new_content)
                        .map_err(|e| format!("write config.toml failed: {}", e))?;
                    tracing::info!(
                        "upsert_provider_models: added {} new models for provider={}",
                        added,
                        provider_name
                    );
                }
            }
            return Ok(());
        }
    }
    Ok(())
}

/// Update `reasoning_effort` on a `[[providers]]` entry in config.toml.
/// `None`/empty removes the field so the provider returns to its default
/// (transport sends no `reasoning_effort` parameter).
pub fn update_reasoning_effort(
    config_path: &std::path::Path,
    provider_name: &str,
    effort: Option<&str>,
) -> Result<(), String> {
    // If file doesn't exist yet, silently skip — creating it is update_config_toml's job
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(()),
    };

    let providers = match doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        Some(p) => p,
        None => return Ok(()),
    };

    for provider in providers.iter_mut() {
        if let Some(name) = provider.get("name").and_then(|n| n.as_str()) {
            if name == provider_name {
                if let Some(map) = provider.as_table_mut() {
                    match effort {
                        Some(e) if !e.is_empty() => {
                            map.insert(
                                "reasoning_effort".to_string(),
                                toml::Value::String(e.to_string()),
                            );
                        }
                        _ => {
                            map.remove("reasoning_effort");
                        }
                    }
                    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
                    let new_content = toml::to_string_pretty(&doc)
                        .map_err(|e| format!("serialize config.toml failed: {}", e))?;
                    std::fs::write(config_path, new_content)
                        .map_err(|e| format!("write config.toml failed: {}", e))?;
                    tracing::info!(
                        "Updated reasoning_effort for {}: {:?}",
                        provider_name,
                        effort
                    );
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

/// Update provider config in config.toml
/// Parse/modify with toml::Value, preserving comments and other fields
pub fn update_config_toml(
    config_path: &std::path::Path,
    provider_name: &str,
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
) -> Result<(), String> {
    // Read existing config, or start fresh if file doesn't exist yet
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = content.parse().unwrap_or_else(|_| {
        let mut table = toml::value::Table::new();
        table.insert("providers".to_string(), toml::Value::Array(Vec::new()));
        toml::Value::Table(table)
    });

    // Ensure providers array exists (file may be valid TOML created by an
    // older path that didn't include the providers key)
    if let Some(table) = doc.as_table_mut() {
        if !table.contains_key("providers") {
            table.insert("providers".to_string(), toml::Value::Array(Vec::new()));
        }
    }

    // Get providers array
    let providers = doc
        .get_mut("providers")
        .and_then(|p| p.as_array_mut())
        .ok_or_else(|| "config.toml missing providers array".to_string())?;

    let mut provider_found = false;
    let mut provider_idx = 0;

    // Find matching provider
    for (idx, provider) in providers.iter_mut().enumerate() {
        if let Some(name) = provider.get("name").and_then(|n| n.as_str()) {
            if name == provider_name {
                provider_found = true;
                provider_idx = idx;

                // Update api_key（DPAPI 加密落盘；读取端透明解密）
                if let Some(map) = provider.as_table_mut() {
                    map.insert(
                        "api_key".to_string(),
                        toml::Value::String(nuphus::cookies::encrypt_secret(api_key)),
                    );
                    // Update base_url only when provided non-empty
                    if let Some(url) = base_url {
                        if !url.is_empty() {
                            map.insert(
                                "base_url".to_string(),
                                toml::Value::String(url.to_string()),
                            );
                        }
                    }
                }
                break;
            }
        }
    }

    // If provider doesn't exist, append new one
    if !provider_found {
        let mut new_provider = toml::value::Table::new();
        new_provider.insert(
            "name".to_string(),
            toml::Value::String(provider_name.to_string()),
        );
        new_provider.insert(
            "provider_type".to_string(),
            toml::Value::String(provider_name.to_string()),
        );
        new_provider.insert(
            "api_key".to_string(),
            toml::Value::String(nuphus::cookies::encrypt_secret(api_key)),
        );
        if let Some(url) = base_url {
            if !url.is_empty() {
                new_provider.insert("base_url".to_string(), toml::Value::String(url.to_string()));
            }
        }

        providers.push(toml::Value::Table(new_provider));
        provider_idx = providers.len() - 1;
    }

    // Ensure the user's model_id is in the provider's models list (needed by find_model)
    if let Some(provider) = providers.get_mut(provider_idx) {
        if let Some(map) = provider.as_table_mut() {
            let models = map
                .entry("models")
                .or_insert_with(|| toml::Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| "models field is not array".to_string())?;

            if !models
                .iter()
                .any(|m| m.get("id").and_then(|i| i.as_str()) == Some(model_id))
            {
                let mut model_entry = toml::value::Table::new();
                model_entry.insert("id".to_string(), toml::Value::String(model_id.to_string()));
                model_entry.insert("supports_streaming".to_string(), toml::Value::Boolean(true));
                models.push(toml::Value::Table(model_entry));
            }
        }
    }

    // Update model
    if let Some(map) = doc.as_table_mut() {
        map.insert(
            "model".to_string(),
            toml::Value::String(model_id.to_string()),
        );
    }

    // Write back to file
    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("serialize config.toml failed: {}", e))?;

    std::fs::write(config_path, new_content)
        .map_err(|e| format!("write config.toml failed: {}", e))?;

    tracing::info!(
        "Updated config.toml: provider={}, model={}",
        provider_name,
        model_id
    );
    Ok(())
}

// ============================================================================
// Provider/key queries
// ============================================================================

/// Read a provider's API key from config.toml.
/// Returns `None` when the key is missing OR empty — prevents callers from
/// silently using an empty auth header (串台 root cause).
pub fn read_provider_api_key_from_config_toml(provider_name: &str) -> Option<String> {
    let config_path = get_config_path()?;
    let content = std::fs::read_to_string(config_path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    let providers = doc.get("providers")?.as_array()?;
    for provider in providers {
        let name = provider.get("name")?.as_str()?;
        if name == provider_name {
            let key = provider.get("api_key").and_then(|k| k.as_str())?;
            if key.is_empty() {
                return None;
            }
            // 透明解密：enc:v1: 前缀走 DPAPI；旧明文配置原样兼容；解密失败视为缺失
            return nuphus::cookies::decrypt_secret(key);
        }
    }
    None
}

/// Read a provider's reasoning-effort value from config.toml
/// (`[[providers]] reasoning_effort`, e.g. `"low" | "high" | "max"`).
/// Returns `None` when absent or empty — transport default applies.
pub fn read_provider_reasoning_effort_from_config_toml(provider_name: &str) -> Option<String> {
    let config_path = get_config_path()?;
    let content = std::fs::read_to_string(config_path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    let providers = doc.get("providers")?.as_array()?;
    for provider in providers {
        let name = provider.get("name")?.as_str()?;
        if name == provider_name {
            let effort = provider.get("reasoning_effort").and_then(|e| e.as_str())?;
            if effort.is_empty() {
                return None;
            }
            return Some(effort.to_string());
        }
    }
    None
}

/// Collect all provider names that have non-empty API keys in config.toml.
pub fn list_configured_providers() -> Vec<String> {
    let config_path = match get_config_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    use nuphus::config::ModelRegistry;
    match ModelRegistry::from_toml(config_path.to_str().unwrap_or("config.toml")) {
        Ok(registry) => registry
            .providers
            .iter()
            .filter(|p| !p.api_key.is_empty())
            .map(|p| p.name.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// ============================================================================
// Config file location
// ============================================================================

/// Get config file path (delegates to shared config_search_paths)
pub fn get_config_path() -> Option<std::path::PathBuf> {
    for path in &nuphus::config::config_search_paths() {
        if path.exists() {
            return Some(path.clone());
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp_config(content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nuphus_toml_ops_test_{}.toml",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn upsert_appends_new_models_and_keeps_existing() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"
base_url = "https://api.deepseek.com"

[[providers.models]]
id = "deepseek-v4-flash"
supports_streaming = true
supports_vision = true
"#,
        );

        let new_ids = vec![
            "deepseek-v4-flash".to_string(),      // 已存在 → 不重复
            "deepseek-v4-multimodal".to_string(), // 新模型 → 追加
            "deepseek-v4-pro".to_string(),        // 新模型 → 追加
        ];
        upsert_provider_models(&path, "deepseek", &new_ids).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        let providers = doc.get("providers").unwrap().as_array().unwrap();
        let deepseek = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("deepseek"))
            .unwrap();
        let models = deepseek.get("models").unwrap().as_array().unwrap();

        // 3 个模型：原 1 + 新 2（无重复）
        assert_eq!(
            models.len(),
            3,
            "models should be merged without dup: {}",
            content
        );
        let ids: Vec<&str> = models
            .iter()
            .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
            .collect();
        assert!(ids.contains(&"deepseek-v4-flash"));
        assert!(ids.contains(&"deepseek-v4-multimodal"));
        assert!(ids.contains(&"deepseek-v4-pro"));

        // 已有模型的元数据必须保留
        let existing = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-v4-flash"))
            .unwrap();
        assert_eq!(
            existing.get("supports_vision").and_then(|v| v.as_bool()),
            Some(true),
            "existing model metadata must be preserved"
        );

        // 新模型带 supports_streaming=true 默认
        let new_m = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-v4-multimodal"))
            .unwrap();
        assert_eq!(
            new_m.get("supports_streaming").and_then(|v| v.as_bool()),
            Some(true)
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn upsert_does_not_touch_other_providers() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-a"
base_url = "https://api.deepseek.com"

[[providers]]
name = "kimi"
provider_type = "kimi"
api_key = "sk-b"
base_url = "https://api.kimi.com"

[[providers.models]]
id = "kimi-for-coding"
supports_streaming = true
"#,
        );

        upsert_provider_models(&path, "deepseek", &["deepseek-new".to_string()]).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        let providers = doc.get("providers").unwrap().as_array().unwrap();

        let deepseek = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("deepseek"))
            .unwrap();
        let ds_models = deepseek.get("models").unwrap().as_array().unwrap();
        assert_eq!(ds_models.len(), 1);
        assert_eq!(
            ds_models[0].get("id").and_then(|i| i.as_str()),
            Some("deepseek-new")
        );

        let kimi = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("kimi"))
            .unwrap();
        let kimi_models = kimi.get("models").unwrap().as_array().unwrap();
        assert_eq!(kimi_models.len(), 1, "kimi should not be touched");
        assert_eq!(
            kimi_models[0].get("id").and_then(|i| i.as_str()),
            Some("kimi-for-coding")
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn upsert_empty_ids_is_noop() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"

[[providers.models]]
id = "deepseek-v4-flash"
"#,
        );
        upsert_provider_models(&path, "deepseek", &[]).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        let models = doc.get("providers").unwrap().as_array().unwrap()[0]
            .get("models")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(models.len(), 1);
        std::fs::remove_file(&path).ok();
    }
}
