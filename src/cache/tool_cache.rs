//! ToolCache — 持久化只读工具结果缓存
//!
//! 单例模式（`global_cache()` 返回全局实例），跨 ExecuteAgent 生命周期共享。
//! 内存 LRU + 文件持久化（`.nuphus/cache/tool_cache.json`）。

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::ToolResult;

// ── 常量 ──

/// 最大缓存条目数（LRU 淘汰）
const MAX_ENTRIES: usize = 500;

/// 文件工具校验通过后，该条目的"生存时间"（秒）。
/// 超过此时间即使 mtime 没变也重新读取（兜底，防止文件系统时间戳精度问题）。
const FILE_CACHE_TTL_SECS: u64 = 300;

/// 网络工具缓存的 TTL（秒）
const WEB_CACHE_TTL_SECS: u64 = 60;

// ── 全局单例 ──

/// 获取全局 ToolCache 实例（懒加载）
pub fn global_cache() -> &'static Mutex<ToolCache> {
    static CACHE: OnceLock<Mutex<ToolCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let path = default_store_path();
        let mut cache = ToolCache::new(path);
        let _ = cache.load();
        Mutex::new(cache)
    })
}

fn default_store_path() -> PathBuf {
    // <project_root>/.nuphus/cache/tool_cache.json
    let mut path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    path.push(".nuphus");
    path.push("cache");
    let _ = std::fs::create_dir_all(&path);
    path.push("tool_cache.json");
    path
}

// ── 缓存 key ──

/// 计算缓存 key（tool + params_json 的哈希）
fn cache_key(tool: &str, params_json: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    tool.hash(&mut hasher);
    params_json.hash(&mut hasher);
    hasher.finish()
}

/// 从参数 JSON 中提取 path 字段
fn extract_path(params: &serde_json::Value) -> Option<String> {
    params
        .get("path")
        .and_then(|v| v.as_str())
        .map(String::from)
}

// ── 持久化条目 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntry {
    key: u64,
    tool: String,
    params: String,
    result: ToolResult,
    /// 文件路径（仅文件工具）
    file_path: Option<String>,
    /// 文件修改时间（UNIX 纪元秒）
    file_mtime: Option<u64>,
    /// 文件大小
    file_size: Option<u64>,
    /// 创建时间（UNIX 纪元秒）
    created_at: u64,
    /// TTL 秒数（None 表示不过期）
    ttl_secs: Option<u64>,
}

// ── 内存条目 ──

struct Entry {
    key: u64,
    result: ToolResult,
    tool: String,
    params: String,
    file_path: Option<String>,
    file_mtime: Option<u64>,
    file_size: Option<u64>,
    created_at: u64,
    ttl_secs: Option<u64>,
}

// ── 工具分类 ──

fn is_file_tool(tool: &str) -> bool {
    matches!(
        tool,
        "Read" | "Write" | "Edit" | "Delete" | "Copy" | "Rename" | "Append" | "Diff"
    ) || tool.starts_with("file_")
        || tool.starts_with("search_")
}

fn is_web_tool(tool: &str) -> bool {
    tool.starts_with("web_")
}

// ── ToolCache ──

/// 持久化只读工具结果缓存
pub struct ToolCache {
    /// key → 条目
    entries: HashMap<u64, Entry>,
    /// LRU 顺序（从头到尾：最近→最旧）
    order: Vec<u64>,
    /// 持久化文件路径
    store_path: PathBuf,
    /// 最大条目数
    max: usize,
}

impl ToolCache {
    /// 创建新缓存（不自动加载持久化数据）
    pub fn new(store_path: PathBuf) -> Self {
        Self {
            entries: HashMap::new(),
            order: Vec::new(),
            store_path,
            max: MAX_ENTRIES,
        }
    }

    // ── 公共 API ──

    /// 获取缓存结果
    ///
    /// 自动执行校验：
    /// - 文件工具：stat 文件验证 mtime + size
    /// - 网络工具：检查 TTL
    /// - 校验失败 → 删除条目 → 返回 None
    pub fn get(&mut self, tool: &str, params: &serde_json::Value) -> Option<ToolResult> {
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let key = cache_key(tool, &params_json);

        // 先拿一个只读引用来校验
        let should_remove = match self.entries.get(&key) {
            Some(entry) => !self.validate(entry),
            None => return None,
        };

        if should_remove {
            self.remove(key);
            return None;
        }

        // LRU：移到前面
        if let Some(pos) = self.order.iter().position(|&k| k == key) {
            self.order.remove(pos);
            self.order.insert(0, key);
        }

        self.entries.get(&key).map(|e| e.result.clone())
    }

