//! custom_agents.rs — Custom Agent 配置系统（Pro 用户的专属 Agent）
//!
//! CustomAgentConfig 定义一个用户自定义 Agent：name（chip 显示名）、l2_prompt（完全
//! 替换 L2 的自由文本）、tools（白名单，空 = 全开）、greeting（激活开场白）、knowledge
//! （知识库绑定）。L0 宪法与 L1 系统协议永远锁定，Custom 只接管 L2 人格层。
//!
//! 存储结构（参照 ChatAgentStore 文件 CRUD 模式）：
//! - plugin/custom-agents/{id}.json  每个卡片独立文件
//! - plugin/custom-agents/active.json  记录当前激活的卡片 id

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::RwLock;

// ── 当前 Custom 会话归属（记忆隔离锚点）──
// set_mode 进入 Custom 时写入激活卡片 id，离开时清空。
// insert_entry 写入记忆时读取此状态填充 custom_agent_id，
// 记忆检索（memory_search/memory_recent）读取此状态做双向隔离过滤。
static CURRENT_CUSTOM_AGENT_ID: OnceLock<RwLock<Option<String>>> = OnceLock::new();

fn current_id_lock() -> &'static RwLock<Option<String>> {
    CURRENT_CUSTOM_AGENT_ID.get_or_init(|| RwLock::new(None))
}

/// 当前 Custom 会话的卡片 id（非 Custom 会话返回 None）
pub fn current_custom_agent_id() -> Option<String> {
    current_id_lock().read().ok().and_then(|g| g.clone())
}

/// 设置/清除当前 Custom 会话归属（mode 切换时调用）
pub fn set_current_custom_agent_id(id: Option<String>) {
    if let Ok(mut g) = current_id_lock().write() {
        *g = id;
    }
}

// ── CustomAgentConfig ──

/// Custom Agent 配置
///
/// name      → chip 显示名 + Agent 自称（情感连接起点）
/// l2_prompt → 完全替换 L2 的自由文本（人格/职责/准则/禁止项全由用户定）
/// tools     → 工具白名单（空 = 全开不过滤；非空 = 只启用列出的）
/// greeting  → 激活时主动说的第一句话（陪伴感）
/// knowledge → 知识库绑定路径列表（可选）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomAgentConfig {
    pub id: String,
    pub name: String,
    /// L2 完全替换文本（用户自由编写）
    #[serde(default)]
    pub l2_prompt: String,
    /// 工具白名单（空 = 全开）
    #[serde(default)]
    pub tools: Vec<String>,
    /// 激活开场白
    #[serde(default)]
    pub greeting: String,
    /// 知识库绑定路径列表
    #[serde(default)]
    pub knowledge: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

impl CustomAgentConfig {
    /// 工具白名单是否生效（非空才过滤）
    pub fn has_tool_whitelist(&self) -> bool {
        !self.tools.is_empty()
    }

    /// 判断工具是否被允许（白名单为空 = 全开；否则必须在列）
    pub fn allows_tool(&self, tool: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == tool)
    }
}

// ── CustomAgentStore ──

static STORE_DIR: OnceLock<PathBuf> = OnceLock::new();
static AGENTS_CACHE: OnceLock<Mutex<Option<Vec<CustomAgentConfig>>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Vec<CustomAgentConfig>>> {
    AGENTS_CACHE.get_or_init(|| Mutex::new(None))
}

fn invalidate_cache() {
    if let Ok(mut guard) = cache().lock() {
        *guard = None;
    }
}

fn store_dir() -> &'static PathBuf {
    STORE_DIR.get_or_init(|| {
        let base = crate::utils::workspace_root()
            .join("plugin")
            .join("custom-agents");
        let _ = std::fs::create_dir_all(&base);
        base
    })
}

fn agents_dir() -> PathBuf {
    store_dir().clone()
}

fn active_path() -> PathBuf {
    store_dir().join("active.json")
}

fn agent_path(id: &str) -> PathBuf {
    agents_dir().join(format!("{}.json", id))
}

// ── 内部加载 ──

fn load_all_raw() -> Vec<CustomAgentConfig> {
    let dir = agents_dir();
    let mut configs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && path.file_name().is_some_and(|n| n != "active.json")
            {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(cfg) = serde_json::from_str::<CustomAgentConfig>(&text) {
                        configs.push(cfg);
                    }
                }
            }
        }
    }
    configs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    configs
}

fn load_all_cached() -> Vec<CustomAgentConfig> {
    let mut guard = cache().lock().unwrap();
    if let Some(ref cached) = *guard {
        return cached.clone();
    }
    let loaded = load_all_raw();
    *guard = Some(loaded.clone());
    loaded
}

fn save_one(config: &CustomAgentConfig) {
    let path = agent_path(&config.id);
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(&path, json);
    }
}

#[derive(Serialize, Deserialize)]
struct ActiveRecord {
    active: Option<String>,
}

fn read_active() -> Option<String> {
    let path = active_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let record: ActiveRecord = serde_json::from_str(&text).ok()?;
    record.active
}

fn write_active(id: &str) {
    let path = active_path();
    let record = ActiveRecord {
        active: Some(id.to_string()),
    };
    if let Ok(json) = serde_json::to_string_pretty(&record) {
        let _ = std::fs::write(&path, json);
    }
}

fn clear_active_file() {
    let path = active_path();
    let record = ActiveRecord { active: None };
    if let Ok(json) = serde_json::to_string_pretty(&record) {
        let _ = std::fs::write(&path, json);
    }
}

// ── 公共 API ──

pub struct CustomAgentStore;

impl CustomAgentStore {
    /// 列出所有 custom agent 卡片
    pub fn list() -> Vec<CustomAgentConfig> {
        load_all_cached()
    }

    /// 按 id 查找
    pub fn load_by_id(id: &str) -> Option<CustomAgentConfig> {
        load_all_cached().into_iter().find(|c| c.id == id)
    }

    /// 保存（新增或更新）
    pub fn save(config: &CustomAgentConfig) -> Result<CustomAgentConfig, String> {
        let mut new_config = config.clone();
        let now = chrono::Local::now().to_rfc3339();
        if new_config.id.is_empty() {
            new_config.id = uuid::Uuid::new_v4().to_string();
            new_config.created_at = now.clone();
        }
        new_config.updated_at = now;
        save_one(&new_config);
        invalidate_cache();
        Ok(new_config)
    }

    /// 按 id 删除
    pub fn delete_by_id(id: &str) -> Result<(), String> {
        let path = agent_path(id);
        if !path.exists() {
            return Err(format!("Custom agent '{}' not found", id));
        }
        // 若删除的是当前激活卡片，清空激活记录
        if read_active().as_deref() == Some(id) {
            clear_active_file();
        }
        let _ = std::fs::remove_file(&path);
        invalidate_cache();
        Ok(())
    }

    /// 设置激活卡片，返回该配置
    pub fn set_active(id: &str) -> Result<CustomAgentConfig, String> {
        let config =
            Self::load_by_id(id).ok_or_else(|| format!("Custom agent '{}' not found", id))?;
        write_active(id);
        Ok(config)
    }

    /// 清除激活（切回 Leader/Workflow 时无激活 Custom）
    pub fn clear_active() {
        clear_active_file();
    }

    /// 获取当前激活的 custom agent 配置
    pub fn get_active() -> Option<CustomAgentConfig> {
        let active_id = read_active()?;
        Self::load_by_id(&active_id)
    }

    /// 卡片总数
    pub fn count() -> usize {
        load_all_cached().len()
    }
}
