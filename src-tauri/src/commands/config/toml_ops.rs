//! TOML config.toml CRUD helpers.
//!
//! Lowest layer of the config module — reads/writes `config.toml` (a.k.a.
//! `providers.toml` / `nuphus.toml`) and provides the canonical `get_config_path`
//! used by every other module in `config::` as well as by sibling modules
//! (`attachment`, `process`, etc.).

// ============================================================================
// context_window & supports_vision model fields
// ============================================================================

/// Validate a Custom provider instance name. The legacy `custom` segment remains valid;
/// new instances must use a stable ASCII `custom-xxx` identity.
fn validate_custom_provider_name(name: &str) -> Result<(), String> {
    if name == "custom" {
        return Ok(());
    }
    let suffix = name.strip_prefix("custom-").unwrap_or("");
    let valid = name.len() <= 64
        && !suffix.is_empty()
        && !suffix.ends_with('-')
        && name.starts_with("custom-")
        && suffix
            .chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_lowercase() || c.is_ascii_digit() || (c == '-' && i > 0));
    if valid {
        Ok(())
    } else {
        Err("自定义服务商名称必须符合 custom-xxx（小写英文、数字和连字符，且不可重复）".to_string())
    }
}

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

/// Read existing context_window for a model entry (if recorded in config.toml).
/// 用于「本地记录优先」：API key 联通时查询到的 context_length 只填充缺失项，
/// 不覆盖本地已记录（用户校准/历史）值——实测 API 返回的 context_length 常为
/// provider 统一值或不准确，无条件覆盖会污染本地各模型记录，前端上下文占用显示混乱。
pub fn read_model_context_window(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
) -> Option<usize> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    let providers = doc.get("providers")?.as_array()?;
    for provider in providers {
        let name = provider.get("name")?.as_str()?;
        if name != provider_name {
            continue;
        }
        let models = provider.get("models")?.as_array()?;
        for model in models {
            let id = model.get("id")?.as_str()?;
            if id == model_id {
                return model
                    .get("context_window")?
                    .as_integer()
                    .map(|v| v as usize);
            }
        }
    }
    None
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

/// Update model supports_vision in config.toml model entry.
///
/// `source` 记录该值的来源：`Some("user")` = 用户在模型行内手动设定，
/// 探测链路（post_configure 的 metadata/HTTP probe）必须让位于用户意图，
/// 否则用户今天勾上的视觉能力会在下次连接时被探测结果覆盖掉。
/// `None` = 自动探测结果，不改动已有的来源标记。
pub fn update_model_supports_vision(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
    supports_vision: bool,
    source: Option<&str>,
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
                                        if let Some(src) = source {
                                            map.insert(
                                                VISION_SOURCE_KEY.to_string(),
                                                toml::Value::String(src.to_string()),
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

/// `supports_vision` 的来源标记键：值 `user` = 用户手动设定，探测链路不得覆盖。
pub const VISION_SOURCE_KEY: &str = "supports_vision_source";

/// 读取模型条目里的来源标记（仅认 `user`；其它/缺失 = 非用户设定）。
pub fn read_model_vision_source(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
) -> Option<String> {
    read_model_field(config_path, provider_name, model_id, VISION_SOURCE_KEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

/// 该模型是否由用户显式设定视觉能力（探测链路据此让位）。
pub fn model_has_user_vision_override(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
) -> bool {
    read_model_vision_source(config_path, provider_name, model_id).as_deref() == Some("user")
}

/// 读取模型条目的 `supports_vision`（用于写入后回读校验）。
pub fn read_model_supports_vision(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
) -> Option<bool> {
    read_model_field(config_path, provider_name, model_id, "supports_vision")
        .and_then(|v| v.as_bool())
}

/// 读取 `providers[provider].models[id]` 下的单个字段。
fn read_model_field(
    config_path: &std::path::Path,
    provider_name: &str,
    model_id: &str,
    key: &str,
) -> Option<toml::Value> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    doc.get("providers")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(provider_name))?
        .get("models")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(|i| i.as_str()) == Some(model_id))?
        .get(key)
        .cloned()
}

/// 原子写入视觉模型绑定：`capabilities.vision` 与 `capabilities.vision_provider`
/// 必须在**同一次读写**内落盘。
///
/// 分两次写会出现「新 model + 旧 provider」的中间态：后端按 provider+model 精确
/// 解析时找不到该组合，视觉请求直接失败（用户看到的是「保存成功但用不了」）。
pub fn set_vision_capability_in_config_toml(
    config_path: &std::path::Path,
    model_id: &str,
    provider_name: &str,
) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config.toml: {}", e))?;
    let mut doc: toml::Value = content
        .parse()
        .map_err(|e| format!("parse config.toml failed: {}", e))?;

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
        "vision".to_string(),
        toml::Value::String(model_id.to_string()),
    );
    // 空 provider（清除视觉模型）时一并清掉归属，避免留下悬空引用。
    if provider_name.is_empty() {
        caps.remove("vision_provider");
    } else {
        caps.insert(
            "vision_provider".to_string(),
            toml::Value::String(provider_name.to_string()),
        );
    }

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(config_path, new_content)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;
    Ok(())
}

// ============================================================================
// Provider config + model registration
// ============================================================================

// ============================================================================
// Model sync engine — reconcile a provider segment with the /v1/models catalog
// ============================================================================

/// `[[providers]].models.source` key (mirrors [`nuphus::config::ModelSource`]).
const MODEL_SOURCE_KEY: &str = "source";

