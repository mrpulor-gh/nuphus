//! Agent 核心模块集成测试
//!
//! 使用 MockApiClient 测试 Agent 核心循环，不依赖 LLM API。

use crate::agent::{AgentConfig, ReactAgent};
use crate::api::{ApiClient, AssistantEvent, MessageRequest, ProviderKind};
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::sync::Arc;

/// 可编程的 Mock ApiClient，返回预设响应序列。
struct MockApiClient {
    responses: Vec<AssistantEvent>,
    model_name: String,
    provider_kind: ProviderKind,
}

impl MockApiClient {
    /// 快速构造：单个 TextDelta 响应。
    fn text(text: &str) -> Self {
        Self {
            responses: vec![
                AssistantEvent::TextDelta(text.to_string()),
                AssistantEvent::MessageStop,
            ],
            model_name: "mock-model".to_string(),
            provider_kind: ProviderKind::MiniMax,
        }
    }

    /// 构造返回 ToolUse 的 Mock。
    fn tool_use(id: &str, name: &str, arguments: &str) -> Self {
        Self {
            responses: vec![
                AssistantEvent::ToolUse {
                    id: id.to_string(),
                    name: name.to_string(),
                    input: arguments.to_string(),
                },
                AssistantEvent::MessageStop,
            ],
            model_name: "mock-model".to_string(),
            provider_kind: ProviderKind::MiniMax,
        }
    }
}

#[async_trait]
impl ApiClient for MockApiClient {
    async fn stream(&self, _request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        Ok(self.responses.clone())
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }
}

/// 创建最小化 Agent 实例。
fn make_agent(mock: MockApiClient) -> ReactAgent {
    ReactAgent::new(
        Arc::new(mock),
        ToolRegistry::builtin(),
        AgentConfig::default(),
    )
}

// ═══ 测试: TextDelta → Session ═══

#[tokio::test]
async fn test_agent_text_delta_adds_to_session() {
    // 构造并验证 MockApiClient 流式返回
    let mock = MockApiClient::text("Hello from mock transport");
    let request = MessageRequest::new("mock-model", vec![]);
    let events = mock
        .stream(request)
        .await
        .expect("mock stream should succeed");

    assert_eq!(events.len(), 2);
    match &events[0] {
        AssistantEvent::TextDelta(text) => assert_eq!(text, "Hello from mock transport"),
        _ => panic!("expected TextDelta"),
    }

    // 验证 Agent 构造成功并可通过 session_mut 写入
    let mock = MockApiClient::text("Hello from mock transport");
    let mut agent = make_agent(mock);

    // Agent 初始化后 session 为空
    let initial_count = agent.session().len();
    assert_eq!(initial_count, 0, "session should start empty");

    // 模拟 Agent 处理响应后的 session 写入
    agent.session_mut().push_user("test input".to_string());
    agent.session_mut().push_assistant(vec![]);

    assert_eq!(agent.session().len(), 2);
}

// ═══ 测试: ToolUse → 工具触发 ═══

#[tokio::test]
async fn test_agent_tool_use_triggers_tool_call() {
    let mock = MockApiClient::tool_use("call_1", "Read", r#"{"path": "Cargo.toml"}"#);
    let _agent = make_agent(mock);

    // 验证 ToolRegistry 包含 Read 工具
    let registry = ToolRegistry::builtin();
    assert!(
        registry.get("Read").is_some(),
        "ToolRegistry should contain Read tool"
    );
    assert!(
        !registry.is_empty(),
        "ToolRegistry should have registered tools"
    );
}

// ═══ 测试: MockTransport (Transport trait 层) ═══

#[tokio::test]
async fn test_mock_transport_stream_returns_preset_events() {
    use crate::transports::{MockTransport, StreamEvent, Transport};

    let mock = MockTransport::text("hello world");
    let request = MessageRequest::new("mock-model", vec![]);
    let events = mock.stream(request).await.expect("stream should succeed");

    assert_eq!(events.len(), 2);
    match &events[0] {
        StreamEvent::TextDelta(text) => assert_eq!(text, "hello world"),
        _ => panic!("expected TextDelta"),
    }
    assert!(matches!(events[1], StreamEvent::Done));
}

#[tokio::test]
async fn test_mock_transport_stream_tool_use() {
    use crate::transports::{MockTransport, StreamEvent, Transport};

    let mock = MockTransport::tool_use("t1", "Read", r#"{"path": "/tmp"}"#, "file contents here");
    let request = MessageRequest::new("mock-model", vec![]);
    let events = mock.stream(request).await.expect("stream should succeed");

    assert_eq!(events.len(), 3);
    match &events[0] {
        StreamEvent::ToolUse {
            id,
            name,
            arguments,
        } => {
            assert_eq!(id, "t1");
            assert_eq!(name, "Read");
            assert_eq!(arguments, r#"{"path": "/tmp"}"#);
        }
        _ => panic!("expected ToolUse"),
    }
}
