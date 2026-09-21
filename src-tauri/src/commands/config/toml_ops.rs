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

/// Whether a segment name is a custom-relay instance (`custom` / `custom-<slug>`).
///
/// **实例身份由段名前缀承载，不由 `provider_type` 承载**：同一批自定义实例里
/// OpenAI 兼容段写 `provider_type = "custom"`、Anthropic 兼容段写 `"anthropic"`，
/// 用协议类型识别实例会把后者漏掉。规则与 `validate_custom_provider_name` 一致。
pub(crate) fn is_custom_segment_name(name: &str) -> bool {
    name == "custom" || name.starts_with("custom-")
}

/// 自定义实例支持的协议类型（与界面「模型提供商」下拉的两项一一对应）。
pub const CUSTOM_PROVIDER_TYPES: [&str; 2] = ["custom", "anthropic"];

/// 编辑入口按段 id 查找失败的文案（界面原样展示）：编辑既有实例的前提是它已落盘。
pub const CUSTOM_PROVIDER_NOT_FOUND: &str = "自定义模型不存在，请先创建";

/// 旧版唯一自定义段名：首建具名实例（custom-<slug>）时接管其数据后删除（升级迁移源段）。
const LEGACY_CUSTOM_SEGMENT: &str = "custom";

/// api_key 落盘编码：空 key（无鉴权端点）写空串，其余走 DPAPI 加密。
///
/// 不能用 `encrypt_secret("")`：空明文经 DPAPI 加密后，`dpapi_decrypt` 因
/// `cbData == 0` 返回 `None`，读回时被判为「无法解密」并打告警 —— 一个无害的
/// 「无鉴权」会被伪装成损坏配置。空串表示与 `clear_provider_api_key_in_config_toml`
/// 完全一致（读回即空 = 不携带鉴权头）。
fn encode_api_key(api_key: &str) -> toml::Value {
    let key = api_key.trim();
    if key.is_empty() {
        toml::Value::String(String::new())
    } else {
        toml::Value::String(nuphus::cookies::encrypt_secret(key))
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
    // 目标段不存在时的统一出口：
    // - 手动添加路径（mark_manual）**必须报错**——此前静默 Ok，前端据此弹「已添加模型」，
    //   用户以为模型加上了，磁盘上其实一行没写（新建实例未落盘时的典型症状）。
    // - 自动刷新路径保持静默：段可能由 update_config_toml 在随后的保存动作里创建。
    let missing_segment = || -> Result<SyncReport, String> {
        if !mark_manual {
            return Ok(SyncReport::default());
        }
        if is_custom_segment_name(provider_name) {
            Err("该自定义模型尚未创建，请先保存基本信息".to_string())
        } else {
            Err(format!(
                "服务商 {provider_name} 尚未保存配置，请先保存基本信息"
            ))
        }
    };
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return missing_segment(),
    };
    let mut doc: toml::Value = match content.parse() {
        Ok(d) => d,
        Err(_) => return missing_segment(),
    };

    let mut mutated = false;
    let mut found = false;
    if let Some(providers) = doc.get_mut("providers").and_then(|p| p.as_array_mut()) {
        for provider in providers.iter_mut() {
            if provider.get("name").and_then(|n| n.as_str()) != Some(provider_name) {
                continue;
            }
            found = true;
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
    if !found {
        return missing_segment();
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
///
/// `provider_type` 只在**新建段**时生效（`None` = 按段名判定：`custom*` 段写
/// `"custom"`，官方段写自身 id）。既有段只更新 `api_key` / `base_url` ——
/// `name` / `display_name` / `provider_type` 是实例身份，改它们属于重命名，
/// 不在本函数的职责内（避免「保存密钥」顺手改掉用户的中转站归属）。
pub fn update_config_toml(
    config_path: &std::path::Path,
    provider_name: &str,
    api_key: &str,
    model_id: &str,
    base_url: Option<&str>,
    provider_type: Option<&str>,
) -> Result<(), String> {
    // Read existing config, or start fresh if file doesn't exist yet
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = content.parse().unwrap_or_else(|_| {
        let mut table = toml::value::Table::new();
        table.insert("providers".to_string(), toml::Value::Array(Vec::new()));
        toml::Value::Table(table)
    });

    if is_custom_segment_name(provider_name) {
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

                // Update api_key（DPAPI 加密落盘；读取端透明解密；空 key 写空串）
                if let Some(map) = provider.as_table_mut() {
                    map.insert("api_key".to_string(), encode_api_key(api_key));
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
        // 协议类型：调用方解析结果优先（自定义实例可能是 anthropic 协议，硬编码
        // "custom" 会把 Anthropic 段写成 OpenAI 协议）；缺省按段名判定。
        let kind = provider_type.unwrap_or(if is_custom_segment_name(provider_name) {
            "custom"
        } else {
            provider_name
        });
        let mut new_provider = toml::value::Table::new();
        new_provider.insert(
            "name".to_string(),
            toml::Value::String(provider_name.to_string()),
        );
        new_provider.insert(
            "provider_type".to_string(),
            toml::Value::String(kind.to_string()),
        );
        new_provider.insert("api_key".to_string(), encode_api_key(api_key));
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
// Custom provider instances（自定义中转站实例）
// ============================================================================

/// OAuth 配置校验（create / update 同源）：三项必填 + 端点必须是 http(s) URL。
///
/// 三态语义（与调用方的清除逻辑配套）：
/// - **三项全空** → `Ok`（= 用户要清除 OAuth 配置，由调用方移除段内 oauth 表）；
/// - **部分填写** → `Err`（半配置存下来必然用不了，当场拦下）；
/// - **齐全** → 端点必须 http(s) 开头。
///
/// 「齐全」的判据与 `oauth_begin` 的 `config_complete()` 保持一致——表单能存下来
/// 的东西必须真的能用来发起授权，否则用户会在点「授权登录」时才撞墙。
fn validate_oauth_config(dto: &super::oauth::OauthConfigDto) -> Result<(), String> {
    let authorize = dto.authorize_url.trim();
    let token = dto.token_url.trim();
    let client = dto.client_id.trim();
    if authorize.is_empty() && token.is_empty() && client.is_empty() {
        return Ok(());
    }
    if authorize.is_empty() || token.is_empty() || client.is_empty() {
        return Err("OAuth 配置不完整：授权端点、令牌端点与 Client ID 均为必填项".to_string());
    }
    for (label, url) in [("授权端点", authorize), ("令牌端点", token)] {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(format!("OAuth {label}必须以 http:// 或 https:// 开头"));
        }
    }
    Ok(())
}

/// 该 DTO 是否表示「清除 OAuth 配置」（三项必填全空）。
fn oauth_dto_is_clear(dto: &super::oauth::OauthConfigDto) -> bool {
    dto.authorize_url.trim().is_empty()
        && dto.token_url.trim().is_empty()
        && dto.client_id.trim().is_empty()
}

/// 段表写入 OAuth 配置：只写配置五项；令牌三字段从 `existing_tokens` 原样搬入
/// （编辑保存保留登录态；清除令牌只能走 oauth_logout）。
/// 入参 DTO 与 `oauth_begin` / create / update 命令共用一份（`super::oauth::OauthConfigDto`）。
fn write_oauth_config(
    segment: &mut toml::value::Table,
    oauth: &super::oauth::OauthConfigDto,
    existing_tokens: Option<&toml::value::Table>,
) {
    let mut table = toml::value::Table::new();
    table.insert(
        "authorize_url".to_string(),
        toml::Value::String(oauth.authorize_url.trim().to_string()),
    );
    table.insert(
        "token_url".to_string(),
        toml::Value::String(oauth.token_url.trim().to_string()),
    );
    table.insert(
        "client_id".to_string(),
        toml::Value::String(oauth.client_id.trim().to_string()),
    );
    let scopes = oauth.scopes.trim();
    if !scopes.is_empty() {
        table.insert(
            "scopes".to_string(),
            toml::Value::String(scopes.to_string()),
        );
    }
    table.insert("use_pkce".to_string(), toml::Value::Boolean(oauth.use_pkce));
    if let Some(port) = oauth.redirect_port {
        table.insert(
            "redirect_port".to_string(),
            toml::Value::Integer(port as i64),
        );
    }
    if let Some(tokens) = existing_tokens {
        for key in ["access_token", "refresh_token", "expires_at"] {
            if let Some(v) = tokens.get(key) {
                table.insert(key.to_string(), v.clone());
            }
        }
    }
    segment.insert("oauth".to_string(), toml::Value::Table(table));
}

/// 旧 `custom` 段升级迁移第一步：从 providers 数组摘除旧段并返回其数据。
/// 不存在旧段（或新段名就是 `custom`）→ `None`，纯新建路径零行为变化。
fn take_legacy_custom_segment(
    providers: &mut Vec<toml::Value>,
    new_name: &str,
) -> Option<toml::Value> {
    if new_name == LEGACY_CUSTOM_SEGMENT {
        return None;
    }
    let idx = providers
        .iter()
        .position(|p| p.get("name").and_then(|n| n.as_str()) == Some(LEGACY_CUSTOM_SEGMENT))?;
    Some(providers.remove(idx))
}

/// 旧 `custom` 段已删除后的引用同步：[last_model] 全表值 + [capabilities] 的
/// provider 归属字段，值 == "custom" → 新段名。`vision` 承载模型 id（不是段名），
/// 显式排除，白名单与 `Capabilities` struct 的 provider 归属字段一一对应。
fn rebind_legacy_custom_refs(doc: &mut toml::Value, new_name: &str) {
    if let Some(last_model) = doc.get_mut("last_model").and_then(|t| t.as_table_mut()) {
        for (_key, value) in last_model.iter_mut() {
            if value.as_str() == Some(LEGACY_CUSTOM_SEGMENT) {
                *value = toml::Value::String(new_name.to_string());
            }
        }
    }
    const PROVIDER_CAPABILITY_FIELDS: [&str; 5] =
        ["vision_provider", "stt", "tts", "voice", "image_generation"];
    if let Some(caps) = doc.get_mut("capabilities").and_then(|t| t.as_table_mut()) {
        for field in PROVIDER_CAPABILITY_FIELDS {
            if caps.get(field).and_then(|v| v.as_str()) == Some(LEGACY_CUSTOM_SEGMENT) {
                caps.insert(field.to_string(), toml::Value::String(new_name.to_string()));
            }
        }
    }
}

/// 自定义标头清洗：key trim 后非空才保留，value 原样；重复 key 后者覆盖
/// （编辑表单全量提交语义：同 key 的新值应生效）。BTreeMap 落盘键序稳定。
/// 命令层回显与落盘共用同一实现（回显即所见）。
pub(crate) fn sanitize_extra_headers(
    headers: &[(String, String)],
) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    for (k, v) in headers {
        let key = k.trim();
        if !key.is_empty() {
            map.insert(key.to_string(), v.clone());
        }
    }
    map
}

/// Create a new custom provider segment:
/// `name` / `display_name` / `provider_type` / `base_url` / `api_key` / `headers`.
///
/// 只新建、不覆盖：段已存在即报错。新建入口是用户显式的一次「创建」动作，
/// 悄悄改写已有实例的地址与密钥比报错危险得多（同名实例各自服务不同中转站）。
///
/// 旧 `custom` 段升级迁移：providers.toml 里存在旧版唯一自定义段 `custom`
/// 且新段名不同时，该段被接管——api_key（用户未填才沿用）/ models /
/// auth_header / auth_prefix / reasoning_effort（display_name 在用户留空时沿用）
/// 迁入新段，旧段删除，[last_model] 与 [capabilities] 中指向旧段的值同步改名；
/// 全部改动在同一份 doc 上完成后一次写回，失败不留半截状态。
///
/// 校验：段名（`validate_custom_provider_name`）、协议类型（custom / anthropic）、
/// 地址非空且非内置文档占位示例。
/// `headers`：自定义标头，空切片 = 不写该键（旧配置语义不变）。
/// `oauth`：Some → 写入授权配置五项（不含令牌字段——它们由登录流程独占维护）。
pub fn create_custom_provider_segment(
    config_path: &std::path::Path,
    name: &str,
    display_name: &str,
    provider_type: &str,
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    oauth: Option<&super::oauth::OauthConfigDto>,
) -> Result<(), String> {
    validate_custom_provider_name(name)?;
    if !CUSTOM_PROVIDER_TYPES.contains(&provider_type) {
        return Err(format!(
            "不支持的模型提供商类型: {provider_type}（可选 custom / anthropic）"
        ));
    }
    let url = base_url.trim();
    if url.is_empty() {
        return Err("请填写模型 API URL".to_string());
    }
    // 内置默认地址是文档占位示例：落盘后请求会打到示例域名，直接拒绝。
    if url.eq_ignore_ascii_case(nuphus::config::providers::custom::PLACEHOLDER_BASE_URL) {
        return Err("请填写真实的模型 API URL".to_string());
    }
    if let Some(dto) = oauth {
        validate_oauth_config(dto)?;
    }
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = content.parse().unwrap_or_else(|_| {
        let mut table = toml::value::Table::new();
        table.insert("providers".to_string(), toml::Value::Array(Vec::new()));
        toml::Value::Table(table)
    });
    if let Some(table) = doc.as_table_mut() {
        if !table.contains_key("providers") {
            table.insert("providers".to_string(), toml::Value::Array(Vec::new()));
        }
    }
    let providers = doc
        .get_mut("providers")
        .and_then(|p| p.as_array_mut())
        .ok_or_else(|| "providers.toml 缺少 providers 数组".to_string())?;
    if providers
        .iter()
        .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
    {
        return Err(format!("已存在同名自定义模型: {name}"));
    }

    // 旧 `custom` 段升级迁移：从数组摘除旧段（同 doc 合并进新段，一次写回）。
    // 同名检查已先行：name == "custom" 且旧段在时上面已报错，走不到这里。
    let legacy = take_legacy_custom_segment(providers, name);

    let shown = display_name.trim();
    let mut segment = toml::value::Table::new();
    segment.insert("name".to_string(), toml::Value::String(name.to_string()));
    // display_name 可选：空则只写段名，读回时回退段名（向后兼容老配置）；
    // 用户留空且旧段有 display_name → 沿用（升级迁移不丢界面显示名）。
    let shown_value = if !shown.is_empty() {
        Some(shown.to_string())
    } else {
        legacy
            .as_ref()
            .and_then(|t| t.get("display_name"))
            .and_then(|d| d.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
    };
    if let Some(display) = shown_value {
        segment.insert("display_name".to_string(), toml::Value::String(display));
    }
    segment.insert(
        "provider_type".to_string(),
        toml::Value::String(provider_type.to_string()),
    );
    segment.insert("base_url".to_string(), toml::Value::String(url.to_string()));
    // api_key 归属：用户填了 → 用用户的（表单必填语义）；用户未填且旧段
    // api_key 非空 → 原样搬运（密文不二次加密，明文交给末尾
    // encrypt_plaintext_provider_keys 统一加密）；都无 → 空串（无鉴权端点）。
    let legacy_key_present = legacy
        .as_ref()
        .and_then(|t| t.get("api_key"))
        .and_then(|v| v.as_str())
        .map(|k| !k.is_empty())
        .unwrap_or(false);
    let key_value = if !api_key.trim().is_empty() {
        encode_api_key(api_key)
    } else if legacy_key_present {
        legacy
            .as_ref()
            .and_then(|t| t.get("api_key"))
            .cloned()
            .unwrap_or_else(|| encode_api_key(""))
    } else {
        encode_api_key("")
    };
    segment.insert("api_key".to_string(), key_value);
    // 旧段模型列表与鉴权/推理配置原样迁入（base_url/provider_type 用户新填，不搬）。
    if let Some(legacy_table) = &legacy {
        for key in ["models", "auth_header", "auth_prefix", "reasoning_effort"] {
            if let Some(v) = legacy_table.get(key) {
                segment.insert(key.to_string(), v.clone());
            }
        }
    }
    // 自定义标头：非空才落盘（嵌套表 [providers.<段>.extra_headers]）；
    // 空切片 = 不写该键，旧配置读回语义不变。
    let extra_headers = sanitize_extra_headers(headers);
    if !extra_headers.is_empty() {
        let mut table = toml::value::Table::new();
        for (k, v) in extra_headers {
            table.insert(k, toml::Value::String(v));
        }
        segment.insert("extra_headers".to_string(), toml::Value::Table(table));
    }
    // OAuth 订阅凭证（可选）：写配置五项；迁移场景从旧段 oauth 表带入已有令牌
    // （旧 custom 段若已登录，升级为具名实例不该丢登录态）。
    // 三项全空 = 不启用（新建态无「清除」可言），不写该键。
    if let Some(cfg) = oauth {
        if !oauth_dto_is_clear(cfg) {
            let legacy_tokens = legacy
                .as_ref()
                .and_then(|t| t.get("oauth"))
                .and_then(|o| o.as_table());
            write_oauth_config(&mut segment, cfg, legacy_tokens);
        }
    }
    providers.push(toml::Value::Table(segment));

    // 旧段已删除：同步指向旧段名的归属记录，生效模型 / 视觉 / 语音不悬空。
    if legacy.is_some() {
        rebind_legacy_custom_refs(&mut doc, name);
    }

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("serialize config.toml failed: {}", e))?;
    std::fs::write(config_path, new_content)
        .map_err(|e| format!("write config.toml failed: {}", e))?;

    tracing::info!(
        "Created custom provider segment: name={}, display_name={}, provider_type={}",
        name,
        shown,
        provider_type
    );
    Ok(())
}

/// Update an existing custom provider segment（重命名 / 换协议 / 改地址 / 换密钥）。
///
/// 与 [`create_custom_provider_segment`] 的分工：段必须**已存在**，这里只改写可变字段。
///
/// - 段 id（`name`）是模型路由依据（同名模型靠「段 id + 模型 ID」精确路由），
///   **绝不修改**：重命名只改 `display_name`，改段 id 会让请求打到别的中转站。
/// - `api_key` 空串 = **保持原 key 不变**（编辑表单留空即「不修改密钥」）；非空才覆盖。
/// - `models` 数组不动：模型列表由 add / refresh / clear 各自维护。
/// - `headers`：自定义标头。空切片 = **清除已存键**（编辑表单全量提交，
///   「删掉所有标头」必须能落盘）；非空 = 覆盖写（key trim 后非空才写，value 原样）。
///
/// 校验与新建同源（段名 / 协议类型 / 地址非空且非占位示例地址），两个入口一条规矩。
pub fn update_custom_provider_segment(
    config_path: &std::path::Path,
    name: &str,
    display_name: &str,
    provider_type: &str,
    base_url: &str,
    api_key: &str,
    headers: &[(String, String)],
    oauth: Option<&super::oauth::OauthConfigDto>,
) -> Result<(), String> {
    validate_custom_provider_name(name)?;
    if !CUSTOM_PROVIDER_TYPES.contains(&provider_type) {
        return Err(format!(
            "不支持的模型提供商类型: {provider_type}（可选 custom / anthropic）"
        ));
    }
    let url = base_url.trim();
    if url.is_empty() {
        return Err("请填写模型 API URL".to_string());
    }
    if url.eq_ignore_ascii_case(nuphus::config::providers::custom::PLACEHOLDER_BASE_URL) {
        return Err("请填写真实的模型 API URL".to_string());
    }
    if let Some(dto) = oauth {
        validate_oauth_config(dto)?;
    }

    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = content.parse().unwrap_or_else(|_| {
        let mut table = toml::value::Table::new();
        table.insert("providers".to_string(), toml::Value::Array(Vec::new()));
        toml::Value::Table(table)
    });
    let providers = doc
        .get_mut("providers")
        .and_then(|p| p.as_array_mut())
        .ok_or_else(|| "providers.toml 缺少 providers 数组".to_string())?;
    let segment = providers
        .iter_mut()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
        .ok_or_else(|| CUSTOM_PROVIDER_NOT_FOUND.to_string())?;
    let map = segment
        .as_table_mut()
        .ok_or_else(|| format!("配置段格式非法: {name}"))?;

    // display_name 只在非空时写：留空 = 读回时回退段名（与新建同一条规则）。
    let shown = display_name.trim();
    if !shown.is_empty() {
        map.insert(
            "display_name".to_string(),
            toml::Value::String(shown.to_string()),
        );
    }
    map.insert(
        "provider_type".to_string(),
        toml::Value::String(provider_type.to_string()),
    );
    map.insert("base_url".to_string(), toml::Value::String(url.to_string()));
    // 空 key = 保持原值：不写入、不覆盖（覆盖成空串等于把已配密钥清掉）。
    if !api_key.trim().is_empty() {
        map.insert("api_key".to_string(), encode_api_key(api_key));
    }
    // 自定义标头：空切片 = 清除已存键（编辑表单全量提交，「删掉所有标头」
    // 必须能落盘）；非空 = 覆盖写，清洗规则与新建一致。
    if headers.is_empty() {
        map.remove("extra_headers");
    } else {
        let mut table = toml::value::Table::new();
        for (k, v) in sanitize_extra_headers(headers) {
            table.insert(k, toml::Value::String(v));
        }
        map.insert("extra_headers".to_string(), toml::Value::Table(table));
    }
    // OAuth 配置三态：Some(齐全) → 覆盖写配置五项、**既有令牌三字段原样保留**
    //（改地址/标头不该把登录态抹掉）；Some(三项全空) → 清除整个 oauth 表
    //（用户切回静态密钥模式的唯一通路：段里留着 oauth 表，凭证解析会一直走 OAuth）；
    // None → 不动（未使用 OAuth 的实例零影响）。
    if let Some(cfg) = oauth {
        if oauth_dto_is_clear(cfg) {
            map.remove("oauth");
        } else {
            let existing_tokens = map.get("oauth").and_then(|o| o.as_table()).cloned();
            write_oauth_config(map, cfg, existing_tokens.as_ref());
        }
    }

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("serialize config.toml failed: {}", e))?;
    std::fs::write(config_path, new_content)
        .map_err(|e| format!("write config.toml failed: {}", e))?;

    tracing::info!(
        "Updated custom provider segment: name={}, display_name={}, provider_type={}, key_updated={}",
        name,
        shown,
        provider_type,
        !api_key.trim().is_empty()
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

/// Read a custom instance's optional `display_name`（界面显示名）。
///
/// 缺失/空串 → `None`：老配置没有这个字段，调用方回退到段名（向后兼容）。
pub fn read_provider_display_name(provider_name: &str) -> Option<String> {
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
        let display = provider
            .get("display_name")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .trim();
        if !display.is_empty() {
            return Some(display.to_string());
        }
    }
    None
}

/// Whether a `[[providers]]` segment with this name exists in config.toml.
///
/// 判据只看「段是否存在」，不看是否配了 key：无鉴权自定义实例（本地网关 /
/// 无 key 中转）同样是登记在册的服务商，不能因为 key 为空就被判为未登记。
pub fn provider_segment_exists(provider_name: &str) -> bool {
    let Some(config_path) = get_config_path() else {
        return false;
    };
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return false;
    };
    let Ok(doc) = content.parse::<toml::Value>() else {
        return false;
    };
    doc.get("providers")
        .and_then(|p| p.as_array())
        .map(|providers| {
            providers
                .iter()
                .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(provider_name))
        })
        .unwrap_or(false)
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
            None,
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
        assert!(
            update_config_toml(&path, "custom", "sk-test", "m", Some("https://gw/v1"), None)
                .is_ok()
        );
        std::fs::remove_file(&path).ok();
    }

    /// 新建自定义实例：四字段一次写全（name / display_name / provider_type /
    /// base_url / api_key），空 key 落盘为空串。
    /// 无鉴权端点（本地网关、Ollama、无 key 中转）必须能建 —— 不能被加密成
    /// 解不开的密文（`encrypt_secret("")` 的 DPAPI 密文读回即「无法解密」）。
    #[test]
    fn create_custom_provider_segment_writes_all_fields() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-my-relay",
            "我的中转站",
            "custom",
            "https://relay.example/v1",
            "",
            &[],
            None,
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let providers = doc.get("providers").and_then(|p| p.as_array()).unwrap();
        assert_eq!(providers.len(), 1, "只应新建一个段");
        let entry = &providers[0];
        assert_eq!(
            entry.get("name").and_then(|v| v.as_str()),
            Some("custom-my-relay")
        );
        assert_eq!(
            entry.get("display_name").and_then(|v| v.as_str()),
            Some("我的中转站"),
            "界面显示名必须落盘（UI 只显示它，不显示段 id）"
        );
        assert_eq!(
            entry.get("provider_type").and_then(|v| v.as_str()),
            Some("custom")
        );
        assert_eq!(
            entry.get("base_url").and_then(|v| v.as_str()),
            Some("https://relay.example/v1")
        );
        assert_eq!(
            entry.get("api_key").and_then(|v| v.as_str()),
            Some(""),
            "空 key 落盘为空串（无鉴权端点）"
        );

        std::fs::remove_file(&path).ok();
    }

    /// Anthropic 兼容实例：协议类型原样落盘。写成 "custom" 会让
    /// `provider_kind_for_segment` 按 OpenAI 协议发 /v1/chat/completions。
    #[test]
    fn create_custom_provider_segment_keeps_anthropic_protocol() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-claude-relay",
            "Claude 中转",
            "anthropic",
            "https://claude-relay.example/v1",
            "sk-test",
            &[],
            None,
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .expect("segment must be created");
        assert_eq!(
            entry.get("provider_type").and_then(|v| v.as_str()),
            Some("anthropic")
        );
        std::fs::remove_file(&path).ok();
    }

    /// 新建入口的输入校验：重名 / 非法协议 / 空地址 / 占位示例地址一律拒绝，
    /// 且拒绝时不得留下半个段。
    #[test]
    fn create_custom_provider_segment_rejects_invalid_input() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-a",
            "A",
            "custom",
            "https://a.example/v1",
            "sk-a",
            &[],
            None,
        )
        .unwrap();

        // 重名：新建是显式动作，不许悄悄改写既有实例
        assert!(create_custom_provider_segment(
            &path,
            "custom-a",
            "A2",
            "custom",
            "https://a2.example/v1",
            "sk-a2",
            &[],
            None,
        )
        .is_err());
        // 非法段名（中文直填段名会被拒 —— 前端必须走 slug 生成）
        assert!(create_custom_provider_segment(
            &path,
            "我的中转站",
            "我的中转站",
            "custom",
            "https://x.example/v1",
            "",
            &[],
            None,
        )
        .is_err());
        // 协议类型必须是下拉里的两项
        assert!(create_custom_provider_segment(
            &path,
            "custom-b",
            "B",
            "openai",
            "https://b.example/v1",
            "",
            &[],
            None,
        )
        .is_err());
        // 地址必填
        assert!(create_custom_provider_segment(
            &path,
            "custom-c",
            "C",
            "custom",
            "   ",
            "",
            &[],
            None,
        )
        .is_err());
        // 内置文档占位示例地址不是可用端点
        assert!(create_custom_provider_segment(
            &path,
            "custom-d",
            "D",
            "custom",
            "https://your-custom-api.com/v1",
            "",
            &[],
            None,
        )
        .is_err());

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(
            doc.get("providers")
                .and_then(|p| p.as_array())
                .map(|a| a.len()),
            Some(1),
            "被拒绝的输入不得留下任何段"
        );
        std::fs::remove_file(&path).ok();
    }

    /// 编辑既有实例：改的是显示名 / 协议 / 地址，**段 id 与 models 一动不动**。
    /// 段 id 是模型路由依据（同名模型靠「段 id + 模型 ID」精确路由），
    /// 重命名若顺手改段 id，请求就会打到别的中转站。
    #[test]
    fn update_custom_provider_segment_renames_without_touching_id_and_models() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "custom-relay"
