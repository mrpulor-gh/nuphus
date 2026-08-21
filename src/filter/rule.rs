//! 过滤器规则定义 — JSON 序列化 + 规则匹配

use serde::{Deserialize, Serialize};

/// 匹配条件：根据 tool_name + 可选内容正则匹配
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleMatch {
    pub tool_names: Vec<String>,
    pub content_match: Option<String>,
}

/// 过滤操作：skip / keep 模式
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuleFilters {
    #[serde(default)]
    pub skip_patterns: Vec<String>,
    #[serde(default)]
    pub keep_patterns: Vec<String>,
}

/// 变换操作：ANSI 去除、截断、去重等
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RuleTransforms {
    #[serde(default)]
    pub strip_ansi: bool,
    #[serde(default)]
    pub trim_empty_edges: bool,
    #[serde(default)]
    pub dedupe_adjacent: bool,
    pub max_lines: Option<usize>,
    pub head_lines: Option<usize>,
    pub tail_lines: Option<usize>,
}

/// 完整规则定义（JSON 序列化）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FilterRule {
    pub id: String,
    pub description: Option<String>,
    pub r#match: RuleMatch,
    pub filters: Option<RuleFilters>,
    pub transforms: Option<RuleTransforms>,
}

impl FilterRule {
    /// 检查规则是否匹配给定的工具名
    pub fn matches_tool(&self, tool_name: &str) -> bool {
        self.r#match
            .tool_names
            .iter()
            .any(|t| t == tool_name || t == "*" || tool_name.starts_with(t.trim_end_matches("::*")))
    }

    /// 检查内容是否匹配可选的正则
    pub fn matches_content(&self, content: &str) -> bool {
        match &self.r#match.content_match {
            None => true,
            Some(pattern) => regex::Regex::new(pattern)
                .map(|re| re.is_match(content))
                .unwrap_or(true),
        }
    }
}