/// Authoritative capability metadata for one model, resolved by the caller
/// (see [`CapabilitySource`]).
///
/// Every field is optional and `None` means "the authority has no declared value
/// for this field" — the sync then never writes a guess: a fresh entry stays
/// empty and an existing value is left untouched. Ambiguity is resolved in the
/// caller (builtin table / OpenRouter aggregate), not by string heuristics here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapabilityOverride {
    pub context_window: Option<usize>,
    pub supports_vision: Option<bool>,
    pub supports_audio: Option<bool>,
    pub supports_image_generation: Option<bool>,
    /// Authoritative effort list (only set when non-empty; `None` = unknown).
    pub reasoning_efforts: Option<Vec<String>>,
    /// Provider-declared default effort (`None` = unknown).
    pub default_effort: Option<String>,
}

/// Capability resolver injected into [`sync_provider_models`].
///
/// Keeping the resolver behind a trait leaves this TOML layer synchronous and
/// network-free: the command layer supplies a builtin + OpenRouter implementation
/// while unit tests supply a fake. `None` = the authority does not know the model
/// at all → every capability field is left untouched.
pub trait CapabilitySource {
    fn resolve(&self, model_id: &str) -> Option<CapabilityOverride>;
}

/// Provider-scoped builtin `ModelDef` → [`CapabilityOverride`].
///
/// Uses the provider-qualified lookup (never the unqualified `find_model`): a
/// same-name model published by a different segment must not leak its metadata
/// into the segment being synced.
pub fn builtin_capability(provider_type: &str, model_id: &str) -> Option<CapabilityOverride> {
    let registry = nuphus::config::registry::ProviderRegistry::builtin();
    let def = registry.find_model_for_provider(provider_type, model_id)?;
    Some(CapabilityOverride {
        context_window: Some(def.context_window as usize),
        supports_vision: Some(def.supports_vision),
        supports_audio: Some(def.supports_audio),
        supports_image_generation: Some(def.supports_image_generation),
        reasoning_efforts: (!def.reasoning_efforts.is_empty()).then(|| {
            def.reasoning_efforts
                .iter()
                .map(|s| s.to_string())
                .collect()
        }),
        default_effort: def.default_effort.map(|s| s.to_string()),
    })
}

/// Builtin-only [`CapabilitySource`] — used by the manual-add path and tests.
pub struct BuiltinCapabilitySource<'a> {
    pub provider_type: &'a str,
}

impl CapabilitySource for BuiltinCapabilitySource<'_> {
    fn resolve(&self, model_id: &str) -> Option<CapabilityOverride> {
        builtin_capability(self.provider_type, model_id)
    }
}

/// Outcome of one [`sync_provider_models`] run — surfaced as the refresh summary
/// (新增 / 更新 / 移除，以及被更新与被移除的 id 供用户知情)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SyncReport {
    /// Official ids appended to the segment.
    pub added: usize,
    /// Existing entries whose capability fields changed under the authority.
    pub updated: usize,
    /// Ids of the updated entries（磁盘顺序），与 `updated` 计数一一对应。
    pub updated_ids: Vec<String>,
    /// Non-manual entries dropped because the official list no longer carries them.
    pub removed: usize,
    /// Ids of the removed entries.
    pub removed_ids: Vec<String>,
    /// Manual entries kept although the official list does not carry them.
    pub kept_manual: usize,
}

/// Reconcile a provider segment's `models` array with the official `/v1/models`
/// catalog.
///
/// For every id in `incoming_ids`:
/// * already on disk → capability fields (`context_window` / `supports_*` /
///   `reasoning_efforts` / `default_effort`) are **overwritten** from `caps`;
///   `alias` / `max_tokens` / `cost_per_million_in|out` are user-authored and
///   preserved. A user vision toggle (`supports_vision_source = "user"`) is
///   never overwritten — explicit user intent outranks the authority chain.
/// * absent → appended with authoritative capabilities and `source = auto`.
///
/// For ids **not** in `incoming_ids`:
/// * `remove_missing == true` and `source != manual` → removed (official names
///   the provider dropped);
/// * `source == manual` → kept (user-added, may live outside the catalog);
/// * `remove_missing == false` → everything kept: the silent auto-sync path
///   never deletes anything.
///
/// An empty `incoming_ids` is a no-op — a hiccupping endpoint must not wipe the
/// segment's model list.
pub fn sync_provider_models(
    config_path: &std::path::Path,
    provider_name: &str,
    incoming_ids: &[String],
    caps: &dyn CapabilitySource,
    remove_missing: bool,
) -> Result<SyncReport, String> {
    sync_provider_models_inner(
        config_path,
        provider_name,
        incoming_ids,
        caps,
        remove_missing,
        false,
    )
}

/// Add (or re-mark) one user-supplied model id as `source = manual`, so an
/// explicit refresh keeps it even when `/v1/models` does not return it. Used by
/// the「添加模型」entry point for grey/temporary models. Capabilities come from
/// the provider-scoped builtin table only (this path has no async context).
pub fn add_provider_model_entry(
    config_path: &std::path::Path,
    provider_name: &str,
    provider_type: &str,
    model_id: &str,
) -> Result<SyncReport, String> {
    let incoming = [model_id.to_string()];
    let caps = BuiltinCapabilitySource { provider_type };
    sync_provider_models_inner(config_path, provider_name, &incoming, &caps, false, true)
}