display_name = "旧名字"
provider_type = "custom"
api_key = ""
base_url = "https://old.example/v1"

[[providers.models]]
id = "gpt-4o"
"#,
        );

        update_custom_provider_segment(
            &path,
            "custom-relay",
            "新名字",
            "anthropic",
            "https://new.example/v1",
            "",
            &[],
            None,
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let providers = doc.get("providers").and_then(|p| p.as_array()).unwrap();
        assert_eq!(providers.len(), 1, "编辑不得新增/删除段");
        let entry = &providers[0];
        assert_eq!(
            entry.get("name").and_then(|v| v.as_str()),
            Some("custom-relay"),
            "段 id 必须稳定：它是模型路由依据"
        );
        assert_eq!(
            entry.get("display_name").and_then(|v| v.as_str()),
            Some("新名字"),
            "重命名只改 display_name"
        );
        assert_eq!(
            entry.get("provider_type").and_then(|v| v.as_str()),
            Some("anthropic"),
            "「模型提供商」下拉可改协议类型"
        );
        assert_eq!(
            entry.get("base_url").and_then(|v| v.as_str()),
            Some("https://new.example/v1")
        );
        assert_eq!(
            entry
                .get("models")
                .and_then(|m| m.as_array())
                .and_then(|a| a.first())
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str()),
            Some("gpt-4o"),
            "models 数组不参与编辑，必须原样保留"
        );
        std::fs::remove_file(&path).ok();
    }

    /// 编辑页「模型 API Key」留空 = 保持原密钥不变（不是清空）。
    /// 非空才覆盖 —— 覆盖后可解密回用户填的新值。
    #[test]
    fn update_custom_provider_segment_keeps_key_when_api_key_empty() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-relay",
            "中继",
            "custom",
            "https://relay.example/v1",
            "sk-old",
            &[],
            None,
        )
        .unwrap();

        let raw_key = |p: &std::path::Path| -> String {
            let doc: toml::Value = std::fs::read_to_string(p).unwrap().parse().unwrap();
            doc.get("providers")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.get("api_key"))
                .and_then(|v| v.as_str())
                .unwrap()
                .to_string()
        };

        let before = raw_key(&path);
        assert_eq!(
            nuphus::cookies::decrypt_secret(&before).as_deref(),
            Some("sk-old")
        );

        // 留空 → 原密钥原封不动（含存储形态：不得被空串覆盖）
        update_custom_provider_segment(
            &path,
            "custom-relay",
            "中继（改名）",
            "custom",
            "https://relay.example/v1",
            "",
            &[],
            None,
        )
        .unwrap();
        assert_eq!(raw_key(&path), before, "留空 API Key 必须保持原密钥");
        assert_eq!(
            nuphus::cookies::decrypt_secret(&raw_key(&path)).as_deref(),
            Some("sk-old")
        );

        // 非空 → 覆盖为新密钥
        update_custom_provider_segment(
            &path,
            "custom-relay",
            "中继（改名）",
            "custom",
            "https://relay.example/v1",
            "sk-new",
            &[],
            None,
        )
        .unwrap();
        assert_eq!(
            nuphus::cookies::decrypt_secret(&raw_key(&path)).as_deref(),
            Some("sk-new"),
            "非空 API Key 才覆盖"
        );
        std::fs::remove_file(&path).ok();
    }

    /// 编辑入口的输入校验：段不存在 / 非法协议 / 空地址 / 占位示例地址一律拒绝，
    /// 且拒绝时磁盘上的既有段保持原样。
    #[test]
    fn update_custom_provider_segment_rejects_invalid_input() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "custom-a"
