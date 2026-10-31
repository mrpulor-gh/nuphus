//! Best-effort capability discovery. Local activation never awaits this work.
//! Lock order: providers config -> runtime. Neither lock may cross an await.

use super::{llm, toml_ops};
use crate::models::aggregator;
use crate::state::{AppState, LlamaConfig, RuntimeContext};
use nuphus::config::{lock_provider_config, registry::ProviderRegistry};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;
use tauri::{Emitter, Manager};

#[derive(Clone, Hash, PartialEq, Eq)]
struct JobKey {
    provider: String,
    model: String,
    connection: u64,
}

#[derive(Clone, Default)]
struct Discovery {
    context: Option<usize>,
    vision: Option<bool>,
    efforts: Vec<String>,
    default_effort: Option<String>,
    /// provider 上报的模型自然最大输出（`/models` 元数据，缺则聚合库目录条目）。
    max_output: Option<u32>,
}

pub(crate) struct MetadataTasks {
    // A repeated selection subscribes the latest generation to the same job.
    pending: HashMap<JobKey, (u64, bool)>,
    cache: HashMap<JobKey, Discovery>,
    permits: std::sync::Arc<tokio::sync::Semaphore>,
}

impl Default for MetadataTasks {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            cache: HashMap::new(),
            permits: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}

impl MetadataTasks {
    fn subscribe(&mut self, key: &JobKey, generation: u64, vision: bool) -> bool {
        if let Some(pending) = self.pending.get_mut(key) {
            pending.0 = generation;
            pending.1 |= vision;
            false
        } else {
            self.pending.insert(key.clone(), (generation, vision));
            true
        }
    }

    fn finish(&mut self, key: &JobKey, result: Discovery) -> Option<u64> {
        // Bound process-local cache growth; credentials/fingerprints are never logged.
        if self.cache.len() >= 128 {
            self.cache.clear();
        }
        self.cache.insert(key.clone(), result);
        self.pending.remove(key).map(|pending| pending.0)
    }
}

fn read_doc(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

fn segment<'a>(doc: &'a toml::Value, provider: &str) -> Option<&'a toml::Value> {
    doc.get("providers")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(provider))
}

fn model_entry<'a>(segment: &'a toml::Value, model: &str) -> Option<&'a toml::Value> {
    segment
        .get("models")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(model))
}

fn context(entry: Option<&toml::Value>) -> Option<usize> {
    entry?
        .get("context_window")?
        .as_integer()
        .filter(|n| *n > 0)
        .map(|n| n as usize)
}

/// providers.toml 里该条目已落盘的 `max_output_tokens`（provider 上报事实）。
/// 与 [`context`] 同口径：非正数/缺字段/条目不存在一律视为「未采集」。
fn output(entry: Option<&toml::Value>) -> Option<u32> {
    entry?
        .get("max_output_tokens")?
        .as_integer()
        .filter(|n| *n > 0 && *n <= u32::MAX as i64)
        .map(|n| n as u32)
}

/// 模型条目是否已声明 effort 档位（providers.toml 侧）。空数组 / 缺字段 /
/// 条目不存在一律视为「未声明」——发现任务据此决定要不要探测。
fn has_efforts(entry: Option<&toml::Value>) -> bool {
    entry
        .and_then(|m| m.get("reasoning_efforts"))
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
}