/// Shared implementation for the sync + manual-add paths. `mark_manual` marks
/// every incoming id as `source = manual` instead of `auto`.
fn sync_provider_models_inner(
    config_path: &std::path::Path,
    provider_name: &str,
    incoming_ids: &[String],
    caps: &dyn CapabilitySource,
    remove_missing: bool,
    mark_manual: bool,
) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    // 空清单 = 异常（接口抖动/解析失败）：不删不清，避免整段模型列表被抹掉。
    if incoming_ids.is_empty() {
        return Ok(report);
    }
    // If file doesn't exist yet, silently skip — creating it is update_config_toml's job
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(report),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(report),
    };

    let mut mutated = false;
    if let Some(providers) = doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        for provider in providers.iter_mut() {
            if provider.get("name").and_then(|n| n.as_str()) != Some(provider_name) {
                continue;
            }
            let map = match provider.as_table_mut() {
                Some(m) => m,
                None => break,
            };
            let (r, m) = reconcile_segment(
                map,
                provider_name,
                incoming_ids,
                caps,
                remove_missing,
                mark_manual,
            );
            report = r;
            mutated = m;
            break;
        }
    }

    if mutated {
        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
        let new_content = toml::to_string_pretty(&doc)
            .map_err(|e| format!("serialize config.toml failed: {}", e))?;
        std::fs::write(config_path, new_content)
            .map_err(|e| format!("write config.toml failed: {}", e))?;
        tracing::info!(
            "sync_provider_models: provider={} added={} updated={} removed={} kept_manual={}",
            provider_name,
            report.added,
            report.updated,
            report.removed,
            report.kept_manual
        );
    }
    Ok(report)
}

/// Rewrite one provider table's `models` array. Returns `(report, mutated)`;
/// `mutated = false` means the array is byte-identical and must not be written.
fn reconcile_segment(
    map: &mut toml::map::Map<String, toml::Value>,
    provider_name: &str,
    incoming_ids: &[String],
    caps: &dyn CapabilitySource,
    remove_missing: bool,
    mark_manual: bool,
) -> (SyncReport, bool) {
    let mut report = SyncReport::default();
    let incoming: std::collections::HashSet<&str> =
        incoming_ids.iter().map(|s| s.as_str()).collect();
    let existing: Vec<toml::Value> = map
        .get("models")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out: Vec<toml::Value> = Vec::with_capacity(existing.len() + incoming_ids.len());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut mutated = false;

    // Pass 1 — reconcile the entries already on disk (disk order preserved).
    for model in existing {
        let mut table = match model {
            toml::Value::Table(t) => t,
            other => {
                out.push(other);
                continue;
            }
        };
        let id = match table.get("id").and_then(|i| i.as_str()) {
            Some(i) => i.to_string(),
            None => {
                // 无 id 的条目不是本引擎产物：原样保留，不猜不改。
                out.push(toml::Value::Table(table));
                continue;
            }
        };
        let is_manual = table.get(MODEL_SOURCE_KEY).and_then(|v| v.as_str())
            == Some(nuphus::config::ModelSource::Manual.as_str());

        if incoming.contains(id.as_str()) {
            seen.insert(id.clone());
            if mark_manual {
                mutated |= set_str(
                    &mut table,
                    MODEL_SOURCE_KEY,
                    nuphus::config::ModelSource::Manual.as_str(),
                );
            }
            if apply_capabilities(&mut table, caps.resolve(&id)) {
                report.updated += 1;
                report.updated_ids.push(id.clone());
                mutated = true;
            }
            out.push(toml::Value::Table(table));
        } else if remove_missing && !is_manual {
            report.removed += 1;
            report.removed_ids.push(id);
            mutated = true;
        } else {
            if is_manual && remove_missing {
                report.kept_manual += 1;
            }
            out.push(toml::Value::Table(table));
        }
    }

    // Pass 2 — append official ids that are not on disk yet (official order).
    for id in incoming_ids {
        if seen.contains(id) {
            continue;
        }
        seen.insert(id.clone());
        let mut entry = toml::map::Map::new();
        entry.insert("id".to_string(), toml::Value::String(id.clone()));
        // 与 ModelEntry 的 serde 默认一致：supports_streaming 默认 true
        entry.insert("supports_streaming".to_string(), toml::Value::Boolean(true));
        apply_capabilities(&mut entry, caps.resolve(id));
        let source = if mark_manual {
            nuphus::config::ModelSource::Manual.as_str()
        } else {
            nuphus::config::ModelSource::Auto.as_str()
        };
        entry.insert(
            MODEL_SOURCE_KEY.to_string(),
            toml::Value::String(source.to_string()),
        );
        out.push(toml::Value::Table(entry));
        report.added += 1;
        mutated = true;
    }

    if !mutated {
        return (report, false);
    }
    tracing::debug!(
        "reconcile_segment: provider={} models_out={}",
        provider_name,
        out.len()
    );
    map.insert("models".to_string(), toml::Value::Array(out));
    (report, true)
}