display_name = "A"
provider_type = "custom"
api_key = ""
base_url = "https://a.example/v1"
"#,
        );

        // 段不存在：编辑的前提是它已落盘
        let err = update_custom_provider_segment(
            &path,
            "custom-ghost",
            "幽灵",
            "custom",
            "https://g.example/v1",
            "",
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err, CUSTOM_PROVIDER_NOT_FOUND);

        // 协议类型必须是下拉里的两项
        assert!(update_custom_provider_segment(
            &path,
            "custom-a",
            "A",
            "openai",
            "https://a.example/v1",
            "",
            &[],
            None,
        )
        .is_err());
        // 地址必填
        assert!(update_custom_provider_segment(
            &path,
            "custom-a",
            "A",
            "custom",
            "  ",
            "",
            &[],
            None,
        )
        .is_err());
        // 内置文档占位示例地址不是可用端点
        assert!(update_custom_provider_segment(
            &path,
            "custom-a",
            "A",
            "custom",
            "https://your-custom-api.com/v1",
            "",
            &[],
            None,
        )
        .is_err());

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .unwrap();
        assert_eq!(
            entry.get("display_name").and_then(|v| v.as_str()),
            Some("A")
        );
        assert_eq!(
            entry.get("base_url").and_then(|v| v.as_str()),
            Some("https://a.example/v1")
        );
        std::fs::remove_file(&path).ok();
    }

    /// 「+ 手动添加」路径下目标段不存在必须报错：此前静默 Ok，前端据此弹
    /// 「已添加模型」而磁盘上什么都没写。自动刷新路径保持静默（官方段常在
    /// 下一次保存时才创建）。
    #[test]
    fn manual_add_reports_missing_segment_but_refresh_stays_silent() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "custom-a"