// Ignore model metadata and display labels, but invalidate for endpoint, protocol,
// headers, credentials and OAuth changes. Decrypt before hashing: DPAPI can encode
// the same key differently on each save. This fingerprint stays in memory only.
fn connection(segment: &toml::Value, cfg: &LlamaConfig) -> Option<u64> {
    let mut fields = segment.as_table()?.clone();
    fields.remove("models");
    fields.remove("display_name");
    if let Some(raw) = fields.get("api_key").and_then(|v| v.as_str()) {
        let plain = nuphus::cookies::decrypt_secret(raw).unwrap_or_default();
        fields.insert("api_key".into(), toml::Value::String(plain));
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    toml::to_string(&fields).ok()?.hash(&mut hash);
    cfg.base_url.hash(&mut hash);
    cfg.api_key.hash(&mut hash);
    Some(hash.finish())
}

fn builtin_context(cfg: &LlamaConfig) -> Option<usize> {
    ProviderRegistry::builtin()
        .get(&cfg.provider)
        .and_then(|p| {
            p.models()
                .iter()
                .find(|m| m.id == cfg.model)
                .map(|m| m.context_window as usize)
        })
        .filter(|n| *n > 0)
}

/// providers.toml 里该 (provider, model) 条目的 `context_window`（**用户配置值**）。
///
/// 供 `get_context_limit` 在 runtime 尚未校准（重启后常见：启动探测失败 → 0）时回退，
/// 把「用户/模型页里明明写着 1.0M」的窗口还给 UI。
/// 仍缺失 → None（调用方继续报 0 = 未知，**不伪造 128000**）。
pub(super) fn configured_context_window(path: &Path, cfg: &LlamaConfig) -> Option<usize> {
    let doc = read_doc(path)?;
    let seg = segment(&doc, &cfg.provider)?;
    context(model_entry(seg, &cfg.model))
}

/// Only local work: explicit/persisted values win, builtin is runtime-only, 0 is unknown.
pub(super) fn activate(
    state: &AppState,
    cfg: &LlamaConfig,
    explicit: Option<usize>,
) -> Result<u64, String> {
    if let Some(value) = explicit {
        toml_ops::update_model_context_window(
            &state.llm_config_path,
            &cfg.provider,
            &cfg.model,
            value,
        )?;
    }
    let _config_write = lock_provider_config();
    let doc = read_doc(&state.llm_config_path);
    let existing = doc
        .as_ref()
        .and_then(|d| segment(d, &cfg.provider))
        .and_then(|p| context(model_entry(p, &cfg.model)));
    let window = explicit
        .or(existing)
        .or_else(|| builtin_context(cfg))
        .unwrap_or(0);
    let mut runtime = state.runtime.lock().map_err(|e| e.to_string())?;
    runtime.model_generation = runtime.model_generation.wrapping_add(1);
    runtime.llm_config = Some(cfg.clone());
    runtime.model_context_window = window;
    runtime.model_context_explicit = explicit;
    Ok(runtime.model_generation)
}

fn is_current(runtime: &RuntimeContext, cfg: &LlamaConfig, generation: u64) -> bool {
    runtime.model_generation == generation
        && runtime.llm_config.as_ref().is_some_and(|current| {
            current.provider == cfg.provider
                && current.model == cfg.model
                && current.base_url == cfg.base_url
                && current.api_key == cfg.api_key
        })
}

pub(super) fn schedule<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cfg: LlamaConfig,
    generation: u64,
) {
    schedule_inner(app, cfg, generation, true);
}

/// Startup historically calibrated context only; it must not send a new paid
/// vision request. A later user switch can subscribe to/upgrade this same job.
pub(super) fn schedule_context_calibration<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cfg: LlamaConfig,
    generation: u64,
) {
    schedule_inner(app, cfg, generation, false);
}

fn schedule_inner<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cfg: LlamaConfig,
    generation: u64,
    vision: bool,
) {
    let state = app.state::<AppState>();
    let snapshot = {
        let _config_write = lock_provider_config();
        read_doc(&state.llm_config_path).and_then(|doc| segment(&doc, &cfg.provider).cloned())
    };
    let Some(snapshot) = snapshot else { return };
    let Some(connection) = connection(&snapshot, &cfg) else {
        return;
    };
    let key = JobKey {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        connection,
    };
    let (cached, permits) = {
        let Ok(mut runtime) = state.runtime.lock() else {
            return;
        };
        if !is_current(&runtime, &cfg, generation) {
            return;
        }
        if !runtime.metadata_tasks.subscribe(&key, generation, vision) {
            return;
        }
        (
            runtime
                .metadata_tasks
                .cache
                .get(&key)
                .cloned()
                .unwrap_or_default(),
            runtime.metadata_tasks.permits.clone(),
        )
    };
    let path = state.llm_config_path.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(_permit) = permits.acquire_owned().await else {
            return;
        };
        // Discard queued work for selections that are no longer current.
        {
            let state = app.state::<AppState>();
            let Ok(mut runtime) = state.runtime.lock() else {
                return;
            };
            let latest = runtime
                .metadata_tasks
                .pending
                .get(&key)
                .map(|pending| pending.0);
            if !latest.is_some_and(|generation| is_current(&runtime, &cfg, generation)) {
                runtime.metadata_tasks.pending.remove(&key);
                return;
            }
        }
        let mut result = discover_capabilities(&cfg, &snapshot, &path, cached).await;
        let state = app.state::<AppState>();
        let vision = state
            .runtime
            .lock()
            .ok()
            .and_then(|runtime| {
                runtime
                    .metadata_tasks
                    .pending
                    .get(&key)
                    .filter(|pending| is_current(&runtime, &cfg, pending.0))
                    .map(|pending| pending.1)
            })
            .unwrap_or(false);
        if vision {
            result = discover_vision(&cfg, &snapshot, &path, result).await;
        }
        let latest = state
            .runtime
            .lock()
            .ok()
            .and_then(|mut runtime| runtime.metadata_tasks.finish(&key, result.clone()));
        if let Some(generation) = latest {
            match commit(&state, &cfg, &snapshot, &key, generation, &result) {
                Ok(true) => {
                    let _ = app.emit(
                        "model-metadata-updated",
                        serde_json::json!({
                            "provider": cfg.provider, "model": cfg.model,
                        }),
                    );
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("[model-metadata] could not persist discovery: {error}")
                }
            }
        }
    });
}