/// Overwrite the authoritative capability fields of one model entry.
///
/// * `None` override → the model is unknown to the authority: nothing is written.
/// * a `None` field inside an override → that field is unknown: left untouched
///   (never guessed).
/// * `supports_vision_source = "user"` shields `supports_vision` — a manual
///   toggle outranks the authority chain (same contract as the probe path).
///
/// `alias` / `max_tokens` / `cost_per_million_in|out` are user-authored and are
/// never touched here. Returns whether any field changed.
fn apply_capabilities(
    entry: &mut toml::map::Map<String, toml::Value>,
    cap: Option<CapabilityOverride>,
) -> bool {
    let cap = match cap {
        Some(c) => c,
        None => return false,
    };
    let mut changed = false;
    if entry.get(VISION_SOURCE_KEY).and_then(|v| v.as_str()) != Some("user") {
        if let Some(v) = cap.supports_vision {
            changed |= set_bool(entry, "supports_vision", v);
        }
    }
    if let Some(v) = cap.supports_audio {
        changed |= set_bool(entry, "supports_audio", v);
    }
    if let Some(v) = cap.supports_image_generation {
        changed |= set_bool(entry, "supports_image_generation", v);
    }
    if let Some(v) = cap.context_window {
        changed |= set_int(entry, "context_window", v as i64);
    }
    if let Some(efforts) = cap.reasoning_efforts {
        let value = toml::Value::Array(
            efforts
                .iter()
                .map(|s| toml::Value::String(s.clone()))
                .collect(),
        );
        if entry.get("reasoning_efforts") != Some(&value) {
            entry.insert("reasoning_efforts".to_string(), value);
            changed = true;
        }
    }
    if let Some(effort) = cap.default_effort {
        changed |= set_str(entry, "default_effort", &effort);
    }
    changed
}

/// Insert `key = value` when it differs from the stored value.
fn set_bool(entry: &mut toml::map::Map<String, toml::Value>, key: &str, value: bool) -> bool {
    let v = toml::Value::Boolean(value);
    if entry.get(key) == Some(&v) {
        return false;
    }
    entry.insert(key.to_string(), v);
    true
}

/// Insert `key = value` when it differs from the stored value.
fn set_int(entry: &mut toml::map::Map<String, toml::Value>, key: &str, value: i64) -> bool {
    let v = toml::Value::Integer(value);
    if entry.get(key) == Some(&v) {
        return false;
    }
    entry.insert(key.to_string(), v);
    true
}

/// Insert `key = value` when it differs from the stored value.
fn set_str(entry: &mut toml::map::Map<String, toml::Value>, key: &str, value: &str) -> bool {
    let v = toml::Value::String(value.to_string());
    if entry.get(key) == Some(&v) {
        return false;
    }
    entry.insert(key.to_string(), v);
    true
}

/// Clear a provider's model list in config.toml (`[[providers]].models` → []).
///
/// 场景：base_url 变更后，旧模型条目可能在新地址失效（模型代号不存在，
/// 或同名模型能力/价格不同）。仅清空 models 数组，保留 name / provider_type /
/// base_url / api_key 等字段。返回清除的条目数；provider 不存在或列表已空
/// 时返回 0（幂等，不报错）。
pub fn clear_provider_models_in_config_toml(
    config_path: &std::path::Path,
    provider_name: &str,
) -> Result<usize, String> {
    // If file doesn't exist yet, nothing to clear — silently skip
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return Ok(0),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return Ok(0),
    };

    let providers = match doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        Some(p) => p,
        None => return Ok(0),
    };

    for provider in providers.iter_mut() {
        if provider.get("name").and_then(|n| n.as_str()) != Some(provider_name) {
            continue;
        }
        let removed = provider
            .get("models")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if removed == 0 {
            return Ok(0);
        }
        if let Some(map) = provider.as_table_mut() {
            map.insert("models".to_string(), toml::Value::Array(Vec::new()));
        }
        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
        let new_content = toml::to_string_pretty(&doc)
            .map_err(|e| format!("serialize config.toml failed: {}", e))?;
        std::fs::write(config_path, new_content)
            .map_err(|e| format!("write config.toml failed: {}", e))?;
        tracing::info!(
            "clear_provider_models: cleared {} models for provider={}",
            removed,
            provider_name
        );
        return Ok(removed);
    }
    Ok(0)
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

