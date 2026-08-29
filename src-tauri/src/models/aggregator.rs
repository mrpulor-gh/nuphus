//! OpenRouter model metadata aggregator — authoritative capability source.
//!
//! Why this module exists: provider `/v1/models` endpoints are inconsistent
//! (Zhipu returns bare id lists, DeepSeek returns a provider-wide
//! `context_length`, …) and built-in `ProviderRegistry` tables are frozen at
//! compile time — new models always miss. OpenRouter publishes a free,
//! key-less model catalog (`GET https://openrouter.ai/api/v1/models`) with
//! per-model `context_length`, `input_modalities`, reasoning efforts and
//! pricing. This module turns that catalog into a local cached aggregate and
//! matches Nuphus `(provider, model)` pairs against it.
//!
//! Trust role: source ④ in the context-window trust chain
//! (① user explicit → ② providers.toml explicit → ③ live provider API →
//!  ④ OpenRouter aggregate → ⑤ builtin table → ⑥ unknown/None).
//! Only ③④ ever persist to disk; ⑤⑥ are runtime-only (never guessed values).
//!
//! Cache: `openrouter_cache.json` next to `providers.toml`, with dual TTL —
//! capability 24h / pricing 6h; any expired ⇒ whole refresh (single cache
//! file, no split-bookkeeping). Network failures degrade silently to the
//! stale cache / empty vec — never blocks the startup sync path.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// OpenRouter public model catalog endpoint (HTTP 200, no key required).
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
/// Cache file name — lives next to providers.toml.
pub const CACHE_FILE_NAME: &str = "openrouter_cache.json";
/// Capability (context window / modalities / efforts) freshness window.
pub const CONTEXT_TTL_SECS: u64 = 24 * 3600;
/// Pricing freshness window (shorter — prices move faster than capabilities).
pub const PRICE_TTL_SECS: u64 = 6 * 3600;
/// Network timeout for the OpenRouter fetch.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// One model entry distilled from the OpenRouter catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenRouterEntry {
    /// OpenRouter id, e.g. `z-ai/glm-5.3-flash` / `moonshotai/kimi-k3:free`.
    pub id: String,
    /// Context window in tokens (`top_provider.context_length` preferred, top-level fallback).
    pub context_length: Option<u64>,
    /// `architecture.input_modalities` — e.g. `["text","image"]`.
    pub input_modalities: Vec<String>,
    /// `architecture.output_modalities` — e.g. `["text","image"]` for image generators.
    pub output_modalities: Vec<String>,
    /// `reasoning.supported_efforts` — e.g. `["low","high"]`.
    pub supported_efforts: Vec<String>,
    /// `reasoning.default_effort`.
    pub default_effort: Option<String>,
    /// Prompt price in USD per *token* (OpenRouter unit); ×1_000_000 → per million.
    pub pricing_prompt_per_million: f64,
    /// Completion price in USD per *token*; ×1_000_000 → per million.
    pub pricing_completion_per_million: f64,
}

/// On-disk cache envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterCache {
    /// Unix seconds of the last successful fetch.
    pub fetched_at_unix: u64,
    pub entries: Vec<OpenRouterEntry>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Parse a price value — OpenRouter serializes pricing as strings (`"0.15"`)
/// but also tolerates raw numbers.
fn parse_price(v: &serde_json::Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    v.as_str().and_then(|s| s.trim().parse::<f64>().ok())
}

