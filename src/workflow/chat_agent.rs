//! chat_agent.rs — Chat Agent 配置系统
//!
//! ChatAgentConfig 定义聊天代理的 persona/style/requirements/knowledge 等配置，
//! 一个卡片就是一套完整的提示词预设，用户定义所有行为，系统不做场景分类。
//! ChatAgentStore 提供文件系统 CRUD（目录 `plugin/chat-agents/`），
//! 供 executor 的 TalkStep 使用，替代旧的 prompt/xxx.md 文件加载方式。
//!
//! 存储结构：
//! - plugin/chat-agents/{id}.json  每个 agent 配置独立文件
//! - plugin/chat-agents/active.json  记录当前激活的 agent 名称

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

// ── ChatAgentConfig ──

/// Chat Agent 配置（四段式语义模型）
///
/// persona → 身份定义（它是谁）
/// goal   → 任务目标（要达成什么）
/// constraints → 任务约束（不能做什么、边界在哪）
/// requirements → 操作规范（怎么做、工具使用规范）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAgentConfig {
    pub id: String,
    pub name: String,
    /// 身份定义（user prompt），叠加在基础框架上
    #[serde(default)]
    pub persona: String,
    /// 任务目标（可被 step 覆盖）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// 任务约束列表
    #[serde(default)]
    pub constraints: Vec<String>,
    /// 操作规范列表
    #[serde(default)]
    pub requirements: Vec<String>,
    /// 知识库引用路径列表
    #[serde(default)]
    pub knowledge: Vec<String>,
    /// ReAct 最大循环轮数，默认 15
    #[serde(default = "build_chatagent_default_max_iterations")]
    pub max_iterations: u32,
}

fn build_chatagent_default_max_iterations() -> u32 {
    build_chatagent_max_iterations()
}

/// 从全局配置读取 ChatAgent 最大轮数默认值（config → capabilities → chat_agent_max_iterations），未配置则 15
pub fn build_chatagent_max_iterations() -> u32 {
    match crate::config::load_registry() {
        Ok(registry) => registry
            .capabilities
            .chat_agent_max_iterations
            .unwrap_or(15),
        Err(_) => 15,
    }
}

impl Default for ChatAgentConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            persona: String::new(),
            goal: None,
            constraints: Vec::new(),
            requirements: Vec::new(),
            knowledge: Vec::new(),
            max_iterations: build_chatagent_max_iterations(),
        }
    }
}

impl ChatAgentConfig {
    pub fn new(name: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            persona: String::new(),
            goal: None,
            constraints: Vec::new(),
            requirements: Vec::new(),
            knowledge: Vec::new(),
            max_iterations: build_chatagent_max_iterations(),
        }
    }

    /// 构建四段式用户配置 prompt（身份 | 目标 | 约束 | 规范）
    pub fn build_chatagent_user_config(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        // 身份定义
        if !self.persona.is_empty() {
            parts.push(format!("## 身份定义\n{}", self.persona));
        }

        // 任务目标
        if let Some(ref goal) = self.goal {
            if !goal.is_empty() {
                parts.push(format!("## 任务目标\n{}", goal));
            }
        }

        // 约束条件
        if !self.constraints.is_empty() {
            let items = self
                .constraints
                .iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("## 约束条件\n{}", items));
        }

        // 操作规范
        if !self.requirements.is_empty() {
            let items = self
                .requirements
                .iter()
                .map(|r| format!("- {}", r))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("## 操作规范\n{}", items));
        }

        parts.join("\n\n")
    }
}

// ── Active 记录 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveRecord {
    active: Option<String>,
}

// ── ChatAgentStore ──

static STORE_DIR: OnceLock<PathBuf> = OnceLock::new();
static AGENTS_CACHE: OnceLock<Mutex<Option<Vec<ChatAgentConfig>>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Vec<ChatAgentConfig>>> {
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
            .join("chat-agents");
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

fn load_all_raw() -> Vec<ChatAgentConfig> {
    let dir = agents_dir();
    let mut configs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && path.file_name().is_some_and(|n| n != "active.json")
            {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(cfg) = serde_json::from_str::<ChatAgentConfig>(&content) {
                        configs.push(cfg);
                    }
                }
            }
        }
    }
    configs
}

fn load_all_cached() -> Vec<ChatAgentConfig> {
    if let Ok(guard) = cache().lock() {
        if let Some(ref data) = *guard {
            return data.clone();
        }
    }
    let data = load_all_raw();
    if let Ok(mut guard) = cache().lock() {
        *guard = Some(data.clone());
    }
    data
}

fn save_one(config: &ChatAgentConfig) {
    let path = agent_path(&config.id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(&path, json);
    }
}

fn delete_file(id: &str) {
    let path = agent_path(id);
    let _ = std::fs::remove_file(&path);
}