/// Clear a provider's API key from config.toml.
///
/// Sets the matching `[[providers]]` `api_key` to an empty string — the key is
/// effectively removed while the provider entry (name / provider_type / base_url
/// / models) is preserved. Idempotent: unknown providers leave the file
/// untouched and return `Ok(())`.
pub fn clear_provider_api_key_in_config_toml(
    config_path: &std::path::Path,
    provider_name: &str,
) -> Result<(), String> {
    // If file doesn't exist yet, nothing to clear — silently skip
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
                    map.insert("api_key".to_string(), toml::Value::String(String::new()));
                    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
                    let new_content = toml::to_string_pretty(&doc)
                        .map_err(|e| format!("serialize config.toml failed: {}", e))?;
                    std::fs::write(config_path, new_content)
                        .map_err(|e| format!("write config.toml failed: {}", e))?;
                    tracing::info!("Cleared api_key for provider {}", provider_name);
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

    if provider_name == "custom" || provider_name.starts_with("custom-") {
        validate_custom_provider_name(provider_name)?;
        let duplicate = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .map(|providers| {
                providers
                    .iter()
                    .filter(|p| {
                        p.get("name").and_then(|n| n.as_str()) == Some(provider_name)
                            && p.get("provider_type").and_then(|t| t.as_str()) == Some("custom")
                    })
                    .count()
                    > 1
            })
            .unwrap_or(false);
        if duplicate {
            return Err(format!("自定义服务商名称已重复: {provider_name}"));
        }
    }

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
            toml::Value::String(
                if provider_name == "custom" || provider_name.starts_with("custom-") {
                    "custom".to_string()
                } else {
                    provider_name.to_string()
                },
            ),
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

/// Read a provider's base_url from config.toml（用户在界面填写的接口地址）。
///
/// 空串/字段缺失 → `None`（调用方自行回落内置默认）。
/// 同一 provider 存在多段时取第一段带非空地址的条目，与 key 读取口径一致。
pub fn read_provider_base_url_from_config_toml(provider_name: &str) -> Option<String> {
    let config_path = get_config_path()?;
    let content = std::fs::read_to_string(config_path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    let providers = doc.get("providers")?.as_array()?;
    for provider in providers {
        let Some(name) = provider.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        if name != provider_name {
            continue;
        }
        let url = provider
            .get("base_url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .trim();
        if !url.is_empty() {
            return Some(url.to_string());
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

/// Read a provider's request timeout from config.toml (`[[providers]] timeout_secs`).
///
/// 本地端点（本机/局域网推理服务）的实际耗时由本地硬件决定，云端那套 60/90/300s
/// 会把长上下文的提炼掐死。调用方据此实现"下限语义"：本地取
/// `max(该值, LOCAL_TIMEOUT_FLOOR_SECS)`，配置配得更高时以配置为准。
/// 返回 `None` 表示该 provider 没配（走 `ProviderConfig::default_timeout` = 300）。
pub fn read_provider_timeout_secs_from_config_toml(provider_name: &str) -> Option<u64> {
    let config_path = get_config_path()?;
    let content = std::fs::read_to_string(config_path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    let providers = doc.get("providers")?.as_array()?;
    for provider in providers {
        let name = provider.get("name")?.as_str()?;
        if name == provider_name {
            return provider
                .get("timeout_secs")
                .and_then(|t| t.as_integer())
                .and_then(|t| {
                    // 0 视为"未配置"：0 秒超时会立刻失败，不是有效意图
                    u64::try_from(t).ok().filter(|v| *v > 0)
                });
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
    fn sync_appends_new_models_and_keeps_user_authored_fields() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"
base_url = "https://api.deepseek.com"

[[providers.models]]
id = "deepseek-v4-flash"
max_tokens = 32768
supports_streaming = true
supports_vision = true
"#,
        );

        let new_ids = vec![
            "deepseek-v4-flash".to_string(),      // 已存在 → 不重复
            "deepseek-v4-multimodal".to_string(), // 新模型 → 追加
            "deepseek-v4-pro".to_string(),        // 新模型 → 追加
        ];
        // remove_missing = false（静默同步语义）：不删任何条目。
        sync_provider_models(
            &path,
            "deepseek",
            &new_ids,
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            false,
        )
        .unwrap();

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

        // 用户手写字段（max_tokens）不得被覆写
        let existing = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-v4-flash"))
            .unwrap();
        assert_eq!(
            existing.get("max_tokens").and_then(|v| v.as_integer()),
            Some(32768),
            "user-authored max_tokens must be preserved"
        );
        // 能力字段由权威链覆写：builtin deepseek 的该 id（alias 命中）支持视觉。
        assert_eq!(
            existing.get("supports_vision").and_then(|v| v.as_bool()),
            Some(true)
        );

        // 新模型带 supports_streaming=true 默认 + source=auto
        let new_m = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-v4-multimodal"))
            .unwrap();
        assert_eq!(
            new_m.get("supports_streaming").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            new_m.get("source").and_then(|v| v.as_str()),
            Some("auto"),
            "synced new entries are marked auto"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn sync_does_not_touch_other_providers() {
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

        sync_provider_models(
            &path,
            "deepseek",
            &["deepseek-new".to_string()],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            false,
        )
        .unwrap();

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
    fn sync_empty_ids_is_noop_even_when_removing() {
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
        // 空清单 + remove_missing=true 仍必须是 no-op：接口抖动不得清空段内模型。
        let report = sync_provider_models(
            &path,
            "deepseek",
            &[],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            true,
        )
        .unwrap();
        assert_eq!(report.added, 0);
        assert_eq!(report.updated, 0);
        assert_eq!(report.removed, 0);
        assert!(
            report.updated_ids.is_empty(),
            "空清单 no-op 不得记录任何被覆写的 id"
        );
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

    /// 端到端：与本机同形的 deepseek 段（5 条，`deepseek-flash.supports_vision`
    /// 为 false）显式刷新后，集合 = 官方返回集（2 条），存量错值被覆写，官方外
    /// 的旧名被移除。
    #[test]
    fn sync_end_to_end_deepseek_matches_official_catalog() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"
base_url = "https://api.deepseek.com"
reasoning_effort = "max"

[[providers.models]]
context_window = 1000000
id = "deepseek-v4-pro"
max_tokens = 32768
supports_audio = false
supports_image_generation = false
supports_streaming = true
supports_vision = false

[[providers.models]]
context_window = 1000000
id = "deepseek-v4-flash"
max_tokens = 32768
supports_audio = false
supports_image_generation = false
supports_streaming = true
supports_vision = false

[[providers.models]]
context_window = 1000000
id = "deepseek-v4-flash-vision-exp"
max_tokens = 32768
supports_audio = false
supports_image_generation = false
supports_streaming = true
supports_vision = true

[[providers.models]]
context_window = 1000000
id = "deepseek-v4.1-flash-expires-on-0910"
supports_streaming = true
supports_vision = true

[[providers.models]]
context_window = 1000000
id = "deepseek-flash"
supports_streaming = true
supports_vision = false
"#,
        );

        // 官方 /v1/models 实测返回集。
        let official = vec!["deepseek-flash".to_string(), "deepseek-v4-pro".to_string()];
        let report = sync_provider_models(
            &path,
            "deepseek",
            &official,
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            true,
        )
        .unwrap();

        assert_eq!(report.added, 0);
        // deepseek-flash（vision 修正）+ deepseek-v4-pro（efforts 补全）
        assert_eq!(report.updated, 2);
        assert_eq!(report.removed, 3);
        assert_eq!(report.kept_manual, 0);
        assert_eq!(
            report.removed_ids,
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-flash-vision-exp".to_string(),
                "deepseek-v4.1-flash-expires-on-0910".to_string(),
            ],
            "官方清单外的 auto 条目（含旧名）必须被移除"
        );
        assert_eq!(
            report.updated_ids,
            vec!["deepseek-v4-pro".to_string(), "deepseek-flash".to_string()],
            "被覆写能力的条目按磁盘顺序记录（pro 补全 efforts、flash 修正 vision）"
        );

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let deepseek = doc.get("providers").unwrap().as_array().unwrap()[0].clone();
        let models = deepseek.get("models").unwrap().as_array().unwrap();
        assert_eq!(models.len(), 2, "同步后集合 = 官方返回集");

        let flash = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-flash"))
            .unwrap();
        assert_eq!(
            flash.get("supports_vision").and_then(|v| v.as_bool()),
            Some(true),
            "存量错值 supports_vision=false 必须被权威值覆写为 true"
        );

        let pro = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-v4-pro"))
            .unwrap();
        assert_eq!(
            pro.get("max_tokens").and_then(|v| v.as_integer()),
            Some(32768),
            "用户手写 max_tokens 保留"
        );
        assert_eq!(
            pro.get("reasoning_efforts")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2,
            "权威链声明的 reasoning_efforts 覆写落盘"
        );

        // 证据输出：`cargo test … -- --nocapture` 直接看到同步后的段内容。
        println!(
            "[e2e] deepseek segment after explicit sync:\n{}",
            toml::to_string_pretty(&deepseek).unwrap()
        );

        std::fs::remove_file(&path).ok();
    }

    /// 官方清单外的 `source = manual` 条目（用户手动添加的灰度模型）必须保留。
    #[test]
    fn sync_keeps_manual_entries_and_reports_them() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"

[[providers.models]]
id = "deepseek-flash"
supports_streaming = true
supports_vision = false

[[providers.models]]
id = "deepseek-v4.1-flash-expires-on-0910"
source = "manual"
supports_streaming = true
supports_vision = true
"#,
        );

        let report = sync_provider_models(
            &path,
            "deepseek",
            &["deepseek-flash".to_string()],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            true,
        )
        .unwrap();

        assert_eq!(report.removed, 0);
        assert_eq!(report.kept_manual, 1);

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let ids: Vec<&str> = doc.get("providers").unwrap().as_array().unwrap()[0]
            .get("models")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec!["deepseek-flash", "deepseek-v4.1-flash-expires-on-0910"]
        );

        std::fs::remove_file(&path).ok();
    }

    /// 静默自动同步（remove_missing=false）：官方清单内的条目照常覆写能力，
    /// 官方清单外的条目一律保留（只增 + 覆写，绝不删除）。
    #[test]
    fn sync_silent_mode_never_removes_but_overwrites_capabilities() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"

[[providers.models]]
id = "deepseek-flash"
supports_streaming = true
supports_vision = false

[[providers.models]]
id = "deepseek-v4.1-flash-expires-on-0910"
supports_streaming = true
supports_vision = true
"#,
        );

        let report = sync_provider_models(
            &path,
            "deepseek",
            &["deepseek-flash".to_string()],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            false,
        )
        .unwrap();

        assert_eq!(report.removed, 0, "静默同步不得删除条目");
        assert_eq!(report.added, 0);
        assert_eq!(report.updated, 1, "deepseek-flash 的 vision 被覆写");

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let models = doc.get("providers").unwrap().as_array().unwrap()[0]
            .get("models")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(models.len(), 2, "官方外条目在静默模式下保留");
        let flash = models
            .iter()
            .find(|m| m.get("id").and_then(|i| i.as_str()) == Some("deepseek-flash"))
            .unwrap();
        assert_eq!(
            flash.get("supports_vision").and_then(|v| v.as_bool()),
            Some(true)
        );

        std::fs::remove_file(&path).ok();
    }

    /// 旧配置（无 `source` 字段）必须可反序列化，并按 auto 处理（显式刷新可移除）。
    #[test]
    fn legacy_config_without_source_loads_as_auto_and_is_removable() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"

[[providers.models]]
id = "deepseek-v4-flash"
supports_streaming = true
supports_vision = true
"#,
        );

        let registry = nuphus::config::ModelRegistry::from_toml(path.to_str().unwrap()).unwrap();
        assert_eq!(
            registry.providers[0].models[0].source,
            nuphus::config::ModelSource::Auto,
            "缺省 source 必须反序列化为 auto"
        );

        let report = sync_provider_models(
            &path,
            "deepseek",
            &["deepseek-flash".to_string()],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            true,
        )
        .unwrap();
        assert_eq!(report.removed, 1);
        assert_eq!(report.removed_ids, vec!["deepseek-v4-flash".to_string()]);

        std::fs::remove_file(&path).ok();
    }

    /// 行内视觉开关（`supports_vision_source = "user"`）让位于用户意图：
    /// 权威链不得把它覆写回去。
    #[test]
    fn sync_respects_user_vision_override() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"

[[providers.models]]
id = "deepseek-flash"
context_window = 1000000
reasoning_efforts = ["high", "max"]
default_effort = "high"
supports_streaming = true
supports_audio = false
supports_image_generation = false
supports_vision = false
supports_vision_source = "user"
"#,
        );

        let report = sync_provider_models(
            &path,
            "deepseek",
            &["deepseek-flash".to_string()],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            true,
        )
        .unwrap();
        assert_eq!(report.updated, 0, "用户显式设定不得被覆写");
        assert_eq!(report.removed, 0);

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc.get("providers").unwrap().as_array().unwrap()[0]
            .get("models")
            .unwrap()
            .as_array()
            .unwrap()[0]
            .clone();
        assert_eq!(
            entry.get("supports_vision").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            entry.get("supports_vision_source").and_then(|v| v.as_str()),
            Some("user")
        );

        std::fs::remove_file(&path).ok();
    }

    /// `add_provider_model_entry` 写入 `source = manual`，显式刷新也不删。
    #[test]
    fn add_provider_model_entry_marks_manual_and_survives_sync() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"

