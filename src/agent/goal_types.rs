//! GoalType — Task type metadata labels
//!
//! Used for tool safety filtering, warmup reminders, and memory entry classification.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::sync::OnceLock;

/// Default for GoalType is ScriptingExec (for #[serde(default)] backward compatibility)
impl Default for GoalType {
    fn default() -> Self {
        GoalType::ScriptingExec
    }
}

/// Goal type — metadata label used for tool safety filtering and warmup reminders
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalType {
    /// Project analysis: explore structure, measure scale, verify claims, produce report
    ProjectAnalysis,
    /// Code generation: understand existing code, plan implementation, code, verify
    CodeGeneration,
    /// Debug diagnosis: reproduce, locate root cause, fix, verify
    DebugDiagnose,
    /// File operation: read/write/edit files, no complex analysis
    FileOperation,
    /// Research query: information retrieval, answer questions, no tool execution
    ResearchQuery,
    /// Script execution: safely execute commands/scripts, return structured results
    ScriptingExec,
}

/// Identity relationship configuration — AI identity info saved by frontend settings page
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationConfig {
    #[serde(default)]
    pub assistant_name: String,
    #[serde(default)]
    pub user_label: String,
    #[serde(default)]
    pub persona: String,
}

impl RelationConfig {
    pub fn is_empty(&self) -> bool {
        self.assistant_name.is_empty() && self.user_label.is_empty() && self.persona.is_empty()
    }
}

/// Decomposed task items from understanding phase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    /// GoalType this task belongs to (labeled by understanding phase LLM). Default general for backward compatibility
    #[serde(default)]
    pub goal_type: GoalType,
    /// Task description (high-level execution goal, LLM decides tools and steps)
    pub description: String,
}

/// Understanding phase output — structured result of the "understand first" step
///
/// Each GoalType's understanding prompt produces information with different focuses,
/// but all reduce to this structure for build_system_prompt to inject into execution prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnderstandingContext {
    /// Understanding summary: one-sentence summary of user's core intent
    /// New format: `type1+type2: <one sentence summary>`
    pub summary: String,
    /// Decomposed task list. Empty = pure chat direct reply, non-empty = sequential execution
    #[serde(default)]
    pub tasks: Vec<TaskItem>,
    /// Primary type combination (parsed from [Task Type] type1+type2)
    #[serde(default)]
    pub primary_types: Vec<GoalType>,
}

