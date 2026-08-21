//! 应用插件（App Plugin）体系：安装器 + KV 存储 + 主题快照
//!
//! 契约：docs/plugin-app-system-plan.md §3（包格式/硬限制）/ §4.3（主题快照）/ §5（Bridge 后端落点）/ §6.2（安装器）。
//! 目录惯例：`plugin/apps/{id}/` + `plugin/apps/registry.json`，与 skills/knowledge/workflows 同级。
//! 路径解析**复用** commands::knowledge 的模式（exe 相对 → current_dir，向上遍历父目录），禁止自创规则。
//!
//! 安全边界：
//! - 安装器：zip-slip 防逃逸（enclosed_name 规范化 + 硬限制 20MB/500 文件/manifest 64KB）
//! - KV：id 必须已注册且 enabled，防未安装插件写数据；单值 ≤256KB、总量 ≤4MB
//! - 主题快照：仅内存态（AppState），由 /plugins-shared/theme.css 渲染下发

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use nuphus::agent::events::{EventEmitter, NuphusEvent};
use nuphus::api::ApiClient;
use nuphus::runtime::{Mode, RuntimeBuilder, RuntimeConfig};
use nuphus::session::Session;

use crate::commands::workflow::inject_workflow_runtime;
use crate::state::{AppState, HistoryMessage};

// ============================================================================
// 常量与硬限制（docs/plugin-app-system-plan.md §3）
// ============================================================================

/// 宿主版本（minHost 比较基准；与 tauri.conf.json version 同步维护）
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 解压后总尺寸上限
const MAX_ARCHIVE_TOTAL_BYTES: u64 = 20 * 1024 * 1024;
/// ZIP 文件数上限
const MAX_ARCHIVE_FILES: usize = 500;
/// manifest.json 单文件上限
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// KV 单值序列化后上限
const MAX_KV_VALUE_BYTES: usize = 256 * 1024;
/// KV 文件总量上限
const MAX_KV_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// 主题覆盖条目上限
const MAX_THEME_OVERRIDES: usize = 100;
/// 主题单值字符数上限
const MAX_THEME_VALUE_CHARS: usize = 500;
/// agent.chat 单条消息字节上限（32KB）
const MAX_PLUGIN_CHAT_BYTES: usize = 32 * 1024;
/// agent.chat 独立运行时单次执行硬超时（前端 120s 先行返回，后端最终收尸）
const PLUGIN_CHAT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// workflow.run 单次执行硬超时（前端 120s 先行返回，后端最终收尸；对齐 agent.chat）
const PLUGIN_WORKFLOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// 全局并发闸：插件侧执行（agent.chat + workflow.run）最多 2 路并行，第 3 路排队等待（不拒绝）
const PLUGIN_CHAT_MAX_CONCURRENT: usize = 2;
/// history 条数上限（与样例插件 KV 最近 50 条一致）
const MAX_PLUGIN_CHAT_HISTORY: usize = 50;
/// history 总字符上限（64KB）
const MAX_PLUGIN_CHAT_HISTORY_CHARS: usize = 64 * 1024;

/// v1 允许的权限枚举（v1.1 起随 Bridge API 面扩展）
const PERMISSIONS_ALLOWED: [&str; 5] = ["kv", "notify", "theme.get", "agent.chat", "workflow.run"];

// ============================================================================
// 路径解析（复用 commands::knowledge::find_plugin_knowledge 的向上遍历模式）
// ============================================================================

/// 查找 plugin 根目录：exe 相对 → current_dir，各自向上遍历父目录，
/// 命中含 `plugin/apps` 或 `plugin/knowledge` 的 `plugin` 目录即返回。
/// 与 commands::knowledge 的解析顺序完全一致，兜底 current_dir/plugin。
pub fn find_plugin_dir() -> PathBuf {
    let candidates: Vec<PathBuf> = vec![
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf())),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .collect();

    for base in &candidates {
        let mut current = Some(base.as_path());
        while let Some(dir) = current {
            let plugin = dir.join("plugin");
            if plugin.join("apps").exists() || plugin.join("knowledge").exists() {
                return plugin;
            }
            current = dir.parent();
        }
    }

    std::env::current_dir().unwrap_or_default().join("plugin")
}

/// 插件 apps 根目录（plugin/apps/）
pub fn apps_root() -> PathBuf {
    find_plugin_dir().join("apps")
}

// ============================================================================
// manifest 校验（docs/plugin-app-system-plan.md §3 Schema v1）
// ============================================================================

/// serde skip 条件：sample=false（缺省）时序列化跳过，保持 Schema v1 最小噪音
fn is_false(b: &bool) -> bool {
    !*b
}

/// manifest.json Schema v1。字段名（minHost/sidecar）为开源契约，勿改名。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// 展示分类（可选；ai/tools/integration/productivity/other，未知值宽容归入 other）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// 官方示例标记（可选；true = 官方示例插件，前端渲染「示例」徽章。缺省 false，向后兼容）
    #[serde(default, skip_serializing_if = "is_false")]
    pub sample: bool,
    /// 宿主最低版本
    #[serde(default, rename = "minHost", skip_serializing_if = "Option::is_none")]
    pub min_host: Option<String>,
    /// 声明的权限子集（v1: kv/notify/theme.get/agent.chat/workflow.run；未声明即拒）
    #[serde(default)]
    pub permissions: Vec<String>,
    /// v1 固定 null；非 null 拒收（v1.1 开放 MCP sidecar 声明）
    #[serde(default)]
    pub sidecar: Option<serde_json::Value>,
}

/// 插件 id：reverse-dns 格式 `^[a-z][a-z0-9-]*(\.[a-z0-9-]+)+$`
pub fn valid_plugin_id(id: &str) -> bool {
    let mut segments = id.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    let mut first_chars = first.chars();
    let Some(c0) = first_chars.next() else {
        return false;
    };
    if !c0.is_ascii_lowercase() {
        return false;
    }
    if !first_chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return false;
    }
    let mut count = 1;
    for seg in segments {
        count += 1;
        if seg.is_empty()
            || !seg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return false;
        }
    }
    count >= 2
}

/// 版本号：三段数字 semver `^(\d+)\.(\d+)\.(\d+)$`
pub fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// 简单 semver 比较（三段数字逐级比较）；任一格式非法返回 None
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let pa: Vec<u64> = a
        .split('.')
        .map(|s| s.parse().ok())
        .collect::<Option<Vec<_>>>()?;
    let pb: Vec<u64> = b
        .split('.')
        .map(|s| s.parse().ok())
        .collect::<Option<Vec<_>>>()?;
    if pa.len() != 3 || pb.len() != 3 {
        return None;
    }
    Some(pa.cmp(&pb))
}

/// 包内相对路径：禁止 `..` 逃逸、绝对路径、Windows 反斜杠与空段
pub fn valid_entry_path(entry: &str) -> bool {
    let normalized = entry.replace('\\', "/");
    if normalized.starts_with('/') || normalized.is_empty() {
        return false;
    }
    normalized
        .split('/')
        .all(|seg| seg != ".." && !seg.is_empty())
}

/// 全量 manifest 校验（安装入口；单测覆盖全部拒绝分支）
fn validate_manifest(m: &PluginManifest) -> Result<(), String> {
    if !valid_plugin_id(&m.id) {
        return Err(format!(
            "插件 id 非法（需 reverse-dns 格式，如 com.author.my-plugin）: {}",
            m.id
        ));
    }
    if !valid_version(&m.version) {
        return Err(format!("插件版本非法（需 x.y.z 三段数字）: {}", m.version));
    }
    if !valid_entry_path(&m.entry) {
        return Err(format!("插件入口路径非法（禁止 .. 逃逸）: {}", m.entry));
    }
    if let Some(icon) = &m.icon {
        if !valid_entry_path(icon) {
            return Err(format!("插件图标路径非法: {}", icon));
        }
        if !icon.to_ascii_lowercase().ends_with(".svg") {
            return Err(format!(
                "插件图标必须为 SVG 格式（市场准入基础要求，如 icon.svg）: {icon}"
            ));
        }
    } else {
        return Err("插件缺少图标：manifest 必须声明 icon（SVG，如 icon.svg）".to_string());
    }
    for p in &m.permissions {
        if !PERMISSIONS_ALLOWED.contains(&p.as_str()) {
            return Err(format!(
                "插件权限非法: {p}（v1 仅允许 kv/notify/theme.get/agent.chat/workflow.run）"
            ));
        }
    }
    if let Some(sidecar) = &m.sidecar {
        if !sidecar.is_null() {
            return Err("v1 暂不支持 sidecar（MCP sidecar 声明 v1.1 开放）".to_string());
        }
    }
    if let Some(min_host) = &m.min_host {
        match compare_versions(min_host, HOST_VERSION) {
            Some(std::cmp::Ordering::Greater) => {
                return Err(format!(
                    "插件要求宿主最低版本 {min_host}，当前宿主 {HOST_VERSION}"
                ));
            }
            Some(_) => {}
            None => return Err(format!("插件 minHost 版本非法: {min_host}")),
        }
    }
    Ok(())
}

// ============================================================================
// 注册表（plugin/apps/registry.json）
// ============================================================================

/// 注册表条目：{ id: PluginRegistryEntry }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryEntry {
    pub version: String,
    pub enabled: bool,
    pub installed_at: String,
    pub permissions: Vec<String>,
}

pub type PluginRegistry = HashMap<String, PluginRegistryEntry>;

fn registry_path_from(apps_root: &Path) -> PathBuf {
    apps_root.join("registry.json")
}