provider_type = "custom"
api_key = "sk-a"
base_url = "https://a.example/v1"
"#,
        );

        // 段存在 → 正常并入
        let report = add_provider_model_entry(&path, "custom-a", "custom", "gpt-4o").unwrap();
        assert_eq!(report.added, 1);

        // 段不存在 → 报错文案要指向「先保存基本信息」
        let err = add_provider_model_entry(&path, "custom-b", "custom", "gpt-4o").unwrap_err();
        assert!(
            err.contains("尚未创建"),
            "自定义实例缺段时错误应说明尚未创建: {err}"
        );

        // 自动刷新（sync）路径对缺失段保持静默，不改变官方服务商既有行为
        let caps = BuiltinCapabilitySource {
            provider_type: "custom",
        };
        assert!(
            sync_provider_models(&path, "custom-b", &["gpt-4o".to_string()], &caps, false).is_ok()
        );
        assert!(sync_provider_models(&path, "deepseek", &["x".to_string()], &caps, false).is_ok());

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
                update_config_toml(&path, bad, "sk-test", "m", Some("https://gw/v1"), None)
                    .is_err(),
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

    // ── 自定义标头（extra_headers）与旧 custom 段升级迁移 ──

    /// 新建时自定义标头落盘为嵌套表，ModelRegistry 读回 ProviderConfig.extra_headers
    /// 往返一致；key trim 后非空才写（空白 key 被过滤），value 原样保留。
    #[test]
    fn create_custom_provider_segment_roundtrips_extra_headers() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-hdr",
            "带标头实例",
            "custom",
            "https://relay.example/v1",
            "",
            &[
                ("X-Gateway".to_string(), "nuphus".to_string()),
                ("   ".to_string(), "skipped".to_string()),
                ("X-Trace".to_string(), "  padded  ".to_string()),
            ],
            None,
        )
        .unwrap();

        // TOML 落盘形态：段内嵌套表，空白 key 不落盘，value 原样
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .unwrap();
        let headers = entry
            .get("extra_headers")
            .and_then(|h| h.as_table())
            .unwrap();
        assert_eq!(headers.len(), 2, "空白 key 被过滤");
        assert_eq!(
            headers.get("X-Gateway").and_then(|v| v.as_str()),
            Some("nuphus")
        );
        assert_eq!(
            headers.get("X-Trace").and_then(|v| v.as_str()),
            Some("  padded  "),
            "value 原样保留"
        );

        // 读回：ProviderConfig.extra_headers 往返一致（serde default 空路径之外的正常链路）
        let registry = nuphus::config::ModelRegistry::from_toml(path.to_str().unwrap()).unwrap();
        let cfg = registry
            .providers
            .iter()
            .find(|p| p.name == "custom-hdr")
            .expect("custom-hdr segment must load");
        assert_eq!(
            cfg.extra_headers.get("X-Gateway").map(String::as_str),
            Some("nuphus")
        );
        assert_eq!(
            cfg.extra_headers.get("X-Trace").map(String::as_str),
            Some("  padded  ")
        );

        std::fs::remove_file(&path).ok();
    }

    /// 新建 headers 为空切片 → 不写 extra_headers 键（旧配置语义不变）。
    #[test]
    fn create_custom_provider_segment_without_headers_writes_no_key() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-plain",
            "无标头",
            "custom",
            "https://relay.example/v1",
            "",
            &[],
            None,
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .unwrap();
        assert!(
            entry.get("extra_headers").is_none(),
            "空标头不得写出 extra_headers 键"
        );
        // serde default：缺键读回空 map
        let registry = nuphus::config::ModelRegistry::from_toml(path.to_str().unwrap()).unwrap();
        let cfg = registry
            .providers
            .iter()
            .find(|p| p.name == "custom-plain")
            .unwrap();
        assert!(cfg.extra_headers.is_empty());

        std::fs::remove_file(&path).ok();
    }

    /// 旧 `custom` 段升级迁移：首建具名实例时接管旧段——key/models/鉴权配置迁入、
    /// 旧段删除、[last_model] 与 [capabilities] 指向旧段的值同步改名，一次写回。
    #[test]
    fn create_custom_provider_segment_migrates_legacy_custom_segment() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "custom"