// ── Active 读写 ──

fn read_active() -> Option<String> {
    let path = active_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(record) = serde_json::from_str::<ActiveRecord>(&content) {
                return record.active;
            }
        }
    }
    None
}

fn write_active(name: &str) {
    let path = active_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let record = ActiveRecord {
        active: Some(name.to_string()),
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

pub struct ChatAgentStore;

impl ChatAgentStore {
    /// 列出所有 agent 配置
    pub fn list() -> Vec<ChatAgentConfig> {
        load_all_cached()
    }

    /// 按 name 查找 agent 配置
    pub fn load(name: &str) -> Option<ChatAgentConfig> {
        load_all_cached()
            .into_iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// 按 id 查找 agent 配置
    pub fn load_by_id(id: &str) -> Option<ChatAgentConfig> {
        load_all_cached().into_iter().find(|c| c.id == id)
    }

    /// 保存（新增或更新）agent 配置
    pub fn save(config: &ChatAgentConfig) -> Result<ChatAgentConfig, String> {
        let existing = load_all_raw();
        if let Some(old) = existing
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(&config.name))
        {
            let mut updated = config.clone();
            updated.id = old.id.clone();
            save_one(&updated);
            invalidate_cache();
            return Ok(updated);
        }

        let mut new_config = config.clone();
        if new_config.id.is_empty() {
            new_config.id = uuid::Uuid::new_v4().to_string();
        }
        save_one(&new_config);
        invalidate_cache();
        Ok(new_config)
    }

    /// 删除指定 name 的 agent 配置
    pub fn delete(name: &str) -> Result<(), String> {
        let all = load_all_raw();
        let target = all
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("Chat agent '{}' not found", name))?;

        if let Some(active_name) = read_active() {
            if active_name.eq_ignore_ascii_case(name) {
                clear_active_file();
            }
        }

        delete_file(&target.id);
        invalidate_cache();
        Ok(())
    }

    /// 删除指定 id 的 agent 配置（运行时按 id 加载；与 delete(name) 并行）
    pub fn delete_by_id(id: &str) -> Result<(), String> {
        let all = load_all_raw();
        let target = all
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("Chat agent '{}' not found", id))?;

        if let Some(active_name) = read_active() {
            if active_name.eq_ignore_ascii_case(&target.name) {
                clear_active_file();
            }
        }

        delete_file(&target.id);
        invalidate_cache();
        Ok(())
    }

    /// 设置激活的 agent（按 name）
    pub fn set_active(name: &str) -> Result<ChatAgentConfig, String> {
        let config = Self::load(name).ok_or_else(|| format!("Chat agent '{}' not found", name))?;
        write_active(name);
        Ok(config)
    }

    /// 获取当前激活的 agent 配置
    pub fn get_active() -> Option<ChatAgentConfig> {
        let active_name = read_active()?;
        load_all_cached()
            .into_iter()
            .find(|c| c.name.eq_ignore_ascii_case(&active_name))
    }

    /// 获取 agent 总数
    pub fn count() -> usize {
        load_all_cached().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_config_has_empty_fields() {
        let cfg = ChatAgentConfig::new("test");
        assert_eq!(cfg.name, "test");
        assert!(cfg.persona.is_empty());
        assert_eq!(cfg.max_iterations, 15);
        assert!(cfg.requirements.is_empty());
        assert!(cfg.knowledge.is_empty());
        // style/model/max_tokens fields removed in frontend-backend alignment
        assert!(!cfg.id.is_empty());
    }

    #[test]
    fn test_build_chatagent_user_config_with_all_fields() {
        let mut cfg = ChatAgentConfig::new("test");
        cfg.persona = "You are a bot.".to_string();
        cfg.requirements = vec!["req1".to_string(), "req2".to_string()];

        let prompt = cfg.build_chatagent_user_config();
        assert!(prompt.contains("You are a bot."));
        assert!(prompt.contains("req1"));
        assert!(prompt.contains("req2"));
    }

    #[test]
    fn test_build_chatagent_user_config_empty() {
        let cfg = ChatAgentConfig::new("test");
        let prompt = cfg.build_chatagent_user_config();
        assert!(prompt.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = std::env::temp_dir().join("nuphus_test_chat_agent_flat");
        let _ = std::fs::create_dir_all(&tmp);

        let mut cfg = ChatAgentConfig::new("roundtrip_test");
        cfg.persona = "Helper".to_string();

        let json = serde_json::to_string_pretty(&cfg).unwrap();
        std::fs::write(tmp.join("test-id.json"), json).unwrap();

        let content = std::fs::read_to_string(tmp.join("test-id.json")).unwrap();
        let loaded: ChatAgentConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.name, "roundtrip_test");
        assert_eq!(loaded.persona, "Helper");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