fn load_registry_from(apps_root: &Path) -> PluginRegistry {
    std::fs::read_to_string(registry_path_from(apps_root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_registry_to(apps_root: &Path, reg: &PluginRegistry) -> Result<(), String> {
    let path = registry_path_from(apps_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建插件目录失败: {e}"))?;
    }
    let data = serde_json::to_string_pretty(reg).map_err(|e| format!("序列化注册表失败: {e}"))?;
    std::fs::write(path, data).map_err(|e| format!("写入注册表失败: {e}"))
}

fn read_manifest(dir: &Path) -> Option<PluginManifest> {
    let data = std::fs::read(dir.join("manifest.json")).ok()?;
    serde_json::from_slice(&data).ok()
}

// ============================================================================
// 安装器
// ============================================================================

/// 从 .nuph（ZIP）安装插件到 apps_root（内部实现，路径可注入以便测试）。
///
/// 流程（契约 §3 + §6.2）：
/// 1. 预扫描：文件数 / 总尺寸 / manifest（≤64KB，先于任何解压动作读取）
/// 2. manifest 校验（id/version/entry/permissions/sidecar/minHost）
/// 3. 同 id 已存在 → 新版本必须严格更高，否则拒绝
/// 4. 解压到临时目录（enclosed_name 规范化防 zip-slip + 实际写入总量复核）
/// 5. 原子改名到 plugin/apps/{id}/，写注册表（enabled=true）
fn install_zip_to(apps_root: &Path, archive_path: &Path) -> Result<PluginManifest, String> {
    let file = std::fs::File::open(archive_path).map_err(|e| format!("打开插件包失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("插件包不是合法 ZIP: {e}"))?;

    if archive.len() > MAX_ARCHIVE_FILES {
        return Err(format!("插件包文件数超过上限 {MAX_ARCHIVE_FILES}"));
    }

    // ── 预扫描：总尺寸 + manifest 首读 ──
    let mut total: u64 = 0;
    let mut manifest_bytes: Option<Vec<u8>> = None;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| format!("ZIP 条目路径非法（含 .. 逃逸）: {}", entry.name()))?;
        total = total.saturating_add(entry.size());
        if total > MAX_ARCHIVE_TOTAL_BYTES {
            return Err(format!(
                "插件包解压后总尺寸超过上限 {MAX_ARCHIVE_TOTAL_BYTES} 字节"
            ));
        }
        if manifest_bytes.is_none() && name == Path::new("manifest.json") {
            if entry.size() > MAX_MANIFEST_BYTES as u64 {
                return Err(format!("manifest.json 超过上限 {MAX_MANIFEST_BYTES} 字节"));
            }
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("读取 manifest.json 失败: {e}"))?;
            manifest_bytes = Some(buf);
        }
    }
    let manifest_bytes = manifest_bytes.ok_or_else(|| "插件包缺少 manifest.json".to_string())?;
    let manifest: PluginManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("manifest.json 解析失败: {e}"))?;
    validate_manifest(&manifest)?;

    // ── 版本升级语义：同 id 仅允许严格更高 ──
    let reg = load_registry_from(apps_root);
    let target_dir = apps_root.join(&manifest.id);
    if let Some(existing) = reg.get(&manifest.id) {
        match compare_versions(&manifest.version, &existing.version) {
            Some(std::cmp::Ordering::Greater) => {}
            Some(_) => {
                return Err("已安装同版本或更高版本，请先卸载或提供更高版本".to_string());
            }
            None => return Err(format!("新版本号非法: {}", manifest.version)),
        }
    }

    // ── 解压到临时目录（成功后原子改名）──
    let tmp_dir = apps_root.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("创建临时目录失败: {e}"))?;

    let extract_result = (|| -> Result<(), String> {
        let mut written: u64 = 0;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry
                .enclosed_name()
                .ok_or_else(|| format!("ZIP 条目路径非法（含 .. 逃逸）: {}", entry.name()))?;
            if name.as_os_str().is_empty() {
                continue;
            }
            let out_path = tmp_dir.join(&name);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
            }
            let mut buf = Vec::with_capacity(entry.size().min(MAX_ARCHIVE_TOTAL_BYTES) as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("解压 {} 失败: {e}", entry.name()))?;
            written = written.saturating_add(buf.len() as u64);
            if written > MAX_ARCHIVE_TOTAL_BYTES {
                return Err(format!(
                    "解压后总尺寸超过上限 {MAX_ARCHIVE_TOTAL_BYTES} 字节"
                ));
            }
            std::fs::write(&out_path, &buf)
                .map_err(|e| format!("写入 {} 失败: {e}", name.display()))?;
        }
        Ok(())
    })();

    if let Err(e) = extract_result {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }

    // ── 原子替换（升级语义）：tmp 与 target 同 apps_root 下，rename 同卷可靠 ──
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir).map_err(|e| format!("移除旧版本目录失败: {e}"))?;
    }
    std::fs::rename(&tmp_dir, &target_dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        format!("移动插件目录失败: {e}")
    })?;

    // ── 写注册表 ──
    let mut reg = load_registry_from(apps_root);
    reg.insert(
        manifest.id.clone(),
        PluginRegistryEntry {
            version: manifest.version.clone(),
            enabled: true,
            installed_at: chrono::Utc::now().to_rfc3339(),
            permissions: manifest.permissions.clone(),
        },
    );
    save_registry_to(apps_root, &reg)?;

    Ok(manifest)
}

// ============================================================================
// Tauri commands：安装器（4）
// ============================================================================

#[tauri::command]
pub fn plugin_app_install(path: String) -> Result<PluginManifest, String> {
    install_zip_to(&apps_root(), Path::new(&path))
}

/// 市场安装（设计文档 §8）：Rust 侧下载 .nuph 到临时文件 → 复用 §6.2 安装器。
/// 原子语义：下载失败/超限/校验失败均不产生任何注册表或目录残留。
/// 完整性：expected_sha256 由市场索引条目提供（可选）。提供时下载后强制比对，
/// 不匹配即失败——索引与包同源（GitHub Pages https），用于防包体被替换。
/// 注意：数字签名校验为路线图项（v1.1），当前无签名；zip-slip 与解压大小上限
/// 由安装器强制（install_zip_to）。
/// 市场为可选特性（feature = "market"，默认关闭）：开源基础版不含市场，命令不编译。
#[cfg(feature = "market")]
#[tauri::command]
pub async fn plugin_market_install(
    url: String,
    expected_sha256: Option<String>,
) -> Result<PluginManifest, String> {
    // 市场源仅允许 https——http 明文可被中间人替换包体，不再放行
    if !url.starts_with("https://") {
        return Err(format!("非法下载地址（仅支持 https）: {url}"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;
    let mut resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载插件包失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载插件包失败: HTTP {}", resp.status()));
    }
    let tmp = std::env::temp_dir().join(format!(
        "nuphus-market-{}.nuph",
        uuid::Uuid::new_v4().simple()
    ));
    let result: Result<PluginManifest, String> = async {
        let mut file = std::fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败: {e}"))?;
        // 流式写入 + 硬上限（对齐安装器解压总量上限），超限即中止
        let mut written: u64 = 0;
        loop {
            match resp.chunk().await.map_err(|e| format!("下载中断: {e}"))? {
                Some(chunk) => {
                    written += chunk.len() as u64;
                    if written > MAX_ARCHIVE_TOTAL_BYTES {
                        return Err(format!("插件包超过大小上限 {MAX_ARCHIVE_TOTAL_BYTES} 字节"));
                    }
                    file.write_all(&chunk)
                        .map_err(|e| format!("写入临时文件失败: {e}"))?;
                }
                None => break,
            }
        }
        drop(file);
        // 可选 sha256 校验：索引提供时强制比对，防包体被替换
        if let Some(expected) = expected_sha256.as_deref() {
            let actual = sha256_file(&tmp).map_err(|e| format!("计算插件包校验值失败: {e}"))?;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err("插件包完整性校验失败（sha256 不匹配，可能已被篡改）".to_string());
            }
        }
        install_zip_to(&apps_root(), &tmp)
    }
    .await;
    let _ = std::fs::remove_file(&tmp);
    result
}

/// 计算文件 SHA-256（hex 小写）。完整性校验用，非认证（无密钥）。
/// 仅市场安装（plugin_market_install）使用——随 market feature 编译。
#[cfg(feature = "market")]
fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hex::encode(hasher.finalize()))
}

/// 插件摘要（plugin_app_list 返回；字段名 camelCase 对齐前端）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAppSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 图标文件名（如 icon.png）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 展示分类（manifest 缺失/未知 → 前端归入 other）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// 详情视图展示：开发者 / 主页
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// 官方示例标记（manifest sample=true 透传；前端据此渲染「示例」徽章）
    pub sample: bool,
    pub permissions: Vec<String>,
    pub enabled: bool,
    pub installed_at: String,
}

/// 插件可见的工作流摘要（plugin_workflow_list 返回；camelCase 对齐 PluginAppSummary 风格）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginWorkflowSummary {
    pub id: String,
    pub name: String,
    /// 工作流生命周期状态（Draft/Ready/Running/Completed/Error，与 index.json 一致）
    pub status: String,
    pub step_count: usize,
}