/// 能力发现（激活时后台跑一次）：上下文窗口 + 推理强度档位**各自独立补缺**。
/// 为什么解耦：旧实现把 effort 发现挂在「context 缺失」条件上——context 一旦
/// 落盘（绝大多数模型首次激活即补写），effort 就再也不探测，档位旋钮长期空转。
/// 现在任缺其一即触发 /models 探测；探测与聚合库只**填空**，不覆写已有值。
async fn discover_capabilities(
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    path: &Path,
    mut result: Discovery,
) -> Discovery {
    if cfg.provider == "local" {
        return result;
    }
    let protocol = snapshot
        .get("provider_type")
        .and_then(|v| v.as_str())
        .unwrap_or(&cfg.provider);
    let registry = ProviderRegistry::builtin();
    let (header, prefix) = registry
        .get(protocol)
        .map(|p| (p.auth_header().to_string(), p.auth_prefix().to_string()))
        .unwrap_or(("authorization".into(), "Bearer ".into()));
    let entry = model_entry(snapshot, &cfg.model);
    let needs_context = context(entry).is_none() && result.context.is_none();
    let needs_efforts = !has_efforts(entry) && result.efforts.is_empty();
    // 输出上限与 context / efforts 同层：已落盘就不再探测（探测只填空，见 merge）。
    let needs_output = output(entry).is_none() && result.max_output.is_none();
    if needs_context || needs_efforts || needs_output {
        // `protocol` 是 snapshot 的借用，spawn_blocking 需要 'static → 取一份 owned。
        let (probe_cfg, header, prefix, protocol_owned) = (
            cfg.clone(),
            header.clone(),
            prefix.clone(),
            protocol.to_string(),
        );
        if let Ok(Some(meta)) = tokio::task::spawn_blocking(move || {
            llm::query_model_metadata_from_api(
                &probe_cfg.base_url,
                &probe_cfg.model,
                &probe_cfg.api_key,
                &header,
                &prefix,
                &protocol_owned,
            )
        })
        .await
        {
            if result.context.is_none() {
                result.context = meta.context_length.filter(|n| *n > 0);
            }
            if result.max_output.is_none() {
                // 只认上游上报的正整数；None = 该 provider 不暴露，交给聚合库兜底。
                result.max_output = meta.max_output_tokens.filter(|n| *n > 0);
            }
            if result.efforts.is_empty() {
                result.efforts = meta.reasoning_efforts;
                result.default_effort = meta.default_effort;
            }
        }
        // ② 内置 ModelDef（编译期官方表，逐模型引用官方文档）。两层作用：
        //    补「上游裸 /models 探测不到」的官方档位；刷新旧版本写入的存量值
        //    （providers.toml 里 unchanged-since-dispatch 的 auto 数据可被覆写，
        //    用户手改值有 merge 的漂移护栏保护）。按 (provider_type, 精确 id)
        //    匹配——custom/local 的占位模型（custom-model 等）永不命中真实 id。
        if result.efforts.is_empty() {
            if let Some(p) = registry.get(protocol) {
                if let Some(m) = p.models().iter().find(|m| m.id == cfg.model) {
                    if !m.reasoning_efforts.is_empty() {
                        result.efforts =
                            m.reasoning_efforts.iter().map(|s| s.to_string()).collect();
                        result.default_effort = m.default_effort.map(String::from);
                    }
                }
            }
        }
        // ③ OpenRouter 聚合库（权威目录）。context / efforts / 输出上限共用同一
        // provider-aware 口径：官方段优先 vendor 精确匹配；custom/opencode-go
        // 等中转段走 lookup_generic 的唯一厂商护栏。此前 context 只对
        // has_vendor 生效，导致连接链已有 1.1M、后台校准却仍为 None。
        let want_context = result.context.is_none();
        let want_output = result.max_output.is_none();
        if want_context || want_output || result.efforts.is_empty() {
            if let Some(dir) = path.parent() {
                let entries = aggregator::ensure_cache(&aggregator::cache_path(dir)).await;
                let vendor_lookup = || aggregator::lookup(&entries, &cfg.provider, &cfg.model);
                let generic_lookup = || aggregator::lookup_generic(&entries, &cfg.model);
                if want_context {
                    let lookup = if aggregator::has_vendor(&cfg.provider) {
                        vendor_lookup().or_else(&generic_lookup)
                    } else {
                        generic_lookup()
                    };
                    result.context = lookup
                        .and_then(|entry| entry.context_length)
                        .map(|n| n as usize)
                        .filter(|n| *n > 0);
                }
                if want_output {
                    // 目录条目的 `top_provider.max_completion_tokens`：官方
                    // `/models` 不暴露输出上限时的权威兜底（同一调用位、同一
                    // vendor 护栏）。永不写 0 / 溢出值。
                    let lookup = if aggregator::has_vendor(&cfg.provider) {
                        vendor_lookup().or_else(&generic_lookup)
                    } else {
                        generic_lookup()
                    };
                    result.max_output = lookup
                        .and_then(|entry| entry.max_completion_tokens)
                        .and_then(|n| u32::try_from(n).ok())
                        .filter(|n| *n > 0);
                }
                if result.efforts.is_empty() {
                    let lookup = vendor_lookup().or_else(generic_lookup);
                    if let Some(entry) = lookup {
                        result.efforts = entry.supported_efforts.clone();
                        result.default_effort = result
                            .default_effort
                            .or_else(|| entry.default_effort.clone());
                    }
                }
            }
        }
        // ④ 输出上限探测兜底（A2）：/models 与聚合库都拿不到上限、且确实缺这个
        // 值时，对 Anthropic 协议段发一次「明显超限」的极短请求，从 400 文案里
        // 解析真实 ceiling。探测只填空（交给 merge 落盘，source=api），失败/
        // 端点不校验一律只 warn——绝不影响连接结论，也绝不写静态猜值。
        if needs_output && result.max_output.is_none() && protocol == "anthropic" {
            // 段级自定义标头随探测带上：配了网关标头的中转站，标头缺失会让探测
            // 必败（只 warn、不产错值——与连接测试同一契约，见 llm.rs 的 headers
            // 参数注释）。
            let extra_headers = segment_extra_headers(snapshot);
            if let Some(ceiling) = probe_max_output_tokens(cfg, extra_headers).await {
                result.max_output = Some(ceiling);
            }
        }
    }
    result
}

