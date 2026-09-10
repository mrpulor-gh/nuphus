//! providers.toml `[last_model]` 表 — model→provider 归属的磁盘记录与权威解析。
//!
//! 背景：`[agent_models]` 只存 model id；同 id 跨 provider 段（官方 deepseek 与
//! opencode-go 网关都列 `deepseek-v4-flash`）时，仅凭磁盘文件顺序无法区分实际
//! 生效归属（旧 `get_current_config` 按段顺序取第一段 → 弹窗勾选串卡）。
//! `switch_model` 切换时经 `record_last_model` 落盘归属；
//! `resolve_model_provider_core` 按「内存 runtime → [last_model] → registry 候选」
//! 三级优先级解析，供桌面端 `get_provider_context` 命令消费。

use super::model::ModelRegistry;

/// 记录某模型最近一次切换归属的 provider：providers.toml `[last_model]` 表
/// `model_id = "provider_name"`。
///
/// 空参数不写盘（避免脏记录）；写后整文档经 `encrypt_plaintext_provider_keys`
/// 再序列化（与 agent_models 落盘同一口径，确保不旁落明文密钥）。
pub fn record_last_model(
    providers_path: &std::path::Path,
    model: &str,
    provider: &str,
) -> Result<(), String> {
    if model.is_empty() || provider.is_empty() {
        return Ok(());
    }
    let content = std::fs::read_to_string(providers_path).unwrap_or_default();
    let mut doc: toml::Value = content
        .parse()
        .unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()));
    let table = doc
        .as_table_mut()
        .ok_or_else(|| "providers.toml is not a table".to_string())?;
    let section = table
        .entry("last_model")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "last_model is not a table".to_string())?;
    section.insert(model.to_string(), toml::Value::String(provider.to_string()));

    crate::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content = toml::to_string_pretty(&doc)
        .map_err(|e| format!("serialize providers.toml failed: {e}"))?;
    std::fs::write(providers_path, new_content)
        .map_err(|e| format!("write providers.toml failed: {e}"))?;
    tracing::info!("[last_model] {model} = {provider}");
    Ok(())
}