    /// 设置缓存结果
    pub fn set(&mut self, tool: &str, params: &serde_json::Value, result: ToolResult) {
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let key = cache_key(tool, &params_json);

        // 提取校验信息
        let (file_path, file_mtime, file_size) = if is_file_tool(tool) {
            extract_path(params).map_or((None, None, None), |p| {
                let fp = p.clone();
                stat_file(&p)
                    .map(|(m, s)| (Some(fp), Some(m), Some(s)))
                    .unwrap_or((Some(p), None, None))
            })
        } else {
            (None, None, None)
        };

        let ttl_secs = if is_web_tool(tool) {
            Some(WEB_CACHE_TTL_SECS)
        } else if file_path.is_some() {
            Some(FILE_CACHE_TTL_SECS)
        } else {
            None
        };

        let now = now_epoch();

        // LRU 淘汰
        if !self.entries.contains_key(&key) && self.entries.len() >= self.max {
            if let Some(old_key) = self.order.pop() {
                self.entries.remove(&old_key);
            }
        }

        let entry = Entry {
            key,
            result,
            tool: tool.to_string(),
            params: params_json,
            file_path,
            file_mtime,
            file_size,
            created_at: now,
            ttl_secs,
        };

        match self.entries.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                // 更新：保持旧位置
                e.insert(entry);
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                // 新增：插到 LRU 前面
                e.insert(entry);
                self.order.insert(0, key);
            }
        }
    }

    /// 按路径失效缓存（Write / Edit 后调用）
    pub fn invalidate(&mut self, path: &str) {
        let normalized = normalize_path(path);
        let mut to_remove = Vec::new();

        for (key, entry) in &self.entries {
            if let Some(ref fp) = entry.file_path {
                if normalize_path(fp) == normalized || path_matches(fp, &normalized) {
                    to_remove.push(*key);
                }
            }
        }

        for key in to_remove {
            self.remove(key);
        }
    }

    /// 按工具名失效所有缓存（如 agent 切换模式时清空）
    pub fn invalidate_tool(&mut self, tool: &str) {
        let mut to_remove = Vec::new();
        for (key, entry) in &self.entries {
            if entry.tool == tool {
                to_remove.push(*key);
            }
        }
        for key in to_remove {
            self.remove(key);
        }
    }

    /// 清空所有缓存
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        let _ = self.persist();
    }

    /// 当前条目数
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // ── 持久化 ──

    /// 从文件加载缓存
    pub fn load(&mut self) -> Result<(), String> {
        if !self.store_path.exists() {
            return Ok(());
        }
        let data = std::fs::read_to_string(&self.store_path)
            .map_err(|e| format!("read cache file: {}", e))?;
        let persisted: Vec<PersistedEntry> =
            serde_json::from_str(&data).map_err(|e| format!("parse cache: {}", e))?;

        for p in persisted {
            let entry = Entry {
                key: p.key,
                result: p.result,
                tool: p.tool,
                params: p.params,
                file_path: p.file_path,
                file_mtime: p.file_mtime,
                file_size: p.file_size,
                created_at: p.created_at,
                ttl_secs: p.ttl_secs,
            };
            // 跳过已过期的
            if is_expired(&entry) {
                continue;
            }
            if self.entries.len() < self.max {
                self.entries.insert(p.key, entry);
                self.order.push(p.key);
            }
        }
        Ok(())
    }

    /// 持久化到文件
    pub fn persist(&self) -> Result<(), String> {
        let persisted: Vec<PersistedEntry> = self
            .entries
            .values()
            .map(|e| PersistedEntry {
                key: e.key,
                tool: e.tool.clone(),
                params: e.params.clone(),
                result: e.result.clone(),
                file_path: e.file_path.clone(),
                file_mtime: e.file_mtime,
                file_size: e.file_size,
                created_at: e.created_at,
                ttl_secs: e.ttl_secs,
            })
            .collect();

        let data = serde_json::to_string_pretty(&persisted)
            .map_err(|e| format!("serialize cache: {}", e))?;

        // 原子写入：先写临时文件，再 rename
        let tmp_path = self.store_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &data).map_err(|e| format!("write cache tmp: {}", e))?;
        std::fs::rename(&tmp_path, &self.store_path).map_err(|e| format!("rename cache: {}", e))?;
        Ok(())
    }

    // ── 内部方法 ──

    fn remove(&mut self, key: u64) {
        self.entries.remove(&key);
        self.order.retain(|&k| k != key);
    }

    fn validate(&self, entry: &Entry) -> bool {
        let now = now_epoch();

        // TTL 过期检查
        if let Some(ttl) = entry.ttl_secs {
            if now > entry.created_at + ttl {
                return false;
            }
        }

        // 文件工具：mtime + size 校验
        if let Some(ref path) = entry.file_path {
            if let Some((mtime, size)) = stat_file(path) {
                let mtime_ok = entry.file_mtime == Some(mtime);
                let size_ok = entry.file_size == Some(size);
                if !mtime_ok || !size_ok {
                    return false;
                }
            } else {
                // 文件已不存在
                return false;
            }
        }

        true
    }
}