/// workflow.run 终态结果：status ∈ "completed" / "failed"，失败时 error 携带原因。
/// 300s 硬超时不作为终态返回——命令级 Err，前端映射 TIMEOUT 信封。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginWorkflowRunResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[tauri::command]
pub fn plugin_app_list() -> Result<Vec<PluginAppSummary>, String> {
    let apps_root = apps_root();
    let reg = load_registry_from(&apps_root);
    let mut out: Vec<PluginAppSummary> = Vec::with_capacity(reg.len());
    for (id, entry) in &reg {
        let manifest = read_manifest(&apps_root.join(id));
        out.push(PluginAppSummary {
            id: id.clone(),
            name: manifest
                .as_ref()
                .map(|m| m.name.clone())
                .unwrap_or_else(|| id.clone()),
            version: entry.version.clone(),
            description: manifest.as_ref().and_then(|m| m.description.clone()),
            icon: manifest.as_ref().and_then(|m| m.icon.clone()),
            category: manifest.as_ref().and_then(|m| m.category.clone()),
            author: manifest.as_ref().and_then(|m| m.author.clone()),
            homepage: manifest.as_ref().and_then(|m| m.homepage.clone()),
            sample: manifest.as_ref().map(|m| m.sample).unwrap_or(false),
            permissions: entry.permissions.clone(),
            enabled: entry.enabled,
            installed_at: entry.installed_at.clone(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[tauri::command]
pub fn plugin_app_uninstall(id: String) -> Result<(), String> {
    // id 双重防线：注册表 key 必为安装时校验过的合法 id，此处再校验防调用方直传逃逸
    if !valid_plugin_id(&id) {
        return Err(format!("插件 id 非法: {id}"));
    }
    let apps_root = apps_root();
    let mut reg = load_registry_from(&apps_root);
    if !reg.contains_key(&id) {
        return Err(format!("插件 {id} 未安装"));
    }
    let dir = apps_root.join(&id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除插件目录失败: {e}"))?;
    }
    reg.remove(&id);
    save_registry_to(&apps_root, &reg)
}

#[tauri::command]
pub fn plugin_app_set_enabled(id: String, enabled: bool) -> Result<(), String> {
    if !valid_plugin_id(&id) {
        return Err(format!("插件 id 非法: {id}"));
    }
    let apps_root = apps_root();
    let mut reg = load_registry_from(&apps_root);
    let entry = reg
        .get_mut(&id)
        .ok_or_else(|| format!("插件 {id} 未安装"))?;
    entry.enabled = enabled;
    save_registry_to(&apps_root, &reg)
}

// ============================================================================
// Tauri commands：打包导出（1）
// ============================================================================

/// 递归收集 dir 下全部文件，返回 (zip 内相对路径[正斜杠], 磁盘绝对路径)
/// 相对路径以 src_dir 为基准（递归子目录时 strip_prefix 必须用根目录，否则层级丢失）
fn collect_plugin_files(
    src_dir: &Path,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取插件目录失败: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取插件目录条目失败: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_plugin_files(src_dir, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(src_dir)
                .map_err(|_| "插件目录路径计算失败".to_string())?;
            let name = rel.to_string_lossy().replace('\\', "/");
            out.push((name, path));
        }
    }
    Ok(())
}

/// 将已安装插件目录打包为 .nuph（ZIP deflate；zip 内相对路径、正斜杠）
fn pack_plugin_dir(src_dir: &Path, dest_path: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_plugin_files(src_dir, src_dir, &mut files)?;
    // 运行时数据不进导出包：kv.json 是本机用户数据（创作侧导出闭环只分发代码与静态资源）
    files.retain(|(name, _)| name != "kv.json");
    if files.is_empty() {
        return Err("插件目录为空，无可打包内容".to_string());
    }

    let file = std::fs::File::create(dest_path).map_err(|e| format!("创建插件包失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    // manifest.json 置首（可读性 + 与安装器预扫描语义一致）；其余按相对路径排序保证输出稳定
    files.sort_by(|a, b| {
        let a_first = a.0 == "manifest.json";
        let b_first = b.0 == "manifest.json";
        b_first.cmp(&a_first).then_with(|| a.0.cmp(&b.0))
    });
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, path) in &files {
        let data =
            std::fs::read(path).map_err(|e| format!("读取插件文件失败 {}: {e}", path.display()))?;
        zip.start_file(name.as_str(), options)
            .map_err(|e| format!("写入 ZIP 条目失败 {name}: {e}"))?;
        zip.write_all(&data)
            .map_err(|e| format!("写入 ZIP 数据失败 {name}: {e}"))?;
    }
    zip.finish().map_err(|e| format!("写入插件包失败: {e}"))?;
    Ok(())
}

/// 将已安装插件目录导出为 .nuph 包（创作侧导出闭环：安装 → 打包分享）
#[tauri::command]
pub fn plugin_app_pack(id: String, dest_path: String) -> Result<(), String> {
    if !valid_plugin_id(&id) {
        return Err(format!("插件 id 非法: {id}"));
    }
    let apps_root = apps_root();
    let reg = load_registry_from(&apps_root);
    if !reg.contains_key(&id) {
        return Err(format!("插件 {id} 未安装"));
    }
    let dir = apps_root.join(&id);
    if !dir.is_dir() {
        return Err(format!("插件目录不存在: {}", dir.display()));
    }
    let dest = Path::new(&dest_path);
    let parent = dest
        .parent()
        .ok_or_else(|| "目标路径缺少父目录".to_string())?;
    if !parent.is_dir() {
        return Err(format!("目标目录不存在: {}", parent.display()));
    }
    pack_plugin_dir(&dir, dest)
}

// ============================================================================
// Tauri command：示例工程导出（插件页开发者入口）
// 模板编译期内嵌（include_str!，路径相对本文件 src-tauri/src/ → 仓库根 examples/），
// 安装包形态下不依赖仓内路径——离线可用。
// ============================================================================

/// 示例工程白名单模板（key = 前端 sample_id；每个工程 3 文件：manifest + 入口 + README）
const SAMPLE_TEMPLATES: &[(&str, &[(&str, &str)])] = &[
    (
        "hello-plugin",
        &[
            (
                "manifest.json",
                include_str!("../../examples/hello-plugin/manifest.json"),
            ),
            (
                "icon.svg",
                include_str!("../../examples/hello-plugin/icon.svg"),
            ),
            (
                "index.html",
                include_str!("../../examples/hello-plugin/index.html"),
            ),
            (
                "README.md",
                include_str!("../../examples/hello-plugin/README.md"),
            ),
        ],
    ),
    (
        "agent-chat",
        &[
            (
                "manifest.json",
                include_str!("../../examples/agent-chat/manifest.json"),
            ),
            (
                "icon.svg",
                include_str!("../../examples/agent-chat/icon.svg"),
            ),
            (
                "index.html",
                include_str!("../../examples/agent-chat/index.html"),
            ),
            (
                "README.md",
                include_str!("../../examples/agent-chat/README.md"),
            ),
        ],
    ),
];

/// 导出官方示例工程到 dest_dir/{sample_id}/（开发者入口）。
/// 白名单校验：sample_id ∈ {hello-plugin, agent-chat}，非法拒绝；
/// 目标目录已存在且非空 → 拒绝覆盖（防止误伤用户已有工作）。
/// 返回写出目录的绝对路径。
#[tauri::command]
pub fn plugin_export_sample(sample_id: String, dest_dir: String) -> Result<String, String> {
    let template = SAMPLE_TEMPLATES
        .iter()
        .find(|(id, _)| *id == sample_id)
        .ok_or_else(|| format!("未知示例工程: {sample_id}（仅支持 hello-plugin / agent-chat）"))?;

    let base = Path::new(&dest_dir);
    if !base.is_dir() {
        return Err(format!("目标目录不存在: {}", base.display()));
    }
    let out = base.join(&sample_id);
    if out.exists() {
        let non_empty = std::fs::read_dir(&out)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
        if non_empty {
            return Err(format!("目标目录已存在且非空，拒绝覆盖: {}", out.display()));
        }
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("创建目录失败 {}: {e}", out.display()))?;
    for (name, content) in template.1 {
        std::fs::write(out.join(name), content)
            .map_err(|e| format!("写入文件失败 {}: {e}", name))?;
    }
    Ok(out.to_string_lossy().into_owned())
}

// ============================================================================
// Tauri commands：插件 KV（4）
// ============================================================================

/// 插件前置校验：id 合法 + 已注册 + enabled
/// （KV 写数据防未安装插件 + agent.chat 调用共用；防调用方直传 id 逃逸）
fn ensure_plugin_enabled(apps_root: &Path, id: &str) -> Result<(), String> {
    if !valid_plugin_id(id) {
        return Err(format!("插件 id 非法: {id}"));
    }
    let reg = load_registry_from(apps_root);
    let entry = reg.get(id).ok_or_else(|| format!("插件 {id} 未安装"))?;
    if !entry.enabled {
        return Err(format!("插件 {id} 已禁用"));
    }
    Ok(())
}

fn kv_path_from(apps_root: &Path, id: &str) -> PathBuf {
    apps_root.join(id).join("kv.json")
}

fn load_kv(apps_root: &Path, id: &str) -> HashMap<String, serde_json::Value> {
    std::fs::read_to_string(kv_path_from(apps_root, id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_kv(
    apps_root: &Path,
    id: &str,
    kv: &HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    let path = kv_path_from(apps_root, id);
    let data = serde_json::to_vec(kv).map_err(|e| format!("序列化 KV 失败: {e}"))?;
    if data.len() as u64 > MAX_KV_FILE_BYTES {
        return Err(format!("插件 KV 数据总量超过上限 {MAX_KV_FILE_BYTES} 字节"));
    }
    std::fs::write(&path, data).map_err(|e| format!("写入 KV 失败: {e}"))
}

/// KV 内部实现（apps_root 可注入以便测试；command 层为薄壳）
fn kv_get_at(apps_root: &Path, id: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    ensure_plugin_enabled(apps_root, id)?;
    Ok(load_kv(apps_root, id).get(key).cloned())
}

fn kv_set_at(
    apps_root: &Path,
    id: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    ensure_plugin_enabled(apps_root, id)?;
    if key.is_empty() {
        return Err("KV key 不能为空".to_string());
    }
    let serialized = serde_json::to_vec(&value).map_err(|e| format!("序列化 KV 值失败: {e}"))?;
    if serialized.len() > MAX_KV_VALUE_BYTES {
        return Err(format!("KV 单值超过上限 {MAX_KV_VALUE_BYTES} 字节"));
    }
    let mut kv = load_kv(apps_root, id);
    kv.insert(key.to_string(), value);
    save_kv(apps_root, id, &kv)
}

fn kv_delete_at(apps_root: &Path, id: &str, key: &str) -> Result<(), String> {
    ensure_plugin_enabled(apps_root, id)?;
    let mut kv = load_kv(apps_root, id);
    kv.remove(key);
    save_kv(apps_root, id, &kv)
}

fn kv_keys_at(apps_root: &Path, id: &str) -> Result<Vec<String>, String> {
    ensure_plugin_enabled(apps_root, id)?;
    let mut keys: Vec<String> = load_kv(apps_root, id).into_keys().collect();
    keys.sort();
    Ok(keys)
}

#[tauri::command]
pub fn plugin_kv_get(id: String, key: String) -> Result<Option<serde_json::Value>, String> {
    kv_get_at(&apps_root(), &id, &key)
}

#[tauri::command]
pub fn plugin_kv_set(id: String, key: String, value: serde_json::Value) -> Result<(), String> {
    kv_set_at(&apps_root(), &id, &key, value)
}

#[tauri::command]
pub fn plugin_kv_delete(id: String, key: String) -> Result<(), String> {
    kv_delete_at(&apps_root(), &id, &key)
}

#[tauri::command]
pub fn plugin_kv_keys(id: String) -> Result<Vec<String>, String> {
    kv_keys_at(&apps_root(), &id)
}

// ============================================================================
// Tauri commands：主题快照（1）
// ============================================================================

/// 保存当前生效主题快照到 AppState 内存（/plugins-shared/theme.css 渲染源）。
/// useTheme 任一主题变化（内置切换 / 自定义保存 / 滑块拖动）时调用。
#[tauri::command]
pub fn theme_snapshot_save(
    state: tauri::State<'_, AppState>,
    base: String,
    overrides: HashMap<String, String>,
) -> Result<(), String> {
    theme_snapshot_save_inner(state.inner(), &base, &overrides)
}

/// 内部实现（AppState 可注入以便测试；command 层为薄壳）
fn theme_snapshot_save_inner(
    state: &AppState,
    base: &str,
    overrides: &HashMap<String, String>,
) -> Result<(), String> {
    if base.trim().is_empty() {
        return Err("主题 base 不能为空".to_string());
    }
    if overrides.len() > MAX_THEME_OVERRIDES {
        return Err(format!("主题覆盖条目超过上限 {MAX_THEME_OVERRIDES}"));
    }
    for (k, v) in overrides {
        if !k.starts_with("--") {
            return Err(format!("主题覆盖 key 必须以 -- 开头: {k}"));
        }
        if v.chars().count() > MAX_THEME_VALUE_CHARS {
            return Err(format!("主题覆盖值超过 {MAX_THEME_VALUE_CHARS} 字符: {k}"));
        }
    }
    let mut snap = state
        .theme_snapshot
        .lock()
        .map_err(|e| format!("锁异常: {e}"))?;
    snap.base = base.to_string();
    snap.overrides = overrides.clone();
    Ok(())
}

// ============================================================================
// Tauri commands：agent.chat（1）——插件可调 Agent（契约 §5.3，独立运行时隔离）
// ============================================================================

/// 每插件在途 chat guard：acquire 成功时在 AppState.plugin_chat_inflight 登记，
/// Drop（含 panic/async future 取消）时移除——保证在途标记绝不泄漏。
struct PluginChatGuard<'a> {
    state: &'a AppState,
    id: String,
}

impl<'a> PluginChatGuard<'a> {
    fn acquire(state: &'a AppState, id: &str) -> Result<Self, String> {
        let mut inflight = state
            .plugin_chat_inflight
            .lock()
            .map_err(|e| format!("锁异常: {e}"))?;
        if !inflight.insert(id.to_string()) {
            return Err("上一个对话进行中".to_string());
        }
        Ok(Self {
            state,
            id: id.to_string(),
        })
    }
}

impl Drop for PluginChatGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut inflight) = self.state.plugin_chat_inflight.lock() {
            inflight.remove(&self.id);
        }
    }
}

/// agent.chat 请求校验（apps_root 可注入以便测试）：
/// 已注册 + enabled → manifest 声明 agent.chat 权限（纵深防御，桥接器才是主鉴权点）
/// → 消息非空且 ≤32KB。
fn validate_agent_chat_request(apps_root: &Path, id: &str, message: &str) -> Result<(), String> {
    ensure_plugin_enabled(apps_root, id)?;
    let manifest =
        read_manifest(&apps_root.join(id)).ok_or_else(|| format!("插件 {id} manifest 缺失"))?;
    if !manifest.permissions.iter().any(|p| p == "agent.chat") {
        return Err(format!("插件 {id} 未声明 agent.chat 权限"));
    }
    if message.trim().is_empty() {
        return Err("消息不能为空".to_string());
    }
    if message.len() > MAX_PLUGIN_CHAT_BYTES {
        return Err(format!("消息超过 {MAX_PLUGIN_CHAT_BYTES} 字节上限"));
    }
    Ok(())
}

/// 全局并发闸：插件侧执行（agent.chat + workflow.run）并发上限 2，第三路排队等待而非拒绝。
/// 模块级 OnceLock——进程生命周期内仅初始化一次。agent.chat 与 workflow.run 共用同一闸：
/// 插件触发的一切重执行共享 2 路额度，避免叠加击穿底层 LLM/工具资源。
static PLUGIN_CHAT_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

fn plugin_chat_semaphore() -> &'static tokio::sync::Semaphore {
    PLUGIN_CHAT_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(PLUGIN_CHAT_MAX_CONCURRENT))
}

/// 插件独立运行时专用事件接收器：事件不进主通道（桌面 Tauri IPC / 手机 WS），
/// 但**写入后端 tracing 日志**——隔离的是 UI 通道，不是可观测性。
/// 插件执行黑盒 = 无法诊断卡顿/失败；生命周期与工具事件必须留痕。
/// 最终回复经 AgentOutput.message 收集，不依赖事件流。
#[derive(Clone)]
struct LoggingEmitter {
    plugin_id: String,
}

impl EventEmitter for LoggingEmitter {
    fn emit(&self, event: NuphusEvent) {
        let id = &self.plugin_id;
        match event {
            NuphusEvent::ExecutionStarted { .. } => {
                tracing::info!("[plugin:{id}] 执行开始");
            }
            NuphusEvent::ToolCallStart {
                tool_name,
                iteration,
                ..
            } => {
                tracing::info!("[plugin:{id}] 工具调用开始 iter={iteration} tool={tool_name}");
            }
            NuphusEvent::ToolCallEnd {
                tool_name,
                success,
                duration_ms,
                ..
            } => {
                tracing::info!(
                    "[plugin:{id}] 工具调用结束 tool={tool_name} ok={success} {duration_ms}ms"
                );
            }
            NuphusEvent::ExecutionCompleted {
                total_duration_ms,
                total_calls,
                ..
            } => {
                tracing::info!("[plugin:{id}] 执行完成 calls={total_calls} {total_duration_ms}ms");
            }
            NuphusEvent::ExecutionError { error, .. } => {
                tracing::warn!("[plugin:{id}] 执行错误: {error}");
            }
            // LLM 重试/连接状态等警告（429 退避循环的唯一可见信号）
            NuphusEvent::Warning { code, message } => {
                tracing::warn!("[plugin:{id}] 警告[{code}]: {message}");
            }
            // LlmTextDelta 等高频/流式事件静默——日志噪声控制
            _ => {}
        }
    }
}

/// agent.chat history 校验（后端纵深防御；前端桥接器为主防线，超限均返回 INVALID_PARAMS）：
/// 元素 {role, content} 形状（role ∈ user/assistant/system），≤50 条、总字符 ≤64KB。
fn validate_agent_chat_history(history: &Option<Vec<HistoryMessage>>) -> Result<(), String> {
    let Some(h) = history else {
        return Ok(());
    };
    if h.len() > MAX_PLUGIN_CHAT_HISTORY {
        return Err(format!("history 超过 {MAX_PLUGIN_CHAT_HISTORY} 条上限"));
    }
    let mut total = 0usize;
    for m in h {
        if m.role != "user" && m.role != "assistant" && m.role != "system" {
            return Err(format!("history 元素 role 非法: {}", m.role));
        }
        if m.content.trim().is_empty() {
            return Err("history 元素 content 不能为空".to_string());
        }
        // 字符数统计（与前端 UTF-16 单位语义对齐；BMP 内 1 字符 = 1 单位）
        total += m.role.chars().count() + m.content.chars().count();
        if total > MAX_PLUGIN_CHAT_HISTORY_CHARS {
            return Err(format!(
                "history 总字符超过 {MAX_PLUGIN_CHAT_HISTORY_CHARS} 上限"
            ));
        }
    }
    Ok(())
}

/// 独立运行时执行体（与主会话共享态零接触）——可测试核心：
/// - 每调用新建 Runtime（无池化）；LoggingEmitter 事件只进 tracing 日志，不碰主窗口/手机
/// - 独立 cancel_flag；不注册 pause_flag、不入 mobile_append、不写 session backup
/// - history 透传注入会话（调用方自管上下文，服务端零会话状态）
/// - 整体 tokio::time::timeout 硬兜底（IPC break / command drop 即停）
async fn run_plugin_chat_isolated(
    plugin_id: &str,
    tools: nuphus::ToolRegistry,
    llm: Arc<dyn ApiClient>,
    model: String,
    provider: String,
    tool_permissions: nuphus::permissions::ToolPermissions,
    tool_permissions_ref: Arc<std::sync::Mutex<nuphus::permissions::ToolPermissions>>,
    message: &str,
    history: &Option<Vec<HistoryMessage>>,
    source: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    let started = std::time::Instant::now();
    tracing::info!(
        "[plugin:{plugin_id}] agent.chat 开始 model={model} provider={provider} msg_chars={} history={} 条",
        message.chars().count(),
        history.as_ref().map(|h| h.len()).unwrap_or(0),
    );
    // Leader 工具集（与主 Agent 同源：state.tools 派生 + 桌面客户端 + 共享信号）
    let mut leader_tools = if let Some(dc) = tools.desktop_client() {
        nuphus::ToolRegistry::leader_with_desktop(dc)
    } else {
        nuphus::ToolRegistry::leader()
    };
    leader_tools.set_signals(tools.signals().clone());

    // 独立 cancel_flag：每调用新实例；command drop / IPC 断 → future 整体取消即停
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let run_fut = async {
        let mut runtime = RuntimeBuilder::new()
            .llm(llm.clone())
            .tools(leader_tools)
            .config(RuntimeConfig {
                mode: Mode::Leader,
                agent_config: nuphus::agent::AgentConfig {
                    model: model.clone(),
                    provider: provider.clone(),
                    tool_permissions,
                    ..Default::default()
                },
                // 共享实时权限引用（与主 Agent 同源，用户全局设置变更即时生效）
                tool_permissions: tool_permissions_ref,
                ..Default::default()
            })
            .emitter(Arc::new(LoggingEmitter {
                plugin_id: plugin_id.to_string(),
            }))
            .build()?;

        // 历史上下文注入（与 submit 路径同语义：text-only Session::from_history）
        if let Some(h) = history {
            if !h.is_empty() {
                let tuples: Vec<(String, String)> = h
                    .iter()
                    .map(|m| (m.role.clone(), m.content.clone()))
                    .collect();
                runtime.set_session(Session::from_history(tuples));
            }
        }

        runtime.set_source(source);
        runtime
            .run(message, &None, &cancel_flag)
            .await
            .map_err(|e| e.to_string())
    };

    let output = match tokio::time::timeout(timeout, run_fut).await {
        Ok(res) => res.map_err(|e| {
            tracing::warn!(
                "[plugin:{plugin_id}] agent.chat 执行失败 elapsed={}ms: {e}",
                started.elapsed().as_millis()
            );
            e
        })?,
        Err(_) => {
            tracing::warn!(
                "[plugin:{plugin_id}] agent.chat 超时（{}s），已强制终止",
                timeout.as_secs()
            );
            return Err(format!(
                "Agent 执行超过 {}s 未完成，已强制终止（前端 120s 超时返回后由后端收尸）",
                timeout.as_secs()
            ));
        }
    };

    // 空回复兜底（与主路径空响应守卫语义一致）
    let mut reply = output.message;
    if reply.trim().is_empty() && output.success {
        reply = "（模型未产出有效回复，可能需重试）".to_string();
    }
    if reply.trim().is_empty() {
        tracing::warn!(
            "[plugin:{plugin_id}] agent.chat 失败（无有效输出），elapsed={}ms",
            started.elapsed().as_millis()
        );
        return Err("Agent 执行失败（无有效输出）".to_string());
    }
    tracing::info!(
        "[plugin:{plugin_id}] agent.chat 完成 elapsed={}ms reply_chars={}",
        started.elapsed().as_millis(),
        reply.chars().count()
    );
    Ok(reply)
}

/// 内部实现（AppState/apps_root 可注入以便测试；command 层为薄壳）。
/// 独立 Agent 运行时：不复用共享入口 submit_user_message——
/// LLM client 由 config.toml ModelRegistry 构建（只读），tools/权限与用户全局同源；
/// LoggingEmitter 事件只进 tracing 日志（UI 通道隔离），不碰 state.busy/runtime/session（插件失控不阻塞/污染主会话）。
/// 每插件串行（PluginChatGuard）+ 全局并发闸（Semaphore=2，第三路排队等待）。
async fn plugin_agent_chat_inner(
    state: &AppState,
    apps_root: &Path,
    id: &str,
    message: &str,
    history: Option<Vec<HistoryMessage>>,
) -> Result<String, String> {
    validate_agent_chat_request(apps_root, id, message)?;
    validate_agent_chat_history(&history)?;
    let _guard = PluginChatGuard::acquire(state, id)?;
    // 全局并发闸：第 3 路排队等待（acquire 即等待，不拒绝）
    if plugin_chat_semaphore().available_permits() == 0 {
        tracing::info!("[plugin:{id}] agent.chat 排队等待并发闸（2 路在跑）");
    }
    let _permit = plugin_chat_semaphore()
        .acquire()
        .await
        .map_err(|e| format!("插件并发闸异常: {e}"))?;

    // ClientFactory 同法构建（与主路径 Priority 1 一致：完整 ModelRegistry 只读）
    let factory = nuphus::config::load_registry()
        .map(nuphus::llm::ClientFactory::new)
        .map_err(|e| format!("无法加载模型配置，请检查 config.toml: {e}"))?;
    let registry = factory.registry();
    // 单一模型解析入口：与主 Agent 相同的 leader 生效模型
    let model =
        crate::commands::config::llm::effective_model(&state.llm_config_path, registry, "leader");
    let llm = factory
        .create_client(&model)
        .map_err(|e| format!("创建 LLM 客户端失败 ({model}): {e}"))?;
    let provider = registry
        .find_model(&model)
        .map(|(p, _)| p.name.clone())
        .unwrap_or_default();

    // 与用户全局设置同源的 tool_permissions（只读快照 + 共享实时引用）
    let tool_permissions = *state
        .tool_permissions_ref
        .lock()
        .map_err(|e| format!("权限配置读取失败: {e}"))?;

    run_plugin_chat_isolated(
        id,
        state.tools.clone(),
        llm,
        model,
        provider,
        tool_permissions,
        state.tool_permissions_ref.clone(),
        message,
        &history,
        &format!("plugin:{id}"),
        PLUGIN_CHAT_TIMEOUT,
    )
    .await
}

/// 插件发消息给 Agent（独立运行时），同步等待最终回复（v1；流式增量 agent.delta 为 v1.1）。
#[tauri::command]
pub async fn plugin_agent_chat(
    state: tauri::State<'_, AppState>,
    id: String,
    message: String,
    history: Option<Vec<HistoryMessage>>,
) -> Result<String, String> {
    plugin_agent_chat_inner(state.inner(), &apps_root(), &id, &message, history).await
}

// ============================================================================
// Tauri commands：工作流 Bridge（workflow.list / workflow.run）
// 权限：两者均挂在单一 "workflow.run" 权限下（PERMISSIONS_ALLOWED 已含），禁止新增枚举。
// ============================================================================

/// 插件列出用户工作流（只读，不触发执行）。权限由桥接器逐次鉴权（has('workflow.run')）。
#[tauri::command]
pub async fn plugin_workflow_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PluginWorkflowSummary>, String> {
    let engine = state.workflow_engine.read().await;
    // 热刷新（对齐 workflow_run 工具：先 load_all 再读摘要），确保刚保存的工作流可见
    if let Err(e) = engine.store.load_all().await {
        tracing::warn!("[plugin_workflow_list] 热刷新失败: {e}");
    }
    let summaries = engine.store.list().await;
    Ok(summaries
        .into_iter()
        .map(|s| PluginWorkflowSummary {
            id: s.id,
            name: s.name,
            status: format!("{:?}", s.status),
            step_count: s.step_count,
        })
        .collect())
}

/// workflow.run 请求校验（apps_root 可注入以便测试）：
/// 已注册 + enabled → manifest 声明 workflow.run 权限（纵深防御，桥接器才是主鉴权点）
/// → workflow_id 非空。
fn validate_workflow_run_request(
    apps_root: &Path,
    id: &str,
    workflow_id: &str,
) -> Result<(), String> {
    ensure_plugin_enabled(apps_root, id)?;
    let manifest =
        read_manifest(&apps_root.join(id)).ok_or_else(|| format!("插件 {id} manifest 缺失"))?;
    if !manifest.permissions.iter().any(|p| p == "workflow.run") {
        return Err(format!("插件 {id} 未声明 workflow.run 权限"));
    }
    if workflow_id.trim().is_empty() {
        return Err("workflow_id 不能为空".to_string());
    }
    Ok(())
}

/// workflow.run 在途串行守卫：同插件上一个 run 未完 → BUSY（Drop 时自动移除）。
/// 与 PluginChatGuard 同模式；独立 inflight 集合——chat 与 workflow 互不阻塞。
struct PluginWorkflowGuard<'a> {
    state: &'a AppState,
    id: String,
}

impl<'a> PluginWorkflowGuard<'a> {
    fn acquire(state: &'a AppState, id: &str) -> Result<Self, String> {
        let mut inflight = state
            .plugin_workflow_inflight
            .lock()
            .map_err(|e| format!("锁异常: {e}"))?;
        if !inflight.insert(id.to_string()) {
            return Err("上一个工作流执行进行中".to_string());
        }
        Ok(Self {
            state,
            id: id.to_string(),
        })
    }
}

impl Drop for PluginWorkflowGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut inflight) = self.state.plugin_workflow_inflight.lock() {
            inflight.remove(&self.id);
        }
    }
}