/// 段快照里的 `[providers.<段>.extra_headers]` 自定义标头（字符串值；非字符串
/// 值跳过）。探测 / 连接链与真实请求必须带同一套标头，否则配了网关标头的中转
/// 站会必败。
fn segment_extra_headers(snapshot: &toml::Value) -> std::collections::BTreeMap<String, String> {
    snapshot
        .get("extra_headers")
        .and_then(|v| v.as_table())
        .map(|table| {
            table
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// 输出上限探测（A2 兜底）的超时（秒）。与连接测试同量级；超时只 warn。
const MAX_OUTPUT_PROBE_TIMEOUT_SECS: u64 = 15;

/// 探测用的「明显超限」`max_tokens`：任何真实模型的输出上限都远小于它，从而
/// 把 Anthropic 的 400 ceiling 文案逼出来（`max_tokens: X > Y, which is the
/// maximum allowed number of output tokens for <model>`）。
const MAX_OUTPUT_PROBE_SENTINEL: u32 = 1_000_000;

/// 从 Anthropic 的 400 错误文案解析真实输出上限（纯函数，供单测）。
///
/// 只认 `max_tokens: <req> > <ceiling>` 形态，取 `>` 后第一个数字（ceiling）：
/// 模型名尾巴（`... for claude-sonnet-4-20250514`）里的数字不是上限。非 400 /
/// 无关 400 / 无比较结构 → `None`——宁可探测不出，也不要一个错的官方感数字
/// （与 `CTX_KEYS` / `OUTPUT_KEYS` 同一条铁律）。
fn ceiling_from_api_error(status: u16, body: &str) -> Option<u32> {
    if status != 400 {
        return None;
    }
    let marker = "max_tokens:";
    // marker / '>' 均为 ASCII：字节偏移即字符边界，切片安全。
    let rest = body.get(body.find(marker)? + marker.len()..)?;
    let tail = rest.get(rest.find('>')? + 1..)?;
    tail.split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse::<u32>()
        .ok()
        // 一致性护栏：探测发出去的 sentinel 都被判超限，回过来的 ceiling 必然
        // 小于它；>= sentinel 的「上限」与 400 事实矛盾，按垃圾值丢弃。
        .filter(|n| *n < MAX_OUTPUT_PROBE_SENTINEL)
}

/// 输出上限探测兜底（A2）：调用方已按协议门控（仅 Anthropic 段）且确认
/// `/models` 与聚合库都没采到上限。
///
/// 经 Provider 自带 `transport()` 发一个 `max_tokens` 明显超限的极短请求
/// （端点 / 鉴权 / 段级标头全部复用官方实现，不另写一套 HTTP）：
/// - `400` 且文案带 `max_tokens: X > Y` → 采到真实 ceiling `Y`；
/// - 成功响应 → 该端点不校验 max_tokens，上限不可知 → `None`；
/// - 其余（网络错 / 超时 / 无关 400）→ 只 warn，`None`。
///
/// 探测结果由调用方写进 `Discovery.max_output`，经既有 merge 落盘
/// （只填空，`source=api`）。本函数不碰任何连接结论。
async fn probe_max_output_tokens(
    cfg: &LlamaConfig,
    extra_headers: std::collections::BTreeMap<String, String>,
) -> Option<u32> {
    let registry = ProviderRegistry::builtin();
    let provider = registry.get("anthropic")?;
    let provider_cfg = nuphus::config::ProviderConfig {
        name: cfg.provider.clone(),
        provider_type: nuphus::config::KnownProvider::from_id(&cfg.provider)
            .unwrap_or(nuphus::config::KnownProvider::Custom),
        api_key: cfg.api_key.clone(),
        base_url: cfg.base_url.clone(),
        auth_header: provider.auth_header().to_string(),
        auth_prefix: provider.auth_prefix().to_string(),
        timeout_secs: MAX_OUTPUT_PROBE_TIMEOUT_SECS,
        models: vec![],
        reasoning_effort: None,
        extra_headers,
        oauth: None,
    };
    let transport = provider.transport(&provider_cfg, &cfg.model);
    // 极短请求：单轮 "hi"，max_tokens 明显超限。input_tokens_hint 不设——
    // 探测要的正是「服务端眼中的上限」，不该被窗口余量 clamp 改写。
    let request = nuphus::api::MessageRequest::new(
        cfg.model.clone(),
        vec![serde_json::json!({ "role": "user", "content": "hi" })],
    )
    .with_max_tokens(MAX_OUTPUT_PROBE_SENTINEL);

    match tokio::time::timeout(
        Duration::from_secs(MAX_OUTPUT_PROBE_TIMEOUT_SECS),
        transport.stream(request),
    )
    .await
    {
        // 端点接受超限 max_tokens → 它不校验，无从得知 ceiling → 不写值
        Ok(Ok(_)) => {
            tracing::info!(
                "[max-output-probe] model={} accepted an oversized max_tokens; \
                 ceiling stays unknown",
                cfg.model
            );
            None
        }
        Ok(Err(nuphus::NuphusError::LLM(nuphus::LLMError::ApiError { status, body }))) => {
            match ceiling_from_api_error(status, &body) {
                Some(ceiling) => {
                    tracing::info!(
                        "[max-output-probe] model={} ceiling={} parsed from HTTP {} body",
                        cfg.model,
                        ceiling,
                        status
                    );
                    Some(ceiling)
                }
                None => {
                    tracing::warn!(
                        "[max-output-probe] model={} HTTP {} body carries no max_tokens \
                         ceiling; output cap stays unknown",
                        cfg.model,
                        status
                    );
                    None
                }
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(
                "[max-output-probe] model={} request failed: {}",
                cfg.model,
                e
            );
            None
        }
        Err(_elapsed) => {
            tracing::warn!(
                "[max-output-probe] model={} timed out after {}s",
                cfg.model,
                MAX_OUTPUT_PROBE_TIMEOUT_SECS
            );
            None
        }
    }
}

async fn discover_vision(
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    path: &Path,
    mut result: Discovery,
) -> Discovery {
    if cfg.provider == "local" {
        return result;
    }
    let protocol = snapshot
        .get("provider_type")
        .and_then(|v| v.as_str())
        .unwrap_or(&cfg.provider);
    let registry = ProviderRegistry::builtin();
    let (header, prefix) = registry
        .get(protocol)
        .map(|p| (p.auth_header().to_string(), p.auth_prefix().to_string()))
        .unwrap_or(("authorization".into(), "Bearer ".into()));
    let user_vision = {
        let _config_write = lock_provider_config();
        toml_ops::model_has_user_vision_override(path, &cfg.provider, &cfg.model)
    };
    if !user_vision && result.vision.is_none() {
        result.vision = registry.get(&cfg.provider).and_then(|p| {
            p.models()
                .iter()
                .find(|m| m.id == cfg.model)
                .map(|m| m.supports_vision)
        });
        // The existing probe uses OpenAI chat/completions; never send it to Anthropic.
        if result.vision.is_none() && protocol != "anthropic" {
            let cfg = cfg.clone();
            result.vision = tokio::task::spawn_blocking(move || {
                llm::probe_vision(&cfg.base_url, &cfg.model, &cfg.api_key, &header, &prefix)
            })
            .await
            .ok()
            .flatten();
        }
    }
    result
}

// Caller holds the shared config transaction lock. Merge into freshly-read fields,
// never the old snapshot. This is also the final manual-override check.
fn merge(
    doc: &mut toml::Value,
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    key: &JobKey,
    result: &Discovery,
) -> Option<(bool, Option<usize>)> {
    let provider = segment(doc, &cfg.provider)?;
    if connection(provider, cfg)? != key.connection {
        return None;
    }
    if model_entry(provider, &cfg.model).is_none() {
        return model_entry(snapshot, &cfg.model)
            .is_none()
            .then_some((false, result.context));
    }
    let entry = doc
        .get_mut("providers")?
        .as_array_mut()?
        .iter_mut()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(cfg.provider.as_str()))?
        .get_mut("models")?
        .as_array_mut()?
        .iter_mut()
        .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(cfg.model.as_str()))?;
    let original = entry.clone();
    let existing = context(Some(entry));
    let existing_output = output(Some(entry));
    let fields = entry.as_table_mut()?;
    if existing.is_none() {
        if let Some(value) = result.context {
            fields.insert("context_window".into(), toml::Value::Integer(value as i64));
        }
    }
    // 输出上限：与 context 同一规则（连接指纹校验 + 只填空）。已有值——用户手写
    // 或先前采集——一律不动，且**绝不**写 `max_tokens`（那是用户配置意图，
    // 见 apply_capabilities 的契约）。写来源戳 `api`：标明这是机器采集的上游事实。
    if existing_output.is_none() {
        if let Some(value) = result.max_output {
            fields.insert(
                "max_output_tokens".into(),
                toml::Value::Integer(value as i64),
            );
            fields.insert(
                toml_ops::MAX_OUTPUT_TOKENS_SOURCE_KEY.into(),
                toml::Value::String("api".into()),
            );
        }
    }
    if fields
        .get(toml_ops::VISION_SOURCE_KEY)
        .and_then(|v| v.as_str())
        != Some("user")
    {
        if let Some(value) = result.vision {
            fields.insert("supports_vision".into(), toml::Value::Boolean(value));
        }
    }
    // Provider refresh or a later manual edit owns fields changed since dispatch.
    let old = model_entry(snapshot, &cfg.model);
    for (field, value) in [
        (
            "reasoning_efforts",
            (!result.efforts.is_empty()).then(|| {
                toml::Value::Array(
                    result
                        .efforts
                        .iter()
                        .cloned()
                        .map(toml::Value::String)
                        .collect(),
                )
            }),
        ),
        (
            "default_effort",
            result.default_effort.clone().map(toml::Value::String),
        ),
    ] {
        if original.get(field) == old.and_then(|m| m.get(field)) {
            if let Some(value) = value {
                fields.insert(field.into(), value);
            }
        }
    }
    Some((*entry != original, existing.or(result.context)))
}

fn commit(
    state: &AppState,
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    key: &JobKey,
    generation: u64,
    result: &Discovery,
) -> Result<bool, String> {
    let _config_write = lock_provider_config();
    let Some(mut doc) = read_doc(&state.llm_config_path) else {
        return Ok(false);
    };
    let Some((changed, window)) = merge(&mut doc, cfg, snapshot, key, result) else {
        return Ok(false);
    };
    if changed {
        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
        let content = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        nuphus::config::write_provider_config(&state.llm_config_path, &content)
            .map_err(|e| e.to_string())?;
    }
    let mut runtime = state.runtime.lock().map_err(|e| e.to_string())?;
    let mut runtime_changed = false;
    if is_current(&runtime, cfg, generation) {
        if let Some(window) = runtime.model_context_explicit.or(window) {
            runtime_changed = runtime.model_context_window != window;
            runtime.model_context_window = window;
        }
    }
    Ok(changed || runtime_changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (LlamaConfig, toml::Value, toml::Value, JobKey) {
        let cfg = LlamaConfig {
            provider: "custom-a".into(),
            model: "m".into(),
            base_url: "https://example.test/v1".into(),
            ..Default::default()
        };
        let doc: toml::Value = "[[providers]]\nname='custom-a'\nbase_url='https://example.test/v1'\n[[providers.models]]\nid='m'\n".parse().unwrap();
        let snapshot = segment(&doc, &cfg.provider).unwrap().clone();
        let key = JobKey {
            provider: cfg.provider.clone(),
            model: cfg.model.clone(),
            connection: connection(&snapshot, &cfg).unwrap(),
        };
        (cfg, doc, snapshot, key)
    }

    /// 回归（2026-10-26 大王实测）：runtime 未校准时 `get_context_limit` 回退读
    /// providers.toml 里的用户配置窗口 —— 该回退链的取值语义在此钉住。
    #[test]
    fn configured_context_window_reads_user_value() {
        let dir = std::env::temp_dir().join(format!("nuphus-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("providers.toml");
        std::fs::write(
            &path,
            "[[providers]]\nname='custom-x'\n[[providers.models]]\nid='m-1'\ncontext_window=1000000\n[[providers.models]]\nid='m-2'\n",
        )
        .unwrap();
        let cfg = |model: &str| LlamaConfig {
            provider: "custom-x".into(),
            model: model.into(),
            base_url: "https://example.test/v1".into(),
            ..Default::default()
        };
        // 用户配置值原样返回（UI 拿它当分母）
        assert_eq!(
            configured_context_window(&path, &cfg("m-1")),
            Some(1_000_000)
        );
        // 缺字段 / 条目不存在 → None（调用方继续报 0 = 未知，不伪造默认值）
        assert_eq!(configured_context_window(&path, &cfg("m-2")), None);
        assert_eq!(configured_context_window(&path, &cfg("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_jobs_follow_latest_generation_and_cache_success() {
        let (_, _, _, key) = fixture();
        let mut tasks = MetadataTasks::default();
        assert!(tasks.subscribe(&key, 1, true));
        assert!(!tasks.subscribe(&key, 3, true));
        assert_eq!(
            tasks.finish(
                &key,
                Discovery {
                    vision: Some(true),
                    ..Default::default()
                }
            ),
            Some(3)
        );
        assert_eq!(tasks.cache[&key].vision, Some(true));
        assert!(tasks.subscribe(&key, 4, true));
    }

    #[test]
    fn stale_binding_or_generation_cannot_change_runtime() {
        let (cfg, _, _, _) = fixture();
        let mut runtime = RuntimeContext {
            llm_config: Some(cfg.clone()),
            model_generation: 2,
            ..Default::default()
        };
        assert!(!is_current(&runtime, &cfg, 1));
        assert!(is_current(&runtime, &cfg, 2));
        runtime.llm_config.as_mut().unwrap().model = "another".into();
        assert!(!is_current(&runtime, &cfg, 2));
    }

    #[test]
    fn has_efforts_gate_drives_discovery() {
        let seg = |models: &str| {
            let doc: toml::Value = format!("[[providers]]\nname='p'\n{models}")
                .parse()
                .unwrap();
            segment(&doc, "p").unwrap().clone()
        };
        // 条目存在但未声明 / 空数组 → 需要探测
        assert!(!has_efforts(model_entry(
            &seg("[[providers.models]]\nid='m'\n"),
            "m"
        )));
        assert!(!has_efforts(model_entry(
            &seg("[[providers.models]]\nid='m'\nreasoning_efforts=[]\n"),
            "m"
        )));
        // 已声明非空 → 不重复探测
        assert!(has_efforts(model_entry(
            &seg("[[providers.models]]\nid='m'\nreasoning_efforts=['low','high']\n"),
            "m"
        )));
        // 条目不存在 → 视为未声明（调用方按 miss 处理）
        assert!(!has_efforts(model_entry(
            &seg("[[providers.models]]\nid='m'\n"),
            "other"
        )));
    }

    #[test]
    fn discovery_persists_efforts_when_entry_lacks_them() {
        let (cfg, mut doc, snapshot, key) = fixture();
        let result = Discovery {
            efforts: vec!["low".into(), "high".into(), "max".into()],
            default_effort: Some("high".into()),
            ..Default::default()
        };
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((true, None))
        );
        let entry = &doc["providers"][0]["models"][0];
        assert_eq!(
            entry["reasoning_efforts"],
            toml::Value::Array(vec!["low".into(), "high".into(), "max".into()])
        );
        assert_eq!(entry["default_effort"], toml::Value::String("high".into()));
    }

    #[test]
    fn discovery_does_not_clobber_user_edited_efforts() {
        let (cfg, mut doc, snapshot, key) = fixture();
        // 发现任务派发后用户手改了档位（doc 相对 snapshot 漂移）→ 不得覆写。
        // 注意：toml::Value 的 IndexMut 对缺失键直接 panic，必须走 as_table_mut 插入。
        doc["providers"][0]["models"][0]
            .as_table_mut()
            .unwrap()
            .insert(
                "reasoning_efforts".into(),
                toml::Value::Array(vec!["low".into()]),
            );
        let result = Discovery {
            efforts: vec!["low".into(), "high".into(), "max".into()],
            ..Default::default()
        };
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((false, None))
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["reasoning_efforts"],
            toml::Value::Array(vec!["low".into()])
        );
    }

    #[test]
    fn late_discovery_preserves_manual_values_and_unrelated_fields() {
        let (cfg, mut doc, snapshot, key) = fixture();
        let model = doc["providers"][0]["models"][0].as_table_mut().unwrap();
        model.insert("context_window".into(), toml::Value::Integer(999));
        model.insert("supports_vision".into(), toml::Value::Boolean(false));
        model.insert(
            toml_ops::VISION_SOURCE_KEY.into(),
            toml::Value::String("user".into()),
        );
        model.insert("note".into(), toml::Value::String("keep".into()));
        let result = Discovery {
            context: Some(123),
            vision: Some(true),
            ..Default::default()
        };
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((false, Some(999)))
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["supports_vision"].as_bool(),
            Some(false)
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["note"].as_str(),
            Some("keep")
        );
    }

    #[test]
    fn discovery_fills_missing_values_but_rejects_changed_endpoint_and_deleted_model() {
        let (cfg, mut doc, snapshot, key) = fixture();
        let result = Discovery {
            context: Some(123),
            vision: Some(true),
            ..Default::default()
        };
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((true, Some(123)))
        );
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((false, Some(123)))
        );
        doc["providers"][0]["base_url"] = toml::Value::String("https://other.test".into());
        assert_eq!(merge(&mut doc, &cfg, &snapshot, &key, &result), None);
        let (cfg, mut doc, snapshot, key) = fixture();
        doc["providers"][0]["models"] = toml::Value::Array(vec![]);
        assert_eq!(merge(&mut doc, &cfg, &snapshot, &key, &result), None);
    }

    /// A2 上限探测的解析契约：只认 `max_tokens: <req> > <ceiling>`，取 `>` 后
    /// 第一个数字。带模型名尾巴的真实文案里，尾巴中的数字（版本日期）不是上限。
    #[test]
    fn ceiling_from_api_error_parses_only_ceiling() {
        // 标准话题（Anthropic 官方文案形态）
        assert_eq!(
            ceiling_from_api_error(
                400,
                "max_tokens: 1000000 > 32000, which is the maximum allowed number of \
                 output tokens for claude-sonnet-4-20250514"
            ),
            Some(32_000)
        );
        // 带模型名尾巴（含数字）：必须取 4096，而不是 20241022 / 4
        assert_eq!(
            ceiling_from_api_error(
                400,
                "max_tokens: 8192 > 4096, which is the maximum allowed number of output \
                 tokens for claude-3-5-sonnet-20241022"
            ),
            Some(4_096)
        );
        // 无关 400（缺 max_tokens 字段的另一种必填校验）→ 解析不出
        assert_eq!(
            ceiling_from_api_error(400, "messages: text content blocks must be non-empty"),
            None
        );
        // 有 max_tokens 但无比较结构（「Field required」= 未解析到上限时省略字段
        // 被服务端打回）→ 解析不出
        assert_eq!(
            ceiling_from_api_error(400, "max_tokens: Field required"),
            None
        );
        // 非 400（401/429/5xx 与上限无关）→ 一律不采
        assert_eq!(
            ceiling_from_api_error(
                401,
                "max_tokens: 1000000 > 32000, which is the maximum allowed"
            ),
            None
        );
        assert_eq!(
            ceiling_from_api_error(
                429,
                "max_tokens: 1000000 > 32000, which is the maximum allowed"
            ),
            None
        );
        // 一致性护栏：ceiling >= 探测 sentinel 与「被 400 打回」矛盾 → 垃圾值丢弃
        assert_eq!(
            ceiling_from_api_error(
                400,
                "max_tokens: 1000000 > 1000000, which is the maximum allowed"
            ),
            None
        );
    }

    struct TestConfig(std::path::PathBuf);
    impl TestConfig {
        fn new(doc: &toml::Value) -> Self {
            let path =
                std::env::temp_dir().join(format!("nuphus-metadata-{}.toml", uuid::Uuid::new_v4()));
            std::fs::write(&path, toml::to_string(doc).unwrap()).unwrap();
            Self(path)
        }
    }
    impl Drop for TestConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[tokio::test]
    async fn activation_returns_while_background_discovery_is_blocked() {
        let (mut cfg, mut doc, _, _) = fixture();
        cfg.provider = "local".into(); // Release below must not contact a real provider.
        doc["providers"][0]["name"] = toml::Value::String("local".into());
        let config = TestConfig::new(&doc);
        let state = AppState {
            llm_config_path: config.0.clone(),
            ..Default::default()
        };
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        {
            let mut runtime = state.runtime.lock().unwrap();
            runtime.model_context_window = 999_999;
            runtime.metadata_tasks.permits = permits.clone();
        }
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let generation = activate(&state, &cfg, None).unwrap();
        schedule(app.handle().clone(), cfg.clone(), generation);
        {
            let runtime = state.runtime.lock().unwrap();
            assert_eq!(
                runtime.model_context_window, 0,
                "must not retain previous model window"
            );
            assert_eq!(runtime.metadata_tasks.pending.len(), 1);
            assert!(is_current(&runtime, &cfg, generation));
        }
        // A second activation is independent of the pending network job.
        let next = activate(&state, &cfg, Some(8192)).unwrap();
        schedule(app.handle().clone(), cfg.clone(), next);
        {
            let runtime = state.runtime.lock().unwrap();
            assert_eq!(runtime.model_context_window, 8192);
            assert_eq!(runtime.metadata_tasks.pending.len(), 1);
        }
        permits.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if state
                    .runtime
                    .lock()
                    .unwrap()
                    .metadata_tasks
                    .pending
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(state.runtime.lock().unwrap().model_context_window, 8192);
    }

    #[test]
    fn concurrent_config_writers_keep_both_fields() {
        let (_, doc, _, _) = fixture();
        let config = TestConfig::new(&doc);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for n in 1..=20 {
                    toml_ops::update_model_context_window(&config.0, "custom-a", "m", n).unwrap();
                }
            });
            scope.spawn(|| {
                for _ in 0..20 {
                    toml_ops::update_model_supports_vision(
                        &config.0,
                        "custom-a",
                        "m",
                        true,
                        Some("user"),
                    )
                    .unwrap();
                }
            });
        });
        assert_eq!(
            toml_ops::read_model_context_window(&config.0, "custom-a", "m"),
            Some(20)
        );
        assert_eq!(
            toml_ops::read_model_supports_vision(&config.0, "custom-a", "m"),
            Some(true)
        );
    }
}