// ── 辅助函数 ──

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn stat_file(path: &str) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    let size = meta.len();
    Some((mtime, size))
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// 检查缓存文件路径是否匹配失效路径
/// 支持精确匹配和前缀匹配（目录下所有文件）
fn path_matches(cached: &str, invalidate: &str) -> bool {
    let c = normalize_path(cached);
    let i = normalize_path(invalidate);
    // 精确匹配
    if c == i {
        return true;
    }
    // 目录前缀匹配：invalid 是目录，cached 是其子文件
    if i.ends_with('/') || i.ends_with('\\') {
        c.starts_with(&i)
    } else {
        c.starts_with(&format!("{}/", i))
    }
}

fn is_expired(entry: &Entry) -> bool {
    if let Some(ttl) = entry.ttl_secs {
        let now = now_epoch();
        return now > entry.created_at + ttl;
    }
    false
}

impl Drop for ToolCache {
    fn drop(&mut self) {
        let _ = self.persist();
    }
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_result(data: &str) -> ToolResult {
        ToolResult {
            success: true,
            output: Some(data.to_string()),
            error: None,
            exit_code: Some(0),
        }
    }

    #[test]
    fn test_cache_key_deterministic() {
        let a = cache_key("Read", r#"{"path":"/tmp/x"}"#);
        let b = cache_key("Read", r#"{"path":"/tmp/x"}"#);
        assert_eq!(a, b);
    }

    #[test]
    fn test_cache_key_differs() {
        let a = cache_key("Read", r#"{"path":"/tmp/x"}"#);
        let b = cache_key("Read", r#"{"path":"/tmp/y"}"#);
        assert_ne!(a, b);
    }

    #[test]
    fn test_basic_set_get() {
        let tmp = std::env::temp_dir().join("test_cache_basic.json");
        let _ = std::fs::remove_file(&tmp);
        let mut c = ToolCache::new(tmp.clone());

        let params = json!({"url": "https://example.com/test"});
        c.set("web_extract", &params, make_result("hello"));

        let got = c.get("web_extract", &params);
        assert!(got.is_some());
        assert_eq!(got.unwrap().output.as_deref(), Some("hello"));

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_cache_miss() {
        let tmp = std::env::temp_dir().join("test_cache_miss.json");
        let mut c = ToolCache::new(tmp);

        let params = json!({"path": "/tmp/nonexistent.txt"});
        assert!(c.get("Read", &params).is_none());
    }

    #[test]
    fn test_lru_eviction() {
        let tmp = std::env::temp_dir().join("test_cache_lru.json");
        let mut c = ToolCache::new(tmp.clone());
        c.max = 3;

        for i in 0..5 {
            let params = json!({"url": format!("https://example.com/{}.html", i)});
            c.set("web_extract", &params, make_result(&format!("data{}", i)));
        }

        // 0 和 1 应被淘汰
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/0.html"}))
            .is_none());
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/1.html"}))
            .is_none());
        // 2, 3, 4 应存在
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/2.html"}))
            .is_some());
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/3.html"}))
            .is_some());
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/4.html"}))
            .is_some());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_web_ttl_expires() {
        let tmp = std::env::temp_dir().join("test_cache_web.json");
        let mut c = ToolCache::new(tmp.clone());

        let params = json!({"url": "https://example.com"});
        c.set("web_extract", &params, make_result("content"));

        // 刚写入应命中
        assert!(c.get("web_extract", &params).is_some());

        // 手动把 created_at 改到过去
        if let Some(entry) = c.entries.get_mut(&cache_key(
            "web_extract",
            r#"{"url":"https://example.com"}"#,
        )) {
            entry.created_at = now_epoch() - 120; // 120秒前 > TTL 60
        }

        // 应过期
        assert!(c.get("web_extract", &params).is_none());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_invalidate_path() {
        let tmp = std::env::temp_dir().join("test_cache_inval.json");
        let mut c = ToolCache::new(tmp.clone());

        // 用 web 工具测 invalidation（无需真实文件）
        c.set(
            "web_extract",
            &json!({"url": "https://example.com/main"}),
            make_result("code"),
        );
        c.set(
            "web_extract",
            &json!({"url": "https://example.com/lib"}),
            make_result("lib"),
        );
        c.set(
            "web_extract",
            &json!({"url": "https://example.com/note"}),
            make_result("note"),
        );

        // invalidation 对 file_path 为 None 的条目无效
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/main"}))
            .is_some());
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/lib"}))
            .is_some());
        assert!(c
            .get("web_extract", &json!({"url": "https://example.com/note"}))
            .is_some());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_persist_and_reload() {
        let tmp = std::env::temp_dir().join("test_cache_persist.json");
        let _ = std::fs::remove_file(&tmp);

        // 写入
        {
            let mut c = ToolCache::new(tmp.clone());
            c.set(
                "web_extract",
                &json!({"url": "https://x.com/a"}),
                make_result("aaa"),
            );
            c.set(
                "web_extract",
                &json!({"url": "https://x.com/b"}),
                make_result("bbb"),
            );
            c.persist().unwrap();
        }

        // 重新加载
        {
            let mut c = ToolCache::new(tmp.clone());
            c.load().unwrap();
            assert_eq!(c.entries.len(), 2);

            let r1 = c.get("web_extract", &json!({"url": "https://x.com/a"}));
            assert!(r1.is_some());
            assert_eq!(r1.unwrap().output.as_deref(), Some("aaa"));

            let r2 = c.get("web_extract", &json!({"url": "https://x.com/b"}));
            assert!(r2.is_some());
            assert_eq!(r2.unwrap().output.as_deref(), Some("bbb"));
        }

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_file_cache_mtime_validation() {
        let tmp = std::env::temp_dir().join("test_cache_mtime.json");
        let _ = std::fs::remove_file(&tmp);

        // 创建临时文件
        let test_file = std::env::temp_dir().join("test_cache_readme.txt");
        let _ = std::fs::remove_file(&test_file);
        std::fs::write(&test_file, b"hello world").unwrap();

        let mut c = ToolCache::new(tmp.clone());
        let params = json!({"path": test_file.to_str().unwrap()});

        c.set("Read", &params, make_result("hello world"));

        // 文件未改 → 命中
        let got = c.get("Read", &params);
        assert!(got.is_some());
        assert_eq!(got.unwrap().output.as_deref(), Some("hello world"));

        // 修改文件 → 自动失效
        std::fs::write(&test_file, b"modified content").unwrap();
        assert!(c.get("Read", &params).is_none());

        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&test_file);
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path(r"C:\Users\test\file.rs"),
            "c:/users/test/file.rs"
        );
        assert_eq!(normalize_path("/users/test/file.rs"), "/users/test/file.rs");
    }

    #[test]
    fn test_path_matches() {
        assert!(path_matches("/proj/main.rs", "/proj/main.rs"));
        assert!(path_matches("/proj/src/main.rs", "/proj"));
        assert!(path_matches("/proj/src/main.rs", "/proj/"));
        assert!(!path_matches("/proj2/main.rs", "/proj"));
    }
}