impl GoalType {
    pub fn all() -> &'static [GoalType] {
        use GoalType::*;
        &[
            ProjectAnalysis,
            CodeGeneration,
            DebugDiagnose,
            FileOperation,
            ResearchQuery,
            ScriptingExec,
        ]
    }

    pub fn id(self) -> &'static str {
        match self {
            GoalType::ProjectAnalysis => "project_analysis",
            GoalType::CodeGeneration => "code_generation",
            GoalType::DebugDiagnose => "debug_diagnose",
            GoalType::FileOperation => "file_operation",
            GoalType::ResearchQuery => "research_query",
            GoalType::ScriptingExec => "scripting_exec",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GoalType::ProjectAnalysis => "项目分析",
            GoalType::CodeGeneration => "代码编写",
            GoalType::DebugDiagnose => "调试诊断",
            GoalType::FileOperation => "文件操作",
            GoalType::ResearchQuery => "研究查询",
            GoalType::ScriptingExec => "脚本执行",
        }
    }

    /// Parse GoalType from string ID (case-insensitive)
    pub fn from_id(s: &str) -> Option<GoalType> {
        let s = s.trim().to_lowercase();
        let s = s.strip_prefix('[').unwrap_or(&s);
        let s = s.strip_suffix(']').unwrap_or(s);
        let s = s.trim();
        match s {
            "project_analysis" | "projectanalysis" => Some(GoalType::ProjectAnalysis),
            "code_generation" | "codegeneration" => Some(GoalType::CodeGeneration),
            "debug_diagnose" | "debugdiagnose" => Some(GoalType::DebugDiagnose),
            "file_operation" | "fileoperation" => Some(GoalType::FileOperation),
            "research_query" | "researchquery" => Some(GoalType::ResearchQuery),
            "scripting_exec" => Some(GoalType::ScriptingExec),
            // Backward compatibility: old "general" maps to ScriptingExec
            "general" => Some(GoalType::ScriptingExec),
            _ => None,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            GoalType::ProjectAnalysis => "项目架构分析：理解项目结构、模块关系、技术栈、代码规模。选这个当主要工作是'理解现有系统'。",
            GoalType::CodeGeneration => "代码工程实现：编写/修改/重构源代码。选这个当主要工作是'产出或修改代码'。",
            GoalType::DebugDiagnose => "问题排查修复：定位Bug根因并验证修复。选这个当输入是'有异常/有Bug'。",
            GoalType::FileOperation => "文件系统管理：文件增删移改、目录整理。选这个当主要工作是'管理文件'而非分析或编码。",
            GoalType::ResearchQuery => "外部信息检索：网络搜索、文档阅读、多源验证。选这个当需要从外部获取信息。",
            GoalType::ScriptingExec => "脚本命令执行：批量处理、测试运行、系统检查。选这个当主要工作是通过命令/脚本完成。",
        }
    }

    /// Selection guide — match GoalType based on task objective nature
    pub fn boundary(self) -> &'static str {
        match self {
            GoalType::ProjectAnalysis => {
                "适合：理解现有系统结构、梳理架构关系、评估技术栈、生成分析报告。"
            }
            GoalType::CodeGeneration => {
                "适合：开发新功能、修改现有代码、重构实现、接口设计、代码迁移。"
            }
            GoalType::DebugDiagnose => "适合：排查异常现象、定位Bug根因、修复验证、性能问题诊断。",
            GoalType::FileOperation => {
                "适合：文件系统管理、目录整理、批量重命名、文件迁移、权限设置。"
            }
            GoalType::ResearchQuery => {
                "适合：外部信息调研、网络搜索、文档阅读、多源交叉验证、知识汇总。"
            }
            GoalType::ScriptingExec => {
                "适合：命令行批量处理、自动化脚本执行、测试运行、环境检查、日志分析。"
            }
        }
    }

    /// Global circuit breaker limit: safety net to prevent LLM infinite loop.
    /// 死循环防护由 ProtectionGuard(dead_loop 阈值5)承担，此值仅为失控保险丝，
    /// 1000 轮对真实任务实质等同不设限。
    pub const MAX_ITERATIONS: usize = 1000;

    /// Whether it's an analytical task (exploration/research/analysis, not execution/operation)
    pub fn is_analysis(self) -> bool {
        matches!(self, GoalType::ProjectAnalysis | GoalType::ResearchQuery)
    }
}

impl fmt::Display for GoalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Model context window mapping
/// Prefers explicit context_window from config.toml (set after API query),
/// falls back to built-in ProviderRegistry metadata, then a reasonable default.
pub fn get_context_window(model_name: &str) -> usize {
    if let Ok(registry) = crate::config::load_registry() {
        if let Some((_, model)) = registry.find_model(model_name) {
            if let Some(window) = model.context_window {
                return window;
            }
        }
    }
    // Fall back to ProviderRegistry metadata table
    if let Some((_, meta)) =
        crate::config::registry::ProviderRegistry::builtin().find_model(model_name)
    {
        return meta.context_window as usize;
    }
    128_000
}

// Warmup reminders — compile-time embedded from goal_warmups.toml

#[derive(Debug, Deserialize)]
struct WarmupReminders {
    reminders: Vec<String>,
}

fn warmups_map() -> &'static HashMap<String, WarmupReminders> {
    static WARMUPS: OnceLock<HashMap<String, WarmupReminders>> = OnceLock::new();
    WARMUPS.get_or_init(|| {
        toml::from_str(include_str!("goal_warmups.toml")).expect("goal_warmups.toml 解析失败")
    })
}