[[providers.models]]
id = "deepseek-flash"
source = "auto"
supports_streaming = true
"#,
        );

        add_provider_model_entry(
            &path,
            "deepseek",
            "deepseek",
            "deepseek-v4.1-flash-expires-on-0910",
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let added = doc.get("providers").unwrap().as_array().unwrap()[0]
            .get("models")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|m| {
                m.get("id").and_then(|i| i.as_str()) == Some("deepseek-v4.1-flash-expires-on-0910")
            })
            .unwrap()
            .clone();
        assert_eq!(
            added.get("source").and_then(|v| v.as_str()),
            Some("manual"),
            "手动添加必须标记 manual"
        );

        let report = sync_provider_models(
            &path,
            "deepseek",
            &["deepseek-flash".to_string()],
            &BuiltinCapabilitySource {
                provider_type: "deepseek",
            },
            true,
        )
        .unwrap();
        assert_eq!(report.removed, 0);
        assert_eq!(report.kept_manual, 1);

        std::fs::remove_file(&path).ok();
    }

    /// builtin 能力查找必须 provider 限定：同名模型跨段不得串味。
    #[test]
    fn builtin_capability_is_provider_scoped() {
        let ds = builtin_capability("deepseek", "deepseek-v4-flash").unwrap();
        assert_eq!(ds.supports_vision, Some(true));
        assert_eq!(ds.context_window, Some(1_000_000));

        let go = builtin_capability("opencode-go", "deepseek-v4-flash").unwrap();
        assert_eq!(
            go.supports_vision,
            Some(false),
            "opencode-go 段的同名模型能力必须来自本段元数据"
        );

        // 真正不在表中的 id：未知留空，不猜。
        assert!(builtin_capability("deepseek", "no-such-model").is_none());
    }

    #[test]
    fn clear_api_key_empties_key_but_keeps_provider_and_models() {
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

        clear_provider_api_key_in_config_toml(&path, "deepseek").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: toml::Value = content.parse().unwrap();
        let providers = doc.get("providers").unwrap().as_array().unwrap();
        assert_eq!(providers.len(), 1, "provider entry must be preserved");

        let deepseek = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("deepseek"))
            .unwrap();
        // api_key 置空（而非删除字段）：等价于删除，且与 has_key=false 判定一致
        let key = deepseek
            .get("api_key")
            .and_then(|k| k.as_str())
            .unwrap_or("");
        assert!(key.is_empty(), "api_key should be empty after clear");
        assert_eq!(
            deepseek.get("provider_type").and_then(|v| v.as_str()),
            Some("deepseek"),
            "provider_type must be preserved"
        );
        assert_eq!(
            deepseek.get("base_url").and_then(|v| v.as_str()),
            Some("https://api.deepseek.com"),
            "base_url must be preserved"
        );
        let models = deepseek.get("models").unwrap().as_array().unwrap();
        assert_eq!(models.len(), 1, "models must be preserved");
        assert_eq!(
            models[0].get("id").and_then(|v| v.as_str()),
            Some("deepseek-v4-flash")
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn clear_api_key_is_idempotent_for_unknown_provider() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"
"#,
        );
        let before = std::fs::read_to_string(&path).unwrap();

        clear_provider_api_key_in_config_toml(&path, "nonexistent").unwrap();

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            before, after,
            "file must not change when provider is not found"
        );
        std::fs::remove_file(&path).ok();
    }

    /// Custom 实例身份契约：`custom-xxx` 段必须写 name=实例名 / provider_type="custom"，
    /// 否则下游（provider_kind_for_segment、find_model_for_provider）无法把实例名解析回
    /// 自定义协议，同名模型就会串台。
    #[test]
    fn custom_instance_segment_keeps_provider_type_custom() {
        let path = write_temp_config("");
        update_config_toml(
            &path,
            "custom-team-a",
            "sk-test",
            "gpt-4o",
            Some("https://gw.example/v1"),
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let providers = doc.get("providers").and_then(|p| p.as_array()).unwrap();
        let entry = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("custom-team-a"))
            .expect("custom instance segment must be created");
        assert_eq!(
            entry.get("provider_type").and_then(|t| t.as_str()),
            Some("custom"),
            "provider_type 必须是协议类型 custom，而不是实例名"
        );
        assert_eq!(
            entry.get("base_url").and_then(|t| t.as_str()),
            Some("https://gw.example/v1")
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn legacy_custom_segment_is_still_accepted() {
        let path = write_temp_config("");
        assert!(update_config_toml(&path, "custom", "sk-test", "m", Some("https://gw/v1")).is_ok());
        std::fs::remove_file(&path).ok();
    }

    /// 视觉模型绑定必须一次写入两个字段：`vision` 与 `vision_provider`。
    /// 分两次写会留下「新 model + 旧 provider」的中间态 —— 后端按 provider+model
    /// 精确解析时找不到该组合，视觉请求直接失败，而 UI 已提示保存成功。
    #[test]
    fn set_vision_capability_writes_model_and_provider_together() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "custom-a"
provider_type = "custom"
api_key = "sk-a"

[[providers.models]]
id = "gpt-4o"

[[providers]]
name = "custom-b"
provider_type = "custom"
api_key = "sk-b"

[[providers.models]]
id = "gpt-4o"

[capabilities]
vision = "gpt-4o"
vision_provider = "custom-a"
"#,
        );

        set_vision_capability_in_config_toml(&path, "gpt-4o", "custom-b").unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let caps = doc.get("capabilities").unwrap();
        assert_eq!(caps.get("vision").and_then(|v| v.as_str()), Some("gpt-4o"));
        assert_eq!(
            caps.get("vision_provider").and_then(|v| v.as_str()),
            Some("custom-b"),
            "model 与 provider 必须同时指向新实例，杜绝半绑定"
        );
        std::fs::remove_file(&path).ok();
    }

    /// 清除视觉模型时一并清掉 provider 归属，不留悬空引用。
    #[test]
    fn set_vision_capability_clears_provider_with_empty_model() {
        let path = write_temp_config(
            r#"
[capabilities]
vision = "gpt-4o"
vision_provider = "custom-a"
"#,
        );

        set_vision_capability_in_config_toml(&path, "", "").unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let caps = doc.get("capabilities").unwrap();
        assert_eq!(caps.get("vision").and_then(|v| v.as_str()), Some(""));
        assert!(
            caps.get("vision_provider").is_none(),
            "provider 为空时必须清除 vision_provider，避免指向已删除的实例"
        );
        std::fs::remove_file(&path).ok();
    }

    /// 用户手动设定视觉能力必须留痕：否则自动探测会在下次「连接/刷新」时把它
    /// 覆盖回去（用户视角：今天勾上能用，明天又选不到了）。
    #[test]
    fn user_vision_setting_records_source_and_survives_auto_probe() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "custom-team-a"