/// 读 `[last_model]` 中某模型最近切换的 provider（None = 无记录）。
///
/// Windows 上刚写盘的文件可能被杀软/索引器短暂独占（读报 PermissionDenied =
/// sharing violation），故对暂时性不可读做有限重试——避免把「暂时读不到」
/// 误判成「无记录」，导致 get_provider_context 归属闪断。
pub fn load_last_model_provider(providers_path: &std::path::Path, model: &str) -> Option<String> {
    let mut content: Option<String> = None;
    for attempt in 0..3u8 {
        match std::fs::read_to_string(providers_path) {
            Ok(c) => {
                content = Some(c);
                break;
            }
            Err(e) if attempt < 2 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    let doc: toml::Value = content?.parse().ok()?;
    doc.get("last_model")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get(model))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// provider 归属解析核心（桌面 `get_provider_context` 命令与单测共用，无 tauri 依赖）。
///
/// 解析顺序（a/b 都要求「该段确实发布此 model」——仅凭段存在会静默错配）：
///   a. 内存权威：`runtime.llm_config.model == 生效model` 且内存 provider 确实
///      发布该 model 时返回之 —— chat 实际用它路由，切换后与磁盘记录必然一致
///      且最新；mode 切走后残留的内存绑定不成立，回落 b；
///   b. 磁盘切换记录：`[last_model]` 查该 model。同 id 跨段时后写覆盖单值键，
///      记录可能指向一个不含此 model 的段（`[last_model]` 单值键 +
///      `[agent_models]` 绑定不一致的实证场景），必须继续回落到 c；
///   c. registry 候选回落：`find_model_candidates` 按段序取首候选——多命中时
///      不含任何段名启发式，权威归属由 a/b 承担，此段仅是最后兜底；
///      无命中 → 空串。
pub fn resolve_model_provider_core(
    memory: Option<(&str, &str)>,
    providers_path: &std::path::Path,
    registry: &ModelRegistry,
    model: &str,
) -> String {
    if model.is_empty() {
        return String::new();
    }
    // 候选集只解析一次：a/b 的存在性校验与 c 的回落共用，避免重复扫描。
    let candidates = registry.find_model_candidates(model);
    let publishes = |provider: &str| candidates.iter().any(|(p, _)| p.name == provider);
    // a. 内存 runtime（chat 实际路由依据）
    if let Some((mem_model, mem_provider)) = memory {
        if mem_model == model && !mem_provider.is_empty() && publishes(mem_provider) {
            return mem_provider.to_string();
        }
    }
    // b. [last_model] 磁盘切换记录 —— 记录必须指向一个「确实发布该 model」的段，
    //    否则是脏记录（同 id 跨段时单值键被后写覆盖）：不猜、继续回落 c。
    if let Some(recorded) = load_last_model_provider(providers_path, model) {
        if !recorded.is_empty() && publishes(&recorded) {
            return recorded;
        }
    }
    // c. registry 同 id 候选回落（多命中按段序取首候选，无语义猜测）
    match candidates.len() {
        0 => String::new(),
        _ => candidates[0].0.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        // 同进程内并发测试线程的 as_nanos() 可能碰撞，用原子序号保证唯一，
        // 避免两测试共用 temp 目录互相覆写 providers.toml（flaky 根因）。
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nuphus-last-model-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            seq
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 同 id 双段 fixture：官方 deepseek 段在前（模拟磁盘文件顺序），
    /// opencode-go 段在后，两段都含 `deepseek-v4-flash`。go_has_id=false 时
    /// GO 段只有别的模型（候选只剩官方段，验证记录优先级）。
    fn dual_segment_registry(go_has_id: bool) -> (std::path::PathBuf, ModelRegistry) {
        let dir = temp_dir();
        let cfg_path = dir.join("providers.toml");
        let mut cfg = String::from(
            "model = \"deepseek-v4-flash\"\n\n[[providers]]\nname = \"deepseek\"\nprovider_type = \"deepseek\"\napi_key = \"sk-x\"\nbase_url = \"https://api.deepseek.com\"\n\n[[providers.models]]\nid = \"deepseek-v4-flash\"\n\n[[providers]]\nname = \"opencode-go\"\nprovider_type = \"opencode-go\"\napi_key = \"sk-y\"\nbase_url = \"https://opencode.ai/zen/go/v1\"\n\n[[providers.models]]\nid = \"qwen3.8-flash\"\n",
        );
        if go_has_id {
            cfg += "\n[[providers.models]]\nid = \"deepseek-v4-flash\"\n";
        }
        std::fs::write(&cfg_path, cfg).unwrap();
        let registry = ModelRegistry::from_toml(cfg_path.to_str().unwrap()).unwrap();
        (dir, registry)
    }

    #[test]
    fn last_model_record_roundtrip() {
        let dir = temp_dir();
        let path = dir.join("providers.toml");
        std::fs::write(&path, "[agent_models]\nleader = \"m1\"\n").unwrap();

        assert_eq!(load_last_model_provider(&path, "m1"), None);
        record_last_model(&path, "m1", "opencode-go").unwrap();
        assert_eq!(
            load_last_model_provider(&path, "m1").as_deref(),
            Some("opencode-go")
        );
        // 覆写：同 model 后写覆盖先写；不同 model 各自独立
        record_last_model(&path, "m1", "deepseek").unwrap();
        record_last_model(&path, "m2", "opencode-go").unwrap();
        assert_eq!(
            load_last_model_provider(&path, "m1").as_deref(),
            Some("deepseek")
        );
        assert_eq!(
            load_last_model_provider(&path, "m2").as_deref(),
            Some("opencode-go")
        );
        // 空参数不写脏记录
        record_last_model(&path, "", "deepseek").unwrap();
        record_last_model(&path, "m3", "").unwrap();
        assert_eq!(load_last_model_provider(&path, "m3"), None);
        // 落盘后 toml 可 reparse，且不破坏既有 [agent_models]
        let doc: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        assert_eq!(
            doc.get("agent_models")
                .and_then(|v| v.get("leader"))
                .and_then(|v| v.as_str()),
            Some("m1")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn provider_context_memory_authority_beats_disk() {
        let (dir, registry) = dual_segment_registry(true);
        let path = dir.join("providers.toml");
        // 内存 runtime（chat 实际路由）= 官方 deepseek，磁盘候选会偏向网关段；
        // 生效 model 与内存一致时以内存为准（弹窗勾官方卡）。
        assert_eq!(
            resolve_model_provider_core(
                Some(("deepseek-v4-flash", "deepseek")),
                &path,
                &registry,
                "deepseek-v4-flash",
            ),
            "deepseek"
        );
        // 内存模型与生效 model 不一致（mode 已切走）→ 不信内存，走候选回落
        // （双段同 id 且无记录 → 段序首候选 = 官方 deepseek 段）
        assert_eq!(
            resolve_model_provider_core(
                Some(("other-model", "deepseek")),
                &path,
                &registry,
                "deepseek-v4-flash",
            ),
            "deepseek"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 内存绑定指向一个未发布该 model 的段（mode 切走后的陈旧记录）→ 不静默
    /// 错配，回落候选；a 段的「内存优先」地位不变，但同样受存在性校验约束。
    #[test]
    fn provider_context_stale_memory_provider_falls_through() {
        let (dir, registry) = dual_segment_registry(false);
        let path = dir.join("providers.toml");
        // GO 段存在但不发布该 id → 内存绑定 opencode-go 为陈旧记录
        assert_eq!(
            resolve_model_provider_core(
                Some(("deepseek-v4-flash", "opencode-go")),
                &path,
                &registry,
                "deepseek-v4-flash",
            ),
            "deepseek"
        );
        // 内存 provider 段已被删除 → 同样回落
        assert_eq!(
            resolve_model_provider_core(
                Some(("deepseek-v4-flash", "gone-provider")),
                &path,
                &registry,
                "deepseek-v4-flash",
            ),
            "deepseek"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn provider_context_last_model_record_and_stale_fallback() {
        // GO 段无该 id（候选只有官方段）→ 任何合法归属都只能落到 deepseek。
        let (dir, registry) = dual_segment_registry(false);
        let path = dir.join("providers.toml");
        // 记录段存在但未发布该 model（同 id 单值键被后写覆盖的脏记录）→
        // 不静默错配，回落候选。
        record_last_model(&path, "deepseek-v4-flash", "opencode-go").unwrap();
        assert_eq!(
            resolve_model_provider_core(None, &path, &registry, "deepseek-v4-flash"),
            "deepseek"
        );
        // 脏记录（provider 段已删除）→ 不信记录，回落到候选
        record_last_model(&path, "deepseek-v4-flash", "gone-provider").unwrap();
        assert_eq!(
            resolve_model_provider_core(None, &path, &registry, "deepseek-v4-flash"),
            "deepseek"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 记录段确实发布该 model（合法归属）→ 记录赢，不被候选回落覆盖。
    #[test]
    fn provider_context_record_wins_when_provider_publishes_model() {
        let (dir, registry) = dual_segment_registry(true);
        let path = dir.join("providers.toml");
        record_last_model(&path, "deepseek-v4-flash", "opencode-go").unwrap();
        assert_eq!(
            resolve_model_provider_core(None, &path, &registry, "deepseek-v4-flash"),
            "opencode-go"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn provider_context_candidates_fallback() {
        let (dir, registry) = dual_segment_registry(true);
        let path = dir.join("providers.toml");
        // 双段同 id 且无记录：多命中按段序取首候选（官方 deepseek 段在前），
        // 不做任何段名猜测
        assert_eq!(
            resolve_model_provider_core(None, &path, &registry, "deepseek-v4-flash"),
            "deepseek"
        );
        // 唯一命中直接取之（GO 段独有模型）
        assert_eq!(
            resolve_model_provider_core(None, &path, &registry, "qwen3.8-flash"),
            "opencode-go"
        );
        // 无命中 → 空串；model 空 → 空串
        assert_eq!(
            resolve_model_provider_core(None, &path, &registry, "missing-model"),
            ""
        );
        assert_eq!(resolve_model_provider_core(None, &path, &registry, ""), "");
        std::fs::remove_dir_all(&dir).ok();
    }
}
