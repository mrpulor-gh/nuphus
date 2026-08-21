//! Mock Transport — 预设响应序列，用于集成测试，不依赖 LLM API。
//!
//! 用法:
//! ```ignore
//! let mock = MockTransport::text("Hello from mock");
//! let events = mock.stream(request).await?;
//! ```

use crate::api::MessageRequest;
use crate::transports::StreamEvent;
use crate::Result;
use async_trait::async_trait;

/// 预设响应序列的 Mock Transport，用于 Agent 集成测试。
pub struct MockTransport {
    responses: Vec<StreamEvent>,
    model_name: String,
    provider_name: &'static str,
}

impl MockTransport {
    /// 用预设的 Vec<StreamEvent> 构造。
    /// 序列末尾会自动追加 Done（如果用户未添加）。
    pub fn new(responses: Vec<StreamEvent>) -> Self {
        let mut responses = responses;
        // 如果用户没有显式添加 Done，自动补一个
        if !matches!(
            responses.last(),
            Some(StreamEvent::Done) | Some(StreamEvent::Error(_)) | Some(StreamEvent::Cancelled)
        ) {
            responses.push(StreamEvent::Done);
        }
        Self {
            responses,
            model_name: "mock-model".to_string(),
            provider_name: "mock",
        }
    }

    /// 快速构造：单个 TextDelta → Done 的简单响应。
    pub fn text(text: &str) -> Self {
        Self::new(vec![StreamEvent::TextDelta(text.to_string())])
    }

    /// 构造返回 ToolUse + 文本响应的 Mock。
    /// 序列: ToolUse → TextDelta(then_text) → Done
    pub fn tool_use(id: &str, name: &str, arguments: &str, then_text: &str) -> Self {
        Self::new(vec![
            StreamEvent::ToolUse {
                id: id.to_string(),
                name: name.to_string(),
                arguments: arguments.to_string(),
            },
            StreamEvent::TextDelta(then_text.to_string()),
        ])
    }

    /// 设置 model name。
    pub fn with_model(mut self, model: &str) -> Self {
        self.model_name = model.to_string();
        self
    }
}

#[async_trait]
impl crate::transports::Transport for MockTransport {
    async fn stream(&self, _request: MessageRequest) -> Result<Vec<StreamEvent>> {
        Ok(self.responses.clone())
    }

    fn provider_name(&self) -> &'static str {
        self.provider_name
    }

    fn model(&self) -> &str {
        &self.model_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transports::Transport;

    #[test]
    fn test_mock_transport_text() {
        let mock = MockTransport::text("hello");
        assert_eq!(mock.model(), "mock-model");
        assert_eq!(mock.provider_name(), "mock");
        // 自动追加 Done
        assert_eq!(mock.responses.len(), 2);
        assert!(matches!(mock.responses[0], StreamEvent::TextDelta(_)));
        assert!(matches!(mock.responses[1], StreamEvent::Done));
    }

    #[test]
    fn test_mock_transport_tool_use() {
        let mock =
            MockTransport::tool_use("call_1", "Read", r#"{"path": "/tmp"}"#, "content from read");
        assert_eq!(mock.responses.len(), 3);
        match &mock.responses[0] {
            StreamEvent::ToolUse {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "Read");
                assert_eq!(arguments, r#"{"path": "/tmp"}"#);
            }
            _ => panic!("expected ToolUse"),
        }
    }
}