/// 插件触发工作流执行（同步等待终态）。
///
/// 复用 execute_workflow 链路（与 workflow_run 工具 / wf_run 同源）：
/// 热刷新 store → 校验工作流存在 → 注入 LLM client/ToolRegistry（共用
/// commands::workflow::inject_workflow_runtime，禁止复制注入段）→ 读锁执行并 await 终态。
///
/// 透明语义（有意设计）：执行事件经 engine EventBus → 主窗口 workflow-event 正常推送，
/// 不传 SinkEmitter——工作流是用户资产，插件触发必须在用户视野内，禁止静默执行。
///
/// 限流对齐 agent.chat：per-plugin 在途串行（PluginWorkflowGuard）+ 全局并发闸
/// （与 agent.chat 共用 plugin_chat_semaphore，插件侧执行总量 ≤2，第 3 路排队等待）
/// + 300s 硬超时收尸（前端 120s 先行返回 TIMEOUT 信封）。
async fn plugin_workflow_run_inner(
    state: &AppState,
    apps_root: &Path,
    plugin_id: &str,
    workflow_id: &str,
) -> Result<PluginWorkflowRunResult, String> {
    validate_workflow_run_request(apps_root, plugin_id, workflow_id)?;
    let _guard = PluginWorkflowGuard::acquire(state, plugin_id)?;
    // 全局并发闸：与 agent.chat 共用同一闸，第 3 路排队等待（acquire 即等待，不拒绝）
    if plugin_chat_semaphore().available_permits() == 0 {
        tracing::info!("[plugin:{plugin_id}] workflow.run 排队等待并发闸（2 路在跑）");
    }
    let _permit = plugin_chat_semaphore()
        .acquire()
        .await
        .map_err(|e| format!("插件并发闸异常: {e}"))?;

    let engine = state.workflow_engine.clone();
    // 热刷新 + 存在性校验 + 注入（写锁区间收窄到这段；执行走读锁，允许并发 pause/cancel）
    {
        let mut engine_w = engine.write().await;
        if let Err(e) = engine_w.store.load_all().await {
            tracing::warn!("[plugin:{plugin_id}] workflow.run 热刷新失败: {e}");
        }
        let exists = engine_w
            .store
            .list()
            .await
            .iter()
            .any(|s| s.id == workflow_id);
        if !exists {
            return Err(format!("工作流 {workflow_id} 不存在"));
        }
        inject_workflow_runtime(state, &mut engine_w);
    }

    // 执行并同步等待终态（复用 workflow_run 工具路径：读锁 + tool_exec + execute_workflow）
    let engine_r = engine.read().await;
    let tools = state.tools.clone();
    let tool_exec = move |tool: String, params: serde_json::Value| {
        let tools = tools.clone();
        async move {
            // browser_ 工具需走异步入口（ToolRegistry::execute 会拒绝）
            let result = if tool.starts_with("browser_") {
                tools.execute_browser_tool(&tool, &params).await
            } else {
                tools.execute(&tool, &params).await
            }
            .map_err(|e| e.to_string())?;
            result.into_exec_result()
        }
    };
    let tool_schemas = engine_r.tools().map(|t| t.get_schemas());

    match tokio::time::timeout(
        PLUGIN_WORKFLOW_TIMEOUT,
        engine_r.execute_workflow(workflow_id, tool_exec, tool_schemas, None, None),
    )
    .await
    {
        Ok(Ok(_msg)) => Ok(PluginWorkflowRunResult {
            status: "completed".to_string(),
            error: None,
        }),
        Ok(Err(e)) => Ok(PluginWorkflowRunResult {
            status: "failed".to_string(),
            error: Some(e.to_string()),
        }),
        Err(_) => Err(format!(
            "工作流 {workflow_id} 执行超时（{}s 硬超时）",
            PLUGIN_WORKFLOW_TIMEOUT.as_secs()
        )),
    }
}