provider_type = "custom"
api_key = "sk"

[[providers.models]]
id = "gpt-4o"
"#,
        );

        update_model_supports_vision(&path, "custom-team-a", "gpt-4o", true, Some("user")).unwrap();
        assert_eq!(
            read_model_supports_vision(&path, "custom-team-a", "gpt-4o"),
            Some(true)
        );
        assert!(model_has_user_vision_override(
            &path,
            "custom-team-a",
            "gpt-4o"
        ));

        // 自动探测写入（source = None）不得清除 user 标记
        update_model_supports_vision(&path, "custom-team-a", "gpt-4o", true, None).unwrap();
        assert!(
            model_has_user_vision_override(&path, "custom-team-a", "gpt-4o"),
            "自动探测不得清除 user 标记"
        );

        // 未手动设定过的模型不带标记
        assert!(!model_has_user_vision_override(
            &path,
            "custom-team-a",
            "other-model"
        ));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn invalid_custom_instance_names_are_rejected() {
        let path = write_temp_config("");
        for bad in [
            "custom-",
            "custom-Bad",
            "custom-team_A",
            "custom--a",
            "custom-a-",
        ] {
            assert!(
                update_config_toml(&path, bad, "sk-test", "m", Some("https://gw/v1")).is_err(),
                "{bad} 不应通过命名校验"
            );
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn validate_custom_provider_name_accepts_expected_forms() {
        for ok in ["custom", "custom-a", "custom-team-a", "custom-gw2"] {
            assert!(
                validate_custom_provider_name(ok).is_ok(),
                "{ok} 应通过命名校验"
            );
        }
        for bad in [
            "",
            "Custom",
            "custom-",
            "custom-a-",
            "deepseek-a",
            "custom-中",
        ] {
            assert!(
                validate_custom_provider_name(bad).is_err(),
                "{bad} 不应通过命名校验"
            );
        }
    }

    #[test]
    fn read_model_context_window_returns_existing_or_none() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "deepseek"
provider_type = "deepseek"
api_key = "sk-test"
base_url = "https://api.deepseek.com"

[[providers.models]]
id = "deepseek-v4-flash"
context_window = 128000
supports_streaming = true

[[providers.models]]
id = "deepseek-v4-pro"
supports_streaming = true
"#,
        );

        // 有记录 → 返回本地值
        assert_eq!(
            read_model_context_window(&path, "deepseek", "deepseek-v4-flash"),
            Some(128000)
        );
        // 无 context_window 字段 → None（API 才填充）
        assert_eq!(
            read_model_context_window(&path, "deepseek", "deepseek-v4-pro"),
            None
        );
        // 未知 provider/model → None
        assert_eq!(read_model_context_window(&path, "kimi", "k3"), None);
        assert_eq!(
            read_model_context_window(&path, "deepseek", "nope-model"),
            None
        );

        std::fs::remove_file(&path).ok();
    }
}