provider_type = "custom"
api_key = "sk-legacy-key"
base_url = "https://old.example/v1"
auth_header = "X-Old-Auth"
auth_prefix = "OldScheme "
reasoning_effort = "high"
display_name = "旧中转站"

[[providers.models]]
id = "step-5-preview"
supports_streaming = true

[[providers]]
name = "zhipu"
provider_type = "zhipu"
api_key = "sk-zhipu"
base_url = "https://open.bigmodel.cn/api/paas/v4"

[last_model]
step-5-preview = "custom"
"glm-5.3-flashx" = "zhipu"

[capabilities]
vision_provider = "custom"
vision = "step-5-preview"
tts = "custom"
"#,
        );

        create_custom_provider_segment(
            &path,
            "custom-new",
            "", // 用户留空 display_name → 沿用旧段显示名（不丢界面名称）
            "custom",
            "https://new.example/v1",
            "", // 用户未填 key → 沿用旧段 key（迁移不丢密钥）
            &[],
            None,
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let providers = doc.get("providers").and_then(|p| p.as_array()).unwrap();
        assert_eq!(
            providers.len(),
            2,
            "旧 custom 段被接管删除，只剩新段与无关段"
        );
        assert!(
            !providers
                .iter()
                .any(|p| p.get("name").and_then(|n| n.as_str()) == Some("custom")),
            "旧 custom 段必须消失"
        );
        let new_seg = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("custom-new"))
            .expect("新段必须存在");

        // 用户填的 base_url 生效；旧段 models / 鉴权配置 / reasoning_effort 迁入
        assert_eq!(
            new_seg.get("base_url").and_then(|v| v.as_str()),
            Some("https://new.example/v1"),
            "base_url 用户新填的"
        );
        assert_eq!(
            new_seg
                .get("models")
                .and_then(|m| m.as_array())
                .and_then(|a| a.first())
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str()),
            Some("step-5-preview"),
            "旧段 models 必须迁入新段"
        );
        assert_eq!(
            new_seg.get("auth_header").and_then(|v| v.as_str()),
            Some("X-Old-Auth")
        );
        assert_eq!(
            new_seg.get("auth_prefix").and_then(|v| v.as_str()),
            Some("OldScheme ")
        );
        assert_eq!(
            new_seg.get("reasoning_effort").and_then(|v| v.as_str()),
            Some("high")
        );
        // 用户留空 display_name → 沿用旧段显示名
        assert_eq!(
            new_seg.get("display_name").and_then(|v| v.as_str()),
            Some("旧中转站"),
            "用户留空 display_name 时沿用旧段显示名"
        );

        // key 沿用旧段（读回经 decrypt_secret 还原）
        let raw_key = new_seg.get("api_key").and_then(|v| v.as_str()).unwrap();
        assert_eq!(
            nuphus::cookies::decrypt_secret(raw_key).as_deref(),
            Some("sk-legacy-key"),
            "用户未填 key 时必须沿用旧段密钥"
        );

        // [last_model]：指向旧段的值改名，无关条目不动
        let last_model = doc.get("last_model").and_then(|t| t.as_table()).unwrap();
        assert_eq!(
            last_model.get("step-5-preview").and_then(|v| v.as_str()),
            Some("custom-new"),
            "生效模型归属同步改名"
        );
        assert_eq!(
            last_model.get("glm-5.3-flashx").and_then(|v| v.as_str()),
            Some("zhipu"),
            "无关条目不动"
        );

        // [capabilities]：provider 归属字段改名；vision 是模型 id 字段，不改
        let caps = doc.get("capabilities").and_then(|t| t.as_table()).unwrap();
        assert_eq!(
            caps.get("vision_provider").and_then(|v| v.as_str()),
            Some("custom-new")
        );
        assert_eq!(caps.get("tts").and_then(|v| v.as_str()), Some("custom-new"));
        assert_eq!(
            caps.get("vision").and_then(|v| v.as_str()),
            Some("step-5-preview"),
            "vision 承载模型 id，不是段名，不改"
        );

        // 迁移后新段可被 ModelRegistry 正常加载（模型归属仍可解析）
        let registry = nuphus::config::ModelRegistry::from_toml(path.to_str().unwrap()).unwrap();
        assert!(registry.providers.iter().any(|p| p.name == "custom-new"));
        assert!(
            registry.providers.iter().all(|p| p.name != "custom"),
            "加载后的 registry 不应再有旧段"
        );

        std::fs::remove_file(&path).ok();
    }

    /// 无旧 custom 段时纯新建：迁移路径零行为变化（摘取函数返回 None）。
    #[test]
    fn create_custom_provider_segment_without_legacy_segment_is_pure_create() {
        let path = write_temp_config(
            r#"
[[providers]]
name = "zhipu"
provider_type = "zhipu"
api_key = "sk-zhipu"
base_url = "https://open.bigmodel.cn/api/paas/v4"
"#,
        );
        create_custom_provider_segment(
            &path,
            "custom-fresh",
            "全新实例",
            "custom",
            "https://fresh.example/v1",
            "sk-fresh",
            &[],
            None,
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let providers = doc.get("providers").and_then(|p| p.as_array()).unwrap();
        assert_eq!(providers.len(), 2, "只新增新段，不动无关段");
        let fresh = providers
            .iter()
            .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("custom-fresh"))
            .expect("新段必须存在");
        assert!(
            fresh.get("models").is_none(),
            "无旧段时新段不得凭空带出 models"
        );

        std::fs::remove_file(&path).ok();
    }

    /// 编辑表单标头语义：空切片清除已存键；非空覆盖写（与新建同一条清洗规则）。
    #[test]
    fn update_custom_provider_segment_headers_clear_and_overwrite() {
        let path = write_temp_config("");
        create_custom_provider_segment(
            &path,
            "custom-hdr",
            "带标头实例",
            "custom",
            "https://relay.example/v1",
            "",
            &[("X-Old".to_string(), "1".to_string())],
            None,
        )
        .unwrap();

        // 非空 → 覆盖写（旧键被整组替换，空白 key 过滤）
        update_custom_provider_segment(
            &path,
            "custom-hdr",
            "带标头实例",
            "custom",
            "https://relay.example/v1",
            "",
            &[
                ("X-New".to_string(), "2".to_string()),
                (" ".to_string(), "dropped".to_string()),
            ],
            None,
        )
        .unwrap();
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .unwrap();
        let headers = entry
            .get("extra_headers")
            .and_then(|h| h.as_table())
            .unwrap();
        assert_eq!(headers.len(), 1, "旧键整组替换，空白 key 过滤");
        assert_eq!(headers.get("X-New").and_then(|v| v.as_str()), Some("2"));
        assert!(headers.get("X-Old").is_none());

        // 空切片 → 清除已存键（「删掉所有标头」必须能落盘）
        update_custom_provider_segment(
            &path,
            "custom-hdr",
            "带标头实例",
            "custom",
            "https://relay.example/v1",
            "",
            &[],
            None,
        )
        .unwrap();
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let entry = doc
            .get("providers")
            .and_then(|p| p.as_array())
            .and_then(|a| a.first())
            .unwrap();
        assert!(
            entry.get("extra_headers").is_none(),
            "空标头编辑后 extra_headers 键必须被清除"
        );

        std::fs::remove_file(&path).ok();
    }
}
