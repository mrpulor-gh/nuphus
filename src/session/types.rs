//! Session 核心类型定义
//!
//! 包含 MessageRole/ContentBlock/Message 等消息类型，
//! 以及 RefineStrategy/TokenUsage 等辅助类型。

use serde::{Deserialize, Serialize};

// ============================================================================
// MessageRole
// ============================================================================

/// 消息角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

// ============================================================================
// ContentBlock
// ============================================================================

/// 消息内容块
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(default)]
        reasoning: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        #[serde(rename = "tool_call_id")]
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// 图片附件（base64 data URL）
    Image { url: String },
    /// 音频附件（base64 data URL）
    Audio { url: String },
}

// ============================================================================
// Message
// ============================================================================

/// 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
    /// 内部注入消息标记（reminders/门铃/追加指令/系统提示等只进 LLM 上下文，
    /// 不显示在前端历史）。旧备份 JSON 反序列化默认 false，兼容历史数据。
    #[serde(default)]
    pub internal: bool,
    /// 消息创建时间（Unix 毫秒）。旧备份 JSON 无此字段 → 反序列化 None，
    /// 前端历史不显示时间（新消息由构造器填充）。
    #[serde(default)]
    pub timestamp: Option<u64>,
}

/// 当前 Unix 毫秒时间戳（消息创建时间）
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Message {
    /// 创建用户文本消息
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: text.into(),
                reasoning: None,
            }],
            internal: false,
            timestamp: Some(now_ms()),
        }
    }

    /// 创建内部注入用户文本消息（reminders/门铃/追加指令等，不显示在前端）
    pub fn user_text_internal(text: impl Into<String>) -> Self {
        let mut m = Self::user_text(text);
        m.internal = true;
        m
    }

    /// 创建助手消息
    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: blocks,
            internal: false,
            timestamp: Some(now_ms()),
        }
    }

    /// 创建工具结果消息（MiniMax API 兼容使用 user role）
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
                is_error,
            }],
            internal: false,
            timestamp: Some(now_ms()),
        }
    }

    /// 获取文本内容（合并所有 text block）
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 获取所有图片 URL
    pub fn image_urls(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Image { url } => Some(url.clone()),
                _ => None,
            })
            .collect()
    }

    /// 获取所有音频 URL
    pub fn audio_urls(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Audio { url } => Some(url.clone()),
                _ => None,
            })
            .collect()
    }
}

// ============================================================================
// Refine 策略
// ============================================================================

/// 提炼策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RefineStrategy {
    /// 语义摘要（LLM 决策策略）
    #[default]
    SemanticSummary,
    /// 不提炼（调试或短会话用）
    None,
}

/// 提炼结果元数据
#[derive(Debug, Clone)]
pub struct RefineResult {
    pub strategy: RefineStrategy,
    pub messages_removed: usize,
    pub messages_kept: usize,
    pub summary_chars: usize,
}

/// 提炼配置：上下文使用阈值
pub struct RefineConfig {
    /// 触发提炼的上下文使用比例 (0.0 ~ 1.0)
    pub threshold: f64,
}

impl Default for RefineConfig {
    fn default() -> Self {
        Self { threshold: 0.60 }
    }
}

// ============================================================================
// TokenUsage
// ============================================================================

/// 用量统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

impl TokenUsage {
    pub fn total(&self) -> u32 {
        self.input_tokens
            + self.output_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
    }
}