/// 获取指定 goal_type 的预热提醒列表（全量，不做轮换）
pub fn get_warmup_reminders(goal_type: GoalType) -> Vec<String> {
    let map = warmups_map();
    let key = goal_type.id();
    map.get(key)
        .map(|w| w.reminders.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_types_have_unique_ids() {
        let mut ids = std::collections::HashSet::new();
        for gt in GoalType::all() {
            assert!(ids.insert(gt.id()), "duplicate id: {}", gt.id());
        }
    }

    #[test]
    fn test_build_exec_prompt_includes_identity() {
        let schemas = "test_schema";
        for gt in GoalType::all() {
            let prompt = crate::agent::prompt::build_exec_prompt(
                "test-model",
                schemas,
                *gt,
                "",
                None,
                false,
                None,
            );
            assert!(
                prompt.contains("Nuphus"),
                "identity missing from prompt for {}",
                gt.label()
            );
            assert!(prompt.contains("test-model"), "model missing from prompt");
        }
    }

    #[test]
    fn test_build_exec_prompt_with_tool_schemas() {
        let schemas = r#"{"tools": [{"name": "Read"}]}"#;
        let prompt = crate::agent::prompt::build_exec_prompt(
            "m",
            schemas,
            GoalType::CodeGeneration,
            "",
            None,
            false,
            None,
        );
        assert!(
            prompt.contains("Read"),
            "tool schema should appear in prompt"
        );
    }

    // ── 身份配置加载验证（用户要求：配了就不能出错，必须对应加载）──

    /// 前后端字段名对齐：前端 localStorage 存 camelCase（assistantName/userLabel），
    /// 经 Tauri invoke / HTTP JSON 到达后端 RelationConfig（snake_case + rename_all=camelCase）。
    /// 若此测试失败 = 字段名不匹配，配置的"丞相"会在反序列化时丢失 → fallback Nuphus。
    #[test]
    fn test_relation_config_camel_case_deserialize() {
        let json = r#"{"assistantName":"丞相","userLabel":"大王"}"#;
        let r: RelationConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            r.assistant_name, "丞相",
            "assistantName must map to assistant_name"
        );
        assert_eq!(r.user_label, "大王", "userLabel must map to user_label");
    }

    /// L0 身份构建：配置 assistant_name="丞相" → 提示词必须是「你是 丞相」，
    /// 绝不能 fallback 成 Nuphus；空配置才允许 fallback。
    #[test]
    fn test_leader_base_prompt_respects_assistant_name() {
        // 配置生效路径
        let relation = RelationConfig {
            assistant_name: "丞相".to_string(),
            user_label: "用户".to_string(),
            ..Default::default()
        };
        let ctx = crate::agent::prompt::LeaderContext {
            soul: String::new(),
            relation: Some(relation),
        };
        let prompt = crate::agent::prompt::build_leader_base_prompt(&ctx);
        assert!(
            prompt.contains("你是 丞相"),
            "configured assistant name must load into L0 identity:\n{prompt}"
        );
        assert!(
            !prompt.contains("你是 Nuphus"),
            "configured assistant name must NOT fallback to Nuphus:\n{prompt}"
        );

        // 空配置 fallback 路径（默认 Nuphus）
        let empty_ctx = crate::agent::prompt::LeaderContext {
            soul: String::new(),
            relation: None,
        };
        let fallback = crate::agent::prompt::build_leader_base_prompt(&empty_ctx);
        assert!(
            fallback.contains("你是 Nuphus"),
            "empty relation must fallback to Nuphus:\n{fallback}"
        );
    }

    #[test]
    fn test_research_query_id_and_label() {
        assert_eq!(GoalType::ResearchQuery.id(), "research_query");
        assert_eq!(GoalType::ResearchQuery.label(), "研究查询");
    }

    #[test]
    fn test_max_iterations_constant() {
        assert_eq!(GoalType::MAX_ITERATIONS, 1000);
    }

    #[test]
    fn test_default_goal_type_is_scripting_exec() {
        let item = TaskItem {
            goal_type: GoalType::default(),
            description: "test".into(),
        };
        assert_eq!(item.goal_type, GoalType::ScriptingExec);
    }
}