/// 插件触发工作流执行（同步等待终态），返回 {status, error?}。
#[tauri::command]
pub async fn plugin_workflow_run(
    state: tauri::State<'_, AppState>,
    plugin_id: String,
    workflow_id: String,
) -> Result<PluginWorkflowRunResult, String> {
    plugin_workflow_run_inner(state.inner(), &apps_root(), &plugin_id, &workflow_id).await
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::Ordering;

    /// 独立临时目录（每次随机，避免并行测试互踩；不引 tempfile crate）
    fn temp_apps_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nuphus-test-{tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(dir.join("apps")).unwrap();
        dir.join("apps")
    }

    fn valid_manifest() -> PluginManifest {
        PluginManifest {
            id: "com.example.demo".to_string(),
            name: "Demo".to_string(),
            version: "1.0.0".to_string(),
            entry: "index.html".to_string(),
            icon: Some("icon.svg".to_string()),
            description: Some("demo plugin".to_string()),
            author: None,
            homepage: None,
            category: None,
            sample: false,
            min_host: None,
            permissions: vec!["kv".to_string()],
            sidecar: Some(serde_json::Value::Null),
        }
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    /// 构造一个最小合法 .nuph 包并写入临时文件
    fn write_plugin_package(dir: &Path, version: &str) -> PathBuf {
        let manifest = serde_json::json!({
            "id": "com.example.demo",
            "name": "Demo Plugin",
            "version": version,
            "entry": "index.html",
            "icon": "icon.svg",
            "permissions": ["kv"],
            "sidecar": null
        });
        let pkg = dir.join(format!("demo-{version}.nuph"));
        write_zip(
            &pkg,
            &[
                ("manifest.json", manifest.to_string().as_bytes()),
                ("icon.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
                ("index.html", b"<html><body>demo</body></html>"),
                ("assets/app.js", b"console.log('demo')"),
            ],
        );
        pkg
    }

    // ── manifest 校验 ──

    #[test]
    fn test_manifest_valid() {
        assert!(validate_manifest(&valid_manifest()).is_ok());
    }

    #[test]
    fn test_manifest_icon_required_svg() {
        // 缺图标 → 拒绝（市场准入基础要求）
        let mut m = valid_manifest();
        m.icon = None;
        let err = validate_manifest(&m).unwrap_err();
        assert!(err.contains("图标"), "应提示缺少图标: {err}");

        // 非 SVG（png/jpg/无扩展名）→ 拒绝
        for bad in [
            "icon.png",
            "icon.PNG",
            "favicon.jpg",
            "assets/app.ico",
            "icon",
        ] {
            let mut m = valid_manifest();
            m.icon = Some(bad.to_string());
            let err = validate_manifest(&m).unwrap_err();
            assert!(err.contains("SVG"), "非 SVG 应拒绝: {bad} → {err}");
        }

        // 路径逃逸 → 拒绝
        let mut m = valid_manifest();
        m.icon = Some("../icon.svg".to_string());
        assert!(validate_manifest(&m).is_err());

        // SVG 通过（含大写扩展名）
        for ok in ["icon.svg", "assets/icon.SVG", "icons/app-icon.svg"] {
            let mut m = valid_manifest();
            m.icon = Some(ok.to_string());
            assert!(validate_manifest(&m).is_ok(), "SVG 应通过: {ok}");
        }
    }

    #[test]
    fn test_manifest_bad_id() {
        for bad in [
            "Demo",             // 大写开头
            "1demo.com",        // 数字开头
            "com",              // 无点段
            "com.",             // 尾点
            ".com.demo",        // 首段空
            "com..demo",        // 空段
            "com/example",      // 非法字符
            "com.example.工具", // 非 ascii
        ] {
            let mut m = valid_manifest();
            m.id = bad.to_string();
            assert!(validate_manifest(&m).is_err(), "id 应拒绝: {bad}");
        }
        for ok in ["com.example", "a.b.c-d", "com.example.my-plugin2"] {
            let mut m = valid_manifest();
            m.id = ok.to_string();
            assert!(validate_manifest(&m).is_ok(), "id 应通过: {ok}");
        }
    }

    #[test]
    fn test_manifest_bad_version() {
        for bad in [
            "1.0",
            "1.0.0.0",
            "1.0-alpha",
            "a.b.c",
            "1..0",
            ".1.0",
            "1.0.",
        ] {
            let mut m = valid_manifest();
            m.version = bad.to_string();
            assert!(validate_manifest(&m).is_err(), "版本应拒绝: {bad}");
        }
    }

    #[test]
    fn test_manifest_entry_escape_rejected() {
        for bad in [
            "../evil.html",
            "a/../../evil.html",
            "/etc/passwd",
            "assets\\..\\evil.html",
            "a//b",
            "",
        ] {
            let mut m = valid_manifest();
            m.entry = bad.to_string();
            assert!(validate_manifest(&m).is_err(), "entry 应拒绝: {bad}");
        }
        for ok in ["index.html", "assets/app.js", "sub/index.html"] {
            let mut m = valid_manifest();
            m.entry = ok.to_string();
            assert!(validate_manifest(&m).is_ok(), "entry 应通过: {ok}");
        }
    }

    #[test]
    fn test_manifest_bad_permission() {
        let mut m = valid_manifest();
        m.permissions = vec!["kv".to_string(), "shell".to_string()];
        assert!(validate_manifest(&m).is_err());
        m.permissions = vec![
            "kv".to_string(),
            "notify".to_string(),
            "agent.chat".to_string(),
            "workflow.run".to_string(),
        ];
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn test_manifest_sidecar_nonnull_rejected() {
        let mut m = valid_manifest();
        m.sidecar = Some(serde_json::json!({ "mcp": { "command": "x" } }));
        assert!(validate_manifest(&m).is_err());
        // null 与缺失均合法
        assert!(validate_manifest(&valid_manifest()).is_ok());
        let mut m2 = valid_manifest();
        m2.sidecar = None;
        assert!(validate_manifest(&m2).is_ok());
    }

    #[test]
    fn test_manifest_sample_field_default() {
        // 缺省（无 sample 字段）→ false：Schema v1 向后兼容，旧 manifest 照常解析
        let without: PluginManifest = serde_json::from_str(
            r#"{"id":"com.example.demo","name":"D","version":"1.0.0","entry":"index.html","icon":"icon.svg","description":"d","permissions":["kv"],"sidecar":null}"#,
        )
        .unwrap();
        assert!(!without.sample, "缺省 sample 应为 false");
        // 显式 true → true（官方示例插件标记）
        let with: PluginManifest = serde_json::from_str(
            r#"{"id":"com.example.demo","name":"D","version":"1.0.0","entry":"index.html","icon":"icon.svg","description":"d","sample":true,"permissions":["kv"],"sidecar":null}"#,
        )
        .unwrap();
        assert!(with.sample, "显式 sample=true 应解析为 true");
        // 序列化缺省 false 不输出噪音字段
        let s = serde_json::to_string(&without).unwrap();
        assert!(!s.contains("sample"), "sample=false 序列化应跳过: {s}");
        // 显式 true 序列化保留（安装器导出 .nuph 时可保留示例标记）
        let s2 = serde_json::to_string(&with).unwrap();
        assert!(
            s2.contains("\"sample\":true"),
            "sample=true 序列化应保留: {s2}"
        );
        // 校验不受影响
        assert!(validate_manifest(&without).is_ok());
        assert!(validate_manifest(&with).is_ok());
    }

    #[test]
    fn test_manifest_min_host() {
        let mut m = valid_manifest();
        m.min_host = Some("9.9.9".to_string());
        assert!(validate_manifest(&m).is_err());
        m.min_host = Some("0.0.1".to_string());
        assert!(validate_manifest(&m).is_ok());
        m.min_host = Some(HOST_VERSION.to_string());
        assert!(validate_manifest(&m).is_ok());
        m.min_host = Some("abc".to_string());
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn test_manifest_category_optional() {
        // 无 category 字段 → None（旧包兼容）
        let m: PluginManifest = serde_json::from_str(
            r#"{"id":"com.example.demo","name":"D","version":"1.0.0","entry":"index.html","permissions":[]}"#,
        )
        .unwrap();
        assert!(m.category.is_none());

        // 有 category → 透传
        let m: PluginManifest = serde_json::from_str(
            r#"{"id":"com.example.demo","name":"D","version":"1.0.0","entry":"index.html","permissions":[],"category":"ai"}"#,
        )
        .unwrap();
        assert_eq!(m.category.as_deref(), Some("ai"));

        // 未知值宽容接受（不拒装，展示侧归入 other）
        let m: PluginManifest = serde_json::from_str(
            r#"{"id":"com.example.demo","name":"D","version":"1.0.0","entry":"index.html","permissions":[],"category":"unknown-x"}"#,
        )
        .unwrap();
        assert_eq!(m.category.as_deref(), Some("unknown-x"));

        // category 参与安装校验不拦截（validate_manifest 不感知该字段）
        let mut m = valid_manifest();
        m.category = Some("ai".to_string());
        assert!(validate_manifest(&m).is_ok());
        m.category = Some("unknown-x".to_string());
        assert!(validate_manifest(&m).is_ok());
    }

    // ── 安装：zip-slip / 硬限制 / 升级语义 ──

    #[test]
    fn test_zip_slip_rejected() {
        let apps = temp_apps_root("zipslip");
        let pkg = apps.parent().unwrap().join("evil.nuph");
        let manifest = serde_json::json!({
            "id": "com.example.evil",
            "name": "Evil",
            "version": "1.0.0",
            "entry": "index.html",
            "icon": "icon.svg",
            "permissions": [],
            "sidecar": null
        });
        write_zip(
            &pkg,
            &[
                ("manifest.json", manifest.to_string().as_bytes()),
                ("../evil.txt", b"pwned"),
            ],
        );
        let err = install_zip_to(&apps, &pkg).unwrap_err();
        assert!(
            err.contains("逃逸") || err.contains("非法"),
            "错误应指向路径逃逸: {err}"
        );
        // 目标目录不得被写出
        assert!(!apps.parent().unwrap().join("evil.txt").exists());
        assert!(!apps.join("com.example.evil").exists());
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_zip_missing_manifest_rejected() {
        let apps = temp_apps_root("nomanifest");
        let pkg = apps.parent().unwrap().join("nomanifest.nuph");
        write_zip(&pkg, &[("index.html", b"<html></html>")]);
        let err = install_zip_to(&apps, &pkg).unwrap_err();
        assert!(err.contains("manifest"), "应报缺少 manifest: {err}");
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    /// 调试安装链路实测：仓库示例 examples/hello-plugin 真实打包 → install_zip_to
    /// （与开发者中心第 4 步「选择 .nuph 安装」同一安装管线）
    #[test]
    fn test_install_examples_hello_plugin() {
        let sample_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/hello-plugin");
        assert!(
            sample_dir.join("manifest.json").exists(),
            "示例工程缺失: {}",
            sample_dir.display()
        );
        let apps = temp_apps_root("install-hello");
        let pkg = apps.parent().unwrap().join("hello-plugin.nuph");
        // 打包示例目录全部文件（相对路径、不套子目录——对齐 .nuph 契约 §3）
        let file = std::fs::File::create(&pkg).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for entry in std::fs::read_dir(&sample_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(&std::fs::read(&path).unwrap()).unwrap();
            }
        }
        zip.finish().unwrap();

        let manifest = install_zip_to(&apps, &pkg).unwrap();
        assert_eq!(manifest.id, "com.nuphus.hello");
        assert!(manifest.sample);
        assert!(apps.join("com.nuphus.hello").join("index.html").exists());
        assert!(apps.join("com.nuphus.hello").join("manifest.json").exists());
        let reg = load_registry_from(&apps);
        assert!(reg.get("com.nuphus.hello").unwrap().enabled);
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_install_ok_and_registry() {
        let apps = temp_apps_root("installok");
        let pkg_dir = apps.parent().unwrap();
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        let manifest = install_zip_to(&apps, &pkg).unwrap();
        assert_eq!(manifest.id, "com.example.demo");
        assert!(apps.join("com.example.demo").join("index.html").exists());
        assert!(apps.join("com.example.demo").join("assets/app.js").exists());
        assert!(!apps.join(".tmp-").exists());

        let reg = load_registry_from(&apps);
        let entry = reg.get("com.example.demo").unwrap();
        assert_eq!(entry.version, "1.0.0");
        assert!(entry.enabled);
        assert_eq!(entry.permissions, vec!["kv".to_string()]);
        assert!(!entry.installed_at.is_empty());
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_install_upgrade_rejects_downgrade() {
        let apps = temp_apps_root("upgrade");
        let pkg_dir = apps.parent().unwrap();
        // 1.0.0 安装成功
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        install_zip_to(&apps, &pkg).unwrap();
        // 0.9.0 降级拒绝
        let lower = write_plugin_package(pkg_dir, "0.9.0");
        let err = install_zip_to(&apps, &lower).unwrap_err();
        assert!(err.contains("请先卸载或提供更高版本"), "降级应拒绝: {err}");
        // 1.0.0 平级拒绝
        let same = write_plugin_package(pkg_dir, "1.0.0");
        let err2 = install_zip_to(&apps, &same).unwrap_err();
        assert!(
            err2.contains("请先卸载或提供更高版本"),
            "平级应拒绝: {err2}"
        );
        // 1.1.0 升级成功
        let higher = write_plugin_package(pkg_dir, "1.1.0");
        install_zip_to(&apps, &higher).unwrap();
        let reg = load_registry_from(&apps);
        assert_eq!(reg.get("com.example.demo").unwrap().version, "1.1.0");
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    // ── 打包导出：pack → install_zip_to 回环 ──

    #[test]
    fn test_pack_excludes_kv_json() {
        let apps = temp_apps_root("packkv");
        let pkg_dir = apps.parent().unwrap();
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        install_zip_to(&apps, &pkg).unwrap();
        // 模拟运行时产生的 KV 数据（本机用户数据，不得进导出包）
        std::fs::write(apps.join("com.example.demo").join("kv.json"), b"{}").unwrap();

        let dest = pkg_dir.join("packed-kv.nuph");
        pack_plugin_dir(&apps.join("com.example.demo"), &dest).unwrap();

        let file = std::fs::File::open(&dest).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n == "kv.json"),
            "kv.json 不应入包: {names:?}"
        );
        assert!(names.iter().any(|n| n == "manifest.json"));
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_pack_rejects_invalid_inputs() {
        let apps = temp_apps_root("packerr");
        let pkg_dir = apps.parent().unwrap();
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        install_zip_to(&apps, &pkg).unwrap();

        // 未安装插件 → 拒绝
        let err =
            pack_plugin_dir(&apps.join("com.example.none"), &pkg_dir.join("x.nuph")).unwrap_err();
        assert!(err.contains("读取插件目录失败"), "未安装目录应拒绝: {err}");
        // 空目录 → 拒绝
        let empty_dir = apps.join("com.example.empty");
        std::fs::create_dir_all(&empty_dir).unwrap();
        let err = pack_plugin_dir(&empty_dir, &pkg_dir.join("x.nuph")).unwrap_err();
        assert!(err.contains("为空"), "空目录应拒绝: {err}");
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_pack_roundtrip_install() {
        let apps = temp_apps_root("packrt");
        let pkg_dir = apps.parent().unwrap();
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        install_zip_to(&apps, &pkg).unwrap();

        // 打包已安装目录 → .nuph（zip 内相对路径、正斜杠）
        let dest = pkg_dir.join("packed.nuph");
        pack_plugin_dir(&apps.join("com.example.demo"), &dest).unwrap();

        // 回环：pack 产物必须能被现有安装器装回（新 apps root）
        let apps2 = temp_apps_root("packrt2");
        let manifest = install_zip_to(&apps2, &dest).unwrap();
        assert_eq!(manifest.id, "com.example.demo");
        assert_eq!(manifest.version, "1.0.0");
        assert!(apps2.join("com.example.demo").join("index.html").exists());
        assert!(apps2
            .join("com.example.demo")
            .join("assets/app.js")
            .exists());
        // 子目录文件也打包（正斜杠路径）
        assert_eq!(manifest.entry, "index.html");
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
        let _ = std::fs::remove_dir_all(apps2.parent().unwrap());
    }

    #[test]
    fn test_pack_roundtrip_preserves_category() {
        let apps = temp_apps_root("packcat");
        let pkg_dir = apps.parent().unwrap();
        // 带 category 的包安装 → 打包 → 回环安装后 category 仍可读
        let manifest = serde_json::json!({
            "id": "com.example.demo",
            "name": "Demo Plugin",
            "version": "1.0.0",
            "entry": "index.html",
            "icon": "icon.svg",
            "category": "ai",
            "permissions": ["kv"],
            "sidecar": null
        });
        let pkg = pkg_dir.join("demo-cat.nuph");
        write_zip(
            &pkg,
            &[
                ("manifest.json", manifest.to_string().as_bytes()),
                ("icon.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
                ("index.html", b"<html><body>demo</body></html>"),
            ],
        );
        let installed = install_zip_to(&apps, &pkg).unwrap();
        assert_eq!(installed.category.as_deref(), Some("ai"));

        let dest = pkg_dir.join("packed-cat.nuph");
        pack_plugin_dir(&apps.join("com.example.demo"), &dest).unwrap();

        let apps2 = temp_apps_root("packcat2");
        let m2 = install_zip_to(&apps2, &dest).unwrap();
        assert_eq!(
            m2.category.as_deref(),
            Some("ai"),
            "category 应在回环中保留"
        );
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
        let _ = std::fs::remove_dir_all(apps2.parent().unwrap());
    }

    // ── KV ──

    #[test]
    fn test_kv_basic_flow() {
        let apps = temp_apps_root("kv");
        let pkg_dir = apps.parent().unwrap();
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        install_zip_to(&apps, &pkg).unwrap();
        let id = "com.example.demo";

        assert_eq!(kv_keys_at(&apps, id).unwrap(), Vec::<String>::new());
        kv_set_at(&apps, id, "a", serde_json::json!({"n": 1})).unwrap();
        kv_set_at(&apps, id, "b", serde_json::json!("text")).unwrap();
        assert_eq!(
            kv_keys_at(&apps, id).unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            kv_get_at(&apps, id, "a").unwrap(),
            Some(serde_json::json!({"n": 1}))
        );
        kv_delete_at(&apps, id, "a").unwrap();
        assert_eq!(kv_get_at(&apps, id, "a").unwrap(), None);
        assert_eq!(kv_keys_at(&apps, id).unwrap(), vec!["b".to_string()]);

        // 单值超限拒绝
        let big = "x".repeat(MAX_KV_VALUE_BYTES + 1);
        let err = kv_set_at(&apps, id, "big", serde_json::json!(big)).unwrap_err();
        assert!(err.contains("单值超过上限"), "超限值应拒绝: {err}");
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_kv_rejects_uninstalled_or_disabled() {
        let apps = temp_apps_root("kvguard");
        let pkg_dir = apps.parent().unwrap();
        // 未安装
        let err = kv_set_at(&apps, "com.example.none", "k", serde_json::json!(1)).unwrap_err();
        assert!(err.contains("未安装"), "未安装应拒绝: {err}");

        // 已安装但禁用
        let pkg = write_plugin_package(pkg_dir, "1.0.0");
        install_zip_to(&apps, &pkg).unwrap();
        let mut reg = load_registry_from(&apps);
        reg.get_mut("com.example.demo").unwrap().enabled = false;
        save_registry_to(&apps, &reg).unwrap();
        let err = kv_set_at(&apps, "com.example.demo", "k", serde_json::json!(1)).unwrap_err();
        assert!(err.contains("禁用"), "禁用应拒绝: {err}");
        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    // ── 主题快照 ──

    #[test]
    fn test_theme_snapshot_validation() {
        use tauri::Manager;
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        handle.manage(AppState::default());
        let state = handle.state::<AppState>();

        // 初值
        let snap = state.theme_snapshot.lock().unwrap();
        assert_eq!(snap.base, "dark");
        assert!(snap.overrides.is_empty());
        drop(snap);

        // 合法写入
        theme_snapshot_save_inner(
            state.inner(),
            "light",
            &HashMap::from([("--accent".to_string(), "#ff0000".to_string())]),
        )
        .unwrap();
        let snap = state.theme_snapshot.lock().unwrap();
        assert_eq!(snap.base, "light");
        assert_eq!(snap.overrides.get("--accent").unwrap(), "#ff0000");
        drop(snap);

        // key 不以 -- 开头
        let err = theme_snapshot_save_inner(
            state.inner(),
            "dark",
            &HashMap::from([("accent".to_string(), "#000".to_string())]),
        )
        .unwrap_err();
        assert!(err.contains("必须以 -- 开头"), "坏 key 应拒绝: {err}");

        // 单值超长
        let err = theme_snapshot_save_inner(
            state.inner(),
            "dark",
            &HashMap::from([("--x".to_string(), "v".repeat(MAX_THEME_VALUE_CHARS + 1))]),
        )
        .unwrap_err();
        assert!(err.contains("超过"), "超长值应拒绝: {err}");

        // 条目超限
        let mut many = HashMap::new();
        for i in 0..(MAX_THEME_OVERRIDES + 1) {
            many.insert(format!("--v{i}"), "1".to_string());
        }
        let err = theme_snapshot_save_inner(state.inner(), "dark", &many).unwrap_err();
        assert!(err.contains("超过上限"), "条目超限应拒绝: {err}");

        // 空 base 拒绝
        let err = theme_snapshot_save_inner(state.inner(), "", &HashMap::new()).unwrap_err();
        assert!(err.contains("不能为空"), "空 base 应拒绝: {err}");
    }

    // ── agent.chat：校验分支 ──

    /// 安装一个声明指定 permissions 的最小 .nuph 包（id 可指定，避免同版本重复安装冲突）
    fn install_with_permissions(apps: &Path, pkg_dir: &Path, id: &str, permissions: &[&str]) {
        let manifest = serde_json::json!({
            "id": id,
            "name": "Demo Plugin",
            "version": "1.0.0",
            "entry": "index.html",
            "icon": "icon.svg",
            "permissions": permissions,
            "sidecar": null
        });
        let pkg = pkg_dir.join(format!("{id}.nuph"));
        write_zip(
            &pkg,
            &[
                ("manifest.json", manifest.to_string().as_bytes()),
                ("icon.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
                ("index.html", b"<html><body>demo</body></html>"),
            ],
        );
        install_zip_to(apps, &pkg).unwrap();
    }

    #[test]
    fn test_agent_chat_validation_branches() {
        let apps = temp_apps_root("agentchat-validation");
        let pkg_dir = apps.parent().unwrap();
        let id = "com.example.demo";
        let noperm = "com.example.noperm";
        install_with_permissions(&apps, pkg_dir, id, &["agent.chat", "kv"]);
        install_with_permissions(&apps, pkg_dir, noperm, &["kv"]);
        let ok_msg = "你好";

        // 合法请求通过校验
        validate_agent_chat_request(&apps, id, ok_msg).unwrap();

        // 未注册 id
        let err = validate_agent_chat_request(&apps, "com.example.nope", ok_msg).unwrap_err();
        assert!(err.contains("未安装"), "未注册应拒绝: {err}");

        // disabled
        let mut reg = load_registry_from(&apps);
        reg.get_mut(id).unwrap().enabled = false;
        save_registry_to(&apps, &reg).unwrap();
        let err = validate_agent_chat_request(&apps, id, ok_msg).unwrap_err();
        assert!(err.contains("已禁用"), "disabled 应拒绝: {err}");
        // 恢复 enabled，避免影响后续断言
        let mut reg = load_registry_from(&apps);
        reg.get_mut(id).unwrap().enabled = true;
        save_registry_to(&apps, &reg).unwrap();

        // 未声明 agent.chat 权限
        let err = validate_agent_chat_request(&apps, noperm, ok_msg).unwrap_err();
        assert!(
            err.contains("未声明 agent.chat 权限"),
            "未声明权限应拒绝: {err}"
        );

        // 空消息（含纯空白）
        for bad in ["", "   ", "\n\t"] {
            let err = validate_agent_chat_request(&apps, id, bad).unwrap_err();
            assert!(err.contains("不能为空"), "空消息应拒绝: {err}");
        }

        // 超长消息（>32KB）
        let long = "x".repeat(MAX_PLUGIN_CHAT_BYTES + 1);
        let err = validate_agent_chat_request(&apps, id, &long).unwrap_err();
        assert!(err.contains("字节上限"), "超长消息应拒绝: {err}");
        // 恰好 32KB 通过
        let exact = "x".repeat(MAX_PLUGIN_CHAT_BYTES);
        validate_agent_chat_request(&apps, id, &exact).unwrap();

        let _ = std::fs::remove_dir_all(apps.parent().unwrap());
    }

    #[test]
    fn test_agent_chat_inflight_guard() {
        let state = AppState::default();
        let id = "com.example.demo";

        // 首次 acquire 成功，同插件重入拒绝
        let g1 = PluginChatGuard::acquire(&state, id).unwrap();
        let err = match PluginChatGuard::acquire(&state, id) {
            Ok(_) => panic!("同插件重入应被拒绝"),
            Err(e) => e,
        };
        assert!(err.contains("进行中"), "重入应拒绝: {err}");

        // 不同插件互不影响
        let g2 = PluginChatGuard::acquire(&state, "com.example.other").unwrap();
        drop(g1);
        drop(g2);

        // 释放后可重入
        let g3 = PluginChatGuard::acquire(&state, id).unwrap();
        assert!(state.plugin_chat_inflight.lock().unwrap().contains(id));
        drop(g3);
        assert!(!state.plugin_chat_inflight.lock().unwrap().contains(id));
    }

    // ── agent.chat 独立运行时：隔离语义 + 并发闸 + 超时 ──

    /// 最小 mock ApiClient：固定返回文本事件序列（无工具调用 → 单轮结束）；
    /// delay 用于超时路径测试。
    struct PluginMockApiClient {
        responses: Vec<nuphus::api::AssistantEvent>,
        delay: Option<std::time::Duration>,
    }

    impl PluginMockApiClient {
        fn text(text: &str) -> Self {
            Self {
                responses: vec![
                    nuphus::api::AssistantEvent::TextDelta(text.to_string()),
                    nuphus::api::AssistantEvent::MessageStop,
                ],
                delay: None,
            }
        }
        fn delayed(text: &str, delay: std::time::Duration) -> Self {
            Self {
                responses: vec![
                    nuphus::api::AssistantEvent::TextDelta(text.to_string()),
                    nuphus::api::AssistantEvent::MessageStop,
                ],
                delay: Some(delay),
            }
        }
    }

    #[async_trait::async_trait]
    impl nuphus::api::ApiClient for PluginMockApiClient {
        async fn stream(
            &self,
            _request: nuphus::api::MessageRequest,
        ) -> nuphus::Result<Vec<nuphus::api::AssistantEvent>> {
            if let Some(d) = self.delay {
                tokio::time::sleep(d).await;
            }
            Ok(self.responses.clone())
        }
        fn model_name(&self) -> &str {
            "mock-model"
        }
        fn provider_kind(&self) -> nuphus::api::ProviderKind {
            nuphus::api::ProviderKind::MiniMax
        }
    }

    fn mock_llm(text: &str) -> Arc<dyn nuphus::api::ApiClient> {
        Arc::new(PluginMockApiClient::text(text))
    }

    fn mock_perms_ref() -> Arc<std::sync::Mutex<nuphus::permissions::ToolPermissions>> {
        Arc::new(std::sync::Mutex::new(
            nuphus::permissions::ToolPermissions::default(),
        ))
    }

    #[test]
    fn test_plugin_chat_semaphore_capacity_two() {
        // 全局闸容量断言（不 acquire，避免与其他测试互相占用）
        assert_eq!(
            plugin_chat_semaphore().available_permits(),
            PLUGIN_CHAT_MAX_CONCURRENT
        );
    }

    #[test]
    fn test_plugin_chat_concurrency_gate_queues_third() {
        // 用独立 Semaphore 验证排队语义（避免并行测试间互相占用全局闸）
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sem = Arc::new(tokio::sync::Semaphore::new(PLUGIN_CHAT_MAX_CONCURRENT));
            let _p1 = sem.acquire().await.unwrap();
            let _p2 = sem.acquire().await.unwrap();
            let sem2 = sem.clone();
            let mut waiter = tokio::spawn(async move {
                let _p3 = sem2.acquire().await.unwrap();
            });
            // 两个空位被占：第三路 100ms 内不应通过（排队等待而非拒绝/立即通过）
            let early =
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut waiter).await;
            assert!(early.is_err(), "第三路在有空位前不应通过");
            drop(_p2);
            // 释放一个空位后，第三路应在 2s 内通过
            let done = tokio::time::timeout(std::time::Duration::from_secs(2), waiter).await;
            assert!(done.is_ok(), "释放空位后第三路应排队通过");
            drop(_p1);
        });
    }

    #[test]
    fn test_plugin_chat_isolated_returns_text_and_touches_no_shared_state() {
        let state = AppState::default();
        // 模拟主会话正在执行：busy=true 不被独立运行时清除
        state.busy.store(true, Ordering::SeqCst);
        let session_before = {
            let g = state.session.lock().unwrap();
            (
                g.last_message.clone(),
                g.last_send_id.clone(),
                g.session_backup.clone(),
            )
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let reply = rt.block_on(async {
            run_plugin_chat_isolated(
                "com.example.demo",
                nuphus::ToolRegistry::leader(),
                mock_llm("插件最终回复"),
                "mock-model".to_string(),
                "mock".to_string(),
                nuphus::permissions::ToolPermissions::default(),
                mock_perms_ref(),
                "你好，插件消息",
                &None,
                "plugin:com.example.demo",
                std::time::Duration::from_secs(5),
            )
            .await
        });
        assert_eq!(reply.unwrap(), "插件最终回复");

        // 共享态零接触断言：独立运行时不得修改 busy / session / inflight
        assert!(
            state.busy.load(Ordering::SeqCst),
            "独立运行时不得清除共享 busy"
        );
        {
            let g = state.session.lock().unwrap();
            assert_eq!(
                g.last_message, session_before.0,
                "session.last_message 未变"
            );
            assert_eq!(
                g.last_send_id, session_before.1,
                "session.last_send_id 未变"
            );
            assert_eq!(g.session_backup, session_before.2, "session_backup 未变");
        }
        assert!(
            !state
                .plugin_chat_inflight
                .lock()
                .unwrap()
                .contains("com.example.demo"),
            "独立路径不登记/泄漏在途标记"
        );
    }

    #[test]
    fn test_plugin_chat_isolated_timeout_hard_kill() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            run_plugin_chat_isolated(
                "com.example.demo",
                nuphus::ToolRegistry::leader(),
                Arc::new(PluginMockApiClient::delayed(
                    "慢回复",
                    std::time::Duration::from_secs(5),
                )),
                "mock-model".to_string(),
                "mock".to_string(),
                nuphus::permissions::ToolPermissions::default(),
                mock_perms_ref(),
                "你好",
                &None,
                "plugin:com.example.demo",
                std::time::Duration::from_millis(100),
            )
            .await
            .unwrap_err()
        });
        assert!(err.contains("超时"), "超时应报强制终止: {err}");
    }

    #[test]
    fn test_plugin_chat_history_validation() {
        let mk = |role: &str, content: &str| HistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            images: vec![],
            audio: vec![],
            timestamp: None,
            trace_items: vec![],
        };
        // 合法
        let ok = Some(vec![
            mk("user", "你好"),
            mk("assistant", "你好！有什么可以帮你？"),
            mk("system", "你是助手"),
        ]);
        assert!(validate_agent_chat_history(&ok).is_ok());
        // None / 空列表合法
        assert!(validate_agent_chat_history(&None).is_ok());
        assert!(validate_agent_chat_history(&Some(vec![])).is_ok());
        // 超过 50 条
        let many: Vec<HistoryMessage> = (0..MAX_PLUGIN_CHAT_HISTORY + 1)
            .map(|i| mk("user", &format!("m{i}")))
            .collect();
        let err = validate_agent_chat_history(&Some(many)).unwrap_err();
        assert!(err.contains("50"), "超条数应拒绝: {err}");
        // 总字符超 64KB
        let big = vec![
            mk("user", &"x".repeat(MAX_PLUGIN_CHAT_HISTORY_CHARS / 2)),
            mk(
                "assistant",
                &"y".repeat(MAX_PLUGIN_CHAT_HISTORY_CHARS / 2 + 1),
            ),
        ];
        let err = validate_agent_chat_history(&Some(big)).unwrap_err();
        assert!(err.contains("字符"), "超字符应拒绝: {err}");
        // 非法 role
        let bad_role = Some(vec![mk("robot", "hi")]);
        let err = validate_agent_chat_history(&bad_role).unwrap_err();
        assert!(err.contains("role"), "非法 role 应拒绝: {err}");
        // 空 content
        let empty = Some(vec![mk("user", "   ")]);
        let err = validate_agent_chat_history(&empty).unwrap_err();
        assert!(err.contains("不能为空"), "空 content 应拒绝: {err}");
    }

    /// 构造已注册 + enabled + 给定 permissions 的插件目录（manifest + registry）
    fn register_plugin(apps_root: &Path, permissions: &[&str], enabled: bool) {
        let dir = apps_root.join("com.example.demo");
        std::fs::create_dir_all(&dir).unwrap();
        let mut manifest = valid_manifest();
        manifest.permissions = permissions.iter().map(|s| s.to_string()).collect();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let mut reg = HashMap::new();
        reg.insert(
            "com.example.demo".to_string(),
            PluginRegistryEntry {
                version: "1.0.0".to_string(),
                enabled,
                installed_at: "2026-08-17T00:00:00Z".to_string(),
                permissions: manifest.permissions,
            },
        );
        save_registry_to(apps_root, &reg).unwrap();
    }

    #[test]
    fn test_workflow_run_request_validation() {
        let apps_root = temp_apps_root("wf-run");

        // 未安装 → 拒绝
        let err =
            validate_workflow_run_request(&apps_root, "com.example.demo", "wf-1").unwrap_err();
        assert!(err.contains("未安装"), "未安装应拒绝: {err}");

        // 已安装但未声明 workflow.run → 拒绝（纵深防御）
        register_plugin(&apps_root, &["kv"], true);
        let err =
            validate_workflow_run_request(&apps_root, "com.example.demo", "wf-1").unwrap_err();
        assert!(err.contains("workflow.run"), "未声明权限应拒绝: {err}");

        // 已声明 workflow.run + 非空 id → 通过
        register_plugin(&apps_root, &["workflow.run"], true);
        assert!(validate_workflow_run_request(&apps_root, "com.example.demo", "wf-1").is_ok());

        // 空 workflow_id → 拒绝
        let err = validate_workflow_run_request(&apps_root, "com.example.demo", "  ").unwrap_err();
        assert!(err.contains("workflow_id"), "空 id 应拒绝: {err}");

        // 已禁用 → 拒绝
        register_plugin(&apps_root, &["workflow.run"], false);
        let err =
            validate_workflow_run_request(&apps_root, "com.example.demo", "wf-1").unwrap_err();
        assert!(err.contains("禁用"), "禁用应拒绝: {err}");
    }

    #[test]
    fn test_export_sample_whitelist_and_write() {
        let dir = std::env::temp_dir().join(format!(
            "nuphus-test-export-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // 非法 sample_id → 白名单拒绝
        let err = plugin_export_sample("evil".to_string(), dir.to_string_lossy().into_owned())
            .unwrap_err();
        assert!(err.contains("未知示例"), "非法 id 应拒绝: {err}");

        // 目标目录不存在 → 拒绝
        let err2 = plugin_export_sample(
            "hello-plugin".to_string(),
            dir.join("missing").to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert!(err2.contains("不存在"), "目标目录不存在应拒绝: {err2}");

        // hello-plugin 导出成功：三文件齐全 + manifest 内容与内嵌模板一致（sample:true）
        let out = plugin_export_sample(
            "hello-plugin".to_string(),
            dir.to_string_lossy().into_owned(),
        )
        .unwrap();
        let out_path = Path::new(&out);
        assert!(out_path.join("manifest.json").is_file());
        assert!(out_path.join("index.html").is_file());
        assert!(out_path.join("README.md").is_file());
        let manifest_text = std::fs::read_to_string(out_path.join("manifest.json")).unwrap();
        assert!(
            manifest_text.contains("\"sample\": true"),
            "内嵌 manifest 应含 sample:true（任务2已更新）: {manifest_text}"
        );

        // 已存在非空目录 → 拒绝覆盖（防止误伤用户已有工作）
        std::fs::write(out_path.join("extra.txt"), "x").unwrap();
        let err3 = plugin_export_sample(
            "hello-plugin".to_string(),
            dir.to_string_lossy().into_owned(),
        )
        .unwrap_err();
        assert!(err3.contains("非空"), "非空目录应拒绝覆盖: {err3}");

        // agent-chat 同样可导出
        let out2 =
            plugin_export_sample("agent-chat".to_string(), dir.to_string_lossy().into_owned())
                .unwrap();
        assert!(Path::new(&out2).join("manifest.json").is_file());
        assert!(Path::new(&out2).join("index.html").is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(feature = "market")]
    #[test]
    fn sha256_file_known_digest() {
        let tmp = std::env::temp_dir().join(format!(
            "nuphus-sha256-test-{}.bin",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&tmp, b"hello world").unwrap();
        let h = sha256_file(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(
            h,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[cfg(feature = "market")]
    #[test]
    fn sha256_file_missing_path_errors() {
        let err = sha256_file(Path::new("Z:/definitely/not/exist.nuph")).unwrap_err();
        assert!(!err.is_empty(), "缺失文件应返回错误而非 panic");
    }

    #[cfg(feature = "market")]
    #[test]
    fn market_scheme_check_rejects_http() {
        // 与命令入口相同的校验逻辑（命令本身依赖网络，仅验证 URL 闸）
        fn scheme_ok(url: &str) -> bool {
            url.starts_with("https://")
        }
        assert!(scheme_ok(
            "https://mrpulor-gh.github.io/nuphus-market/plugins/x.nuph"
        ));
        assert!(
            !scheme_ok("http://192.168.1.5:8080/x.nuph"),
            "http 明文不再放行"
        );
        assert!(!scheme_ok("file:///tmp/x.nuph"));
    }
}