/// Distill one catalog item into an [`OpenRouterEntry`]. Malformed items are
/// skipped (filter_map) — the catalog is third-party and a single broken row
/// must not poison the whole cache.
fn parse_entry(item: &serde_json::Value) -> Option<OpenRouterEntry> {
    let id = item.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let top_provider = item.get("top_provider");
    let context_length = top_provider
        .and_then(|tp| tp.get("context_length").and_then(|v| v.as_u64()))
        .or_else(|| item.get("context_length").and_then(|v| v.as_u64()));

    let modalities = |key: &str| -> Vec<String> {
        item.get("architecture")
            .and_then(|a| a.get(key))
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let input_modalities = modalities("input_modalities");
    let output_modalities = modalities("output_modalities");

    let reasoning = item.get("reasoning");
    let supported_efforts = reasoning
        .and_then(|r| r.get("supported_efforts"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let default_effort = reasoning
        .and_then(|r| r.get("default_effort"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let pricing = item.get("pricing");
    let pricing_prompt_per_million = pricing
        .and_then(|p| p.get("prompt"))
        .and_then(parse_price)
        .unwrap_or(0.0);
    let pricing_completion_per_million = pricing
        .and_then(|p| p.get("completion"))
        .and_then(parse_price)
        .unwrap_or(0.0);

    Some(OpenRouterEntry {
        id,
        context_length,
        input_modalities,
        output_modalities,
        supported_efforts,
        default_effort,
        pricing_prompt_per_million,
        pricing_completion_per_million,
    })
}

/// Fetch the full catalog from OpenRouter (async, 10s timeout).
pub async fn fetch() -> Result<Vec<OpenRouterEntry>, String> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent("Nuphus/1.0")
        .build()
        .map_err(|e| format!("create OpenRouter client failed: {e}"))?;
    let resp = client
        .get(OPENROUTER_MODELS_URL)
        .send()
        .await
        .map_err(|e| format!("OpenRouter 拉取失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("OpenRouter HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("OpenRouter 响应解析失败: {e}"))?;
    let data = body
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "OpenRouter 响应缺少 data 数组".to_string())?;
    let entries: Vec<OpenRouterEntry> = data.iter().filter_map(parse_entry).collect();
    if entries.is_empty() {
        return Err("OpenRouter 响应无有效模型条目".to_string());
    }
    tracing::info!("[openrouter] fetched {} model entries", entries.len());
    Ok(entries)
}

/// Cache file path next to the config dir (providers.toml parent).
pub fn cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CACHE_FILE_NAME)
}

/// Load the on-disk cache (None = missing / corrupt).
pub fn load_cache(path: &Path) -> Option<OpenRouterCache> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persist the cache envelope.
pub fn save_cache(path: &Path, entries: &[OpenRouterEntry]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let cache = OpenRouterCache {
        fetched_at_unix: now_unix(),
        entries: entries.to_vec(),
    };
    let json = serde_json::to_string(&cache).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// Dual-TTL staleness check — capability 24h / pricing 6h; either expired ⇒ stale.
pub fn is_stale(cache: &OpenRouterCache) -> bool {
    let age = now_unix().saturating_sub(cache.fetched_at_unix);
    age > CONTEXT_TTL_SECS || age > PRICE_TTL_SECS
}

/// Stale-while-revalidate: fresh cache → return it; stale/missing → refetch and
/// rewrite, falling back to the stale cache on network failure (never blocks
/// the startup sync path with a hard error).
pub async fn ensure_cache(path: &Path) -> Vec<OpenRouterEntry> {
    if let Some(cache) = load_cache(path) {
        if !is_stale(&cache) {
            return cache.entries;
        }
    }
    match fetch().await {
        Ok(entries) => {
            let _ = save_cache(path, &entries);
            entries
        }
        Err(e) => {
            tracing::warn!("[openrouter] refresh failed, falling back to stale cache: {e}");
            load_cache(path).map(|c| c.entries).unwrap_or_default()
        }
    }
}

/// Whether the aggregator knows a vendor mapping for this provider id.
/// `custom` / `local` / unknown providers are never matched against OpenRouter.
pub fn has_vendor(provider_id: &str) -> bool {
    vendor_for(provider_id).is_some()
}

/// Vendor prefix mapping: Nuphus provider id → OpenRouter vendor namespace.
fn vendor_for(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "zhipu" => Some("z-ai"),
        "deepseek" => Some("deepseek"),
        "minimax" => Some("minimax"),
        "kimi" => Some("moonshotai"),
        "openai" => Some("openai"),
        "anthropic" => Some("anthropic"),
        "google" => Some("google"),
        "openrouter" => Some("openrouter"), // 直连：model_id 即 OR id
        _ => None,
    }
}

/// Normalize an OpenRouter id to its matching base segment:
/// strip the `vendor/` prefix, `:free`/`:batch` variants and trailing
/// `-YYYYMMDD` date suffixes (`z-ai/glm-5.2:free` → `glm-5.2`).
fn normalize_base(or_id: &str) -> String {
    let after_slash = or_id.rsplit('/').next().unwrap_or(or_id);
    let after_colon = after_slash.split(':').next().unwrap_or(after_slash);
    let base = after_colon.to_lowercase();
    // 末尾日期段 `-YYYYMMDD`（如 glm-5.2-20250801）→ 去掉
    if let Some(idx) = base.rfind('-') {
        let tail = &base[idx + 1..];
        if tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()) {
            return base[..idx].to_string();
        }
    }
    base
}

/// Look up a Nuphus (provider_id, model_id) pair in the OpenRouter aggregate.
///
/// Match rules (in order):
/// ① exact `vendor/model_id` (case-insensitive) — e.g. zhipu+glm-5.3-flash
///    → `z-ai/glm-5.3-flash`;
/// ② degraded: the id's base segment equals `model_id` (`:free`/`:batch` and
///    `-YYYYMMDD` suffixes ignored) — e.g. zhipu+glm-5.2 → `z-ai/glm-5.2:free`;
/// ③ degraded: base segment ends with `-{model_id}` (vendor shorthand) —
///    e.g. kimi+k3 → `moonshotai/kimi-k3`.
///
/// `openrouter` provider matches the model_id directly against OR ids.
/// Unknown providers (`custom`/`local`/…) return `None`.
pub fn lookup<'a>(
    entries: &'a [OpenRouterEntry],
    provider_id: &str,
    model_id: &str,
) -> Option<&'a OpenRouterEntry> {
    let model_lower = model_id.trim().to_lowercase();
    if model_lower.is_empty() {
        return None;
    }

    if provider_id == "openrouter" {
        for e in entries {
            if e.id.to_lowercase() == model_lower {
                return Some(e);
            }
        }
        for e in entries {
            if normalize_base(&e.id) == model_lower {
                return Some(e);
            }
        }
        return None;
    }

    let vendor = vendor_for(provider_id)?;

    // ① 精确 vendor/model_id
    let exact = format!("{}/{}", vendor, model_lower);
    for e in entries {
        if e.id.to_lowercase() == exact {
            return Some(e);
        }
    }

    // ②③ 退化匹配
    for e in entries {
        let base = normalize_base(&e.id);
        if base == model_lower {
            return Some(e);
        }
        if base.len() > model_lower.len() + 1 && base.ends_with(&format!("-{}", model_lower)) {
            return Some(e);
        }
    }
    None
}

/// Two-tier read-only-cache lookup:
/// tier 1 — known vendor via [`lookup`]（builtin 映射表里的厂商）；
/// tier 2 — unknown/custom providers（中转站、自建网关）：无法静态映射 vendor，
/// 用模型 id 的基名段在全目录里做「唯一命中」匹配 —— 中转站转发的大多是
/// 与官方同名同源的模型，能力数据可信；恰好一个命中才采信，零或多个
/// → None（宁缺勿错，避免跨厂商重名误配出假能力值）。
pub fn lookup_generic_cached(
    config_dir: &Path,
    provider_id: &str,
    model_id: &str,
) -> Option<OpenRouterEntry> {
    let path = cache_path(config_dir);
    let cache = load_cache(&path)?;
    if let Some(e) = lookup(&cache.entries, provider_id, model_id) {
        return Some(e.clone());
    }
    let model_lower = model_id.trim().to_lowercase();
    if model_lower.is_empty() {
        return None;
    }
    let hits: Vec<&OpenRouterEntry> = cache
        .entries
        .iter()
        .filter(|e| normalize_base(&e.id) == model_lower)
        .collect();
    if hits.len() == 1 {
        Some(hits[0].clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, ctx: Option<u64>) -> OpenRouterEntry {
        OpenRouterEntry {
            id: id.to_string(),
            context_length: ctx,
            input_modalities: vec!["text".to_string()],
            output_modalities: vec!["text".to_string()],
            supported_efforts: Vec::new(),
            default_effort: None,
            pricing_prompt_per_million: 0.0,
            pricing_completion_per_million: 0.0,
        }
    }

    #[test]
    fn lookup_exact_vendor_model() {
        let entries = vec![entry("z-ai/glm-5.3-flash", Some(1_048_576))];
        let hit = lookup(&entries, "zhipu", "glm-5.3-flash").expect("should hit");
        assert_eq!(hit.context_length, Some(1_048_576));
    }

    #[test]
    fn lookup_ignores_free_variant() {
        let entries = vec![entry("z-ai/glm-5.2:free", Some(131_072))];
        let hit = lookup(&entries, "zhipu", "glm-5.2").expect("should hit :free variant");
        assert_eq!(hit.id, "z-ai/glm-5.2:free");
    }

    #[test]
    fn lookup_kimi_shorthand_suffix() {
        let entries = vec![entry("moonshotai/kimi-k3", Some(1_048_576))];
        let hit = lookup(&entries, "kimi", "k3").expect("should hit kimi-k3 via -suffix");
        assert_eq!(hit.id, "moonshotai/kimi-k3");
    }

    #[test]
    fn lookup_trim_date_suffix() {
        let entries = vec![entry("z-ai/glm-5.2-20250801", Some(200_000))];
        let hit = lookup(&entries, "zhipu", "glm-5.2").expect("should trim date suffix");
        assert_eq!(hit.id, "z-ai/glm-5.2-20250801");
    }

    #[test]
    fn lookup_openrouter_direct_id() {
        let entries = vec![entry("z-ai/glm-5.3-flash", Some(1_048_576))];
        let hit = lookup(&entries, "openrouter", "z-ai/glm-5.3-flash").expect("direct id");
        assert_eq!(hit.id, "z-ai/glm-5.3-flash");
    }

    #[test]
    fn lookup_unknown_provider_none() {
        let entries = vec![entry("z-ai/glm-5.3-flash", Some(1_048_576))];
        assert!(lookup(&entries, "custom", "glm-5.3-flash").is_none());
        assert!(lookup(&entries, "local", "glm-5.3-flash").is_none());
        assert!(lookup(&entries, "unknown-provider", "x").is_none());
    }

    #[test]
    fn normalize_base_strips_variant_and_date() {
        assert_eq!(normalize_base("z-ai/glm-5.2:free"), "glm-5.2");
        assert_eq!(normalize_base("moonshotai/kimi-k3"), "kimi-k3");
        assert_eq!(normalize_base("z-ai/glm-5.2-20250801"), "glm-5.2");
        assert_eq!(normalize_base("z-ai/glm-5.2:batch"), "glm-5.2");
    }

    #[test]
    fn vendor_mapping() {
        assert_eq!(vendor_for("zhipu"), Some("z-ai"));
        assert_eq!(vendor_for("deepseek"), Some("deepseek"));
        assert_eq!(vendor_for("kimi"), Some("moonshotai"));
        assert_eq!(vendor_for("custom"), None);
        assert_eq!(vendor_for("local"), None);
        assert!(has_vendor("zhipu"));
        assert!(!has_vendor("custom"));
    }

    #[test]
    fn parse_entry_from_sample_payload() {
        let payload: serde_json::Value = serde_json::json!({
            "id": "z-ai/glm-5.3-flash",
            "top_provider": { "context_length": 1048576 },
            "architecture": {
                "input_modalities": ["text", "image"],
                "output_modalities": ["text"]
            },
            "reasoning": { "supported_efforts": ["low", "high"], "default_effort": "high" },
            "pricing": { "prompt": "0.15", "completion": "0.6" }
        });
        let e = parse_entry(&payload).expect("entry");
        assert_eq!(e.id, "z-ai/glm-5.3-flash");
        assert_eq!(e.context_length, Some(1_048_576));
        assert_eq!(e.input_modalities, vec!["text", "image"]);
        assert_eq!(e.output_modalities, vec!["text"]);
        assert_eq!(e.supported_efforts, vec!["low", "high"]);
        assert_eq!(e.default_effort.as_deref(), Some("high"));
        assert!((e.pricing_prompt_per_million - 0.15).abs() < 1e-9);
        assert!((e.pricing_completion_per_million - 0.6).abs() < 1e-9);
    }

    #[test]
    fn cache_roundtrip_and_stale() {
        let dir = std::env::temp_dir().join(format!(
            "nuphus-oragg-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = cache_path(&dir);
        let entries = vec![entry("z-ai/glm-5.3-flash", Some(1_048_576))];
        save_cache(&path, &entries).unwrap();
        let loaded = load_cache(&path).expect("cache");
        assert_eq!(loaded.entries.len(), 1);
        assert!(!is_stale(&loaded));
        // 旧时间戳 → stale（价格 TTL 6h）
        let mut stale = loaded.clone();
        stale.fetched_at_unix = now_unix().saturating_sub(PRICE_TTL_SECS + 1);
        assert!(is_stale(&stale));
        std::fs::remove_dir_all(&dir).ok();
    }
}
