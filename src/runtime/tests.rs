//! Runtime integration tests
//!
//! Validates RuntimeBuilder construction, Session lifecycle, Mode switching,
//! turn advancement, and ReAct cycle component wiring without actual LLM calls.

use super::*;
use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::api::{ApiClient, AssistantEvent, MessageRequest, ProviderKind};
use crate::permissions::ToolCategory;
use crate::session::Session;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;
use async_trait::async_trait;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

// ── Mock types ────────────────────────────────────────────────────────────

/// Programmable mock ApiClient returning preset AssistantEvent sequences.
struct MockApiClient {
    responses: Vec<AssistantEvent>,
    model_name: String,
    provider_kind: ProviderKind,
}

impl MockApiClient {
    fn new(responses: Vec<AssistantEvent>) -> Self {
        Self {
            responses,
            model_name: "mock-model".to_string(),
            provider_kind: ProviderKind::MiniMax,
        }
    }

    fn text(text: &str) -> Self {
        Self::new(vec![
            AssistantEvent::TextDelta(text.to_string()),
            AssistantEvent::MessageStop,
        ])
    }
}

#[async_trait]
impl ApiClient for MockApiClient {
    async fn stream(&self, _request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        Ok(self.responses.clone())
    }

    async fn stream_with_cancellation(
        &self,
        _request: MessageRequest,
        _cancel_flag: &AtomicBool,
    ) -> crate::Result<Vec<AssistantEvent>> {
        Ok(self.responses.clone())
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }
}

/// Recording mock EventEmitter capturing all emitted events.
struct MockEventEmitter {
    events: Mutex<Vec<NuphusEvent>>,
}

impl MockEventEmitter {
    fn new() -> Self {
        Self {
            events: Mutex::new(vec![]),
        }
    }

    fn count(&self) -> usize {
        self.events.lock().unwrap().len()
    }
}

impl EventEmitter for MockEventEmitter {
    fn emit(&self, event: NuphusEvent) {
        self.events.lock().unwrap().push(event);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_mock_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(ToolDef {
        name: "mock_echo".to_string(),
        description: "Echo back the input for testing".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Text to echo" }
            },
            "required": ["text"]
        }),
        category: ToolCategory::Core,
        executor: |params, _ctx| {
            let text = params
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("no text");
            Ok(ToolResult::success(format!("echo: {}", text)))
        },
        depends_on: vec![],
    });
    registry
}

fn make_mock_llm() -> Arc<dyn ApiClient> {
    Arc::new(MockApiClient::text("Hello from mock"))
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// Verify RuntimeBuilder can construct a Runtime with all required components.
#[test]
fn test_runtime_builder_builds() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let runtime = RuntimeBuilder::new()
        .llm(llm.clone())
        .tools(tools.clone())
        .build()
        .expect("RuntimeBuilder::build should succeed with llm + tools");

    // Verify core fields are populated
    assert_eq!(runtime.mode(), Mode::Leader);
    assert!(!runtime.session().id.is_empty());
    assert_eq!(runtime.session().len(), 0);
    assert!(runtime.llm().model_name() == "mock-model");

    // Verify configuration
    let config = runtime.config();
    assert!(!config.model.is_empty());
    assert_eq!(config.provider, "minimax");
    assert!(config.max_iterations > 0);
}

/// Verify RuntimeBuilder fails without required fields.
#[test]
fn test_runtime_builder_requires_llm() {
    let tools = make_mock_tool_registry();
    let result = RuntimeBuilder::new().tools(tools).build();
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.contains("llm is required"));
}

#[test]
fn test_runtime_builder_requires_tools() {
    let llm = make_mock_llm();
    let result = RuntimeBuilder::new().llm(llm).build();
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.contains("tools is required"));
}

/// Verify Session injection and recovery.
#[test]
fn test_session_inject_and_recover() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let mut runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .build()
        .expect("build should succeed");

    // Create a pre-populated session
    let mut session = Session::new();
    let session_id = session.id.clone();
    session.push_user("Hello, this is a test message.".to_string());
    session.advance_turn();
    assert_eq!(session.len(), 1);
    assert_eq!(session.current_turn_id(), "t1");

    // Inject session
    runtime.set_session(session);

    // Verify session is recovered
    let recovered = runtime.session();
    assert_eq!(recovered.id, session_id);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered.current_turn_id(), "t1");

    // Verify mutation through session_mut
    {
        let s = runtime.session_mut();
        s.push_user("Second message.".to_string());
    }
    assert_eq!(runtime.session().len(), 2);
}

/// Verify Session isolation: setting a new session replaces the old one.
#[test]
fn test_session_isolation() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let mut runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .build()
        .expect("build should succeed");

    // First session
    let s1 = Session::new();
    let id1 = s1.id.clone();
    runtime.set_session(s1);
    assert_eq!(runtime.session().id, id1);

    // Second session
    let s2 = Session::new();
    let id2 = s2.id.clone();
    runtime.set_session(s2);
    assert_eq!(runtime.session().id, id2);

    // Verify isolation: id1 is no longer active
    assert_ne!(runtime.session().id, id1);
}

/// Verify Mode switching preserves Runtime state.
#[test]
fn test_mode_switching() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .mode(Mode::Leader)
        .build()
        .expect("build should succeed");

    assert_eq!(runtime.mode(), Mode::Leader);

    // No mutable access needed for mode() — mode is on config.
    // set_mode is tested indirectly below.
}

/// Verify RuntimeBuilder::mode() sets initial mode correctly.
#[test]
fn test_runtime_builder_mode_leader() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .mode(Mode::Leader)
        .build()
        .expect("build should succeed");

    assert_eq!(runtime.mode(), Mode::Leader);
}

#[test]
fn test_runtime_builder_mode_workflow() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .mode(Mode::Workflow)
        .build()
        .expect("build should succeed");

    assert_eq!(runtime.mode(), Mode::Workflow);
}

/// Verify set_mode transitions are handled.
#[test]
fn test_set_mode_transition() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let mut runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .mode(Mode::Leader)
        .build()
        .expect("build should succeed");

    assert_eq!(runtime.mode(), Mode::Leader);

    // Leader → Workflow
    runtime.set_mode(Mode::Workflow);
    assert_eq!(runtime.mode(), Mode::Workflow);

    // Workflow → Leader
    runtime.set_mode(Mode::Leader);
    assert_eq!(runtime.mode(), Mode::Leader);

    // Idempotent
    runtime.set_mode(Mode::Leader);
    assert_eq!(runtime.mode(), Mode::Leader);
}

/// Verify turn advancement through Session.
#[test]
fn test_turn_advancement() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let mut runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .build()
        .expect("build should succeed");

    // Initial state
    assert_eq!(runtime.session().current_turn_id(), "t0");
    assert_eq!(runtime.session().turn_count, 0);

    // Advance turns
    let t1 = runtime.session_mut().advance_turn();
    assert_eq!(t1, "t1");
    assert_eq!(runtime.session().turn_count, 1);

    let t2 = runtime.session_mut().advance_turn();
    assert_eq!(t2, "t2");
    assert_eq!(runtime.session().turn_count, 2);

    let t3 = runtime.session_mut().advance_turn();
    assert_eq!(t3, "t3");
    assert_eq!(runtime.session().turn_count, 3);
}

/// Verify token usage tracking on Session.
#[test]
fn test_token_usage_initial_state() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .build()
        .expect("build should succeed");

    // Fresh session starts with zero tokens
    assert_eq!(runtime.session().api_input_tokens, 0);
    assert_eq!(runtime.session().turn_count, 0);
}

/// Verify RuntimeBuilder with emitter passes through.
#[test]
fn test_runtime_builder_with_emitter() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();
    let emitter = Arc::new(MockEventEmitter::new());

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .emitter(emitter.clone())
        .build()
        .expect("build should succeed");

    // Runtime built successfully with emitter
    assert_eq!(runtime.mode(), Mode::Leader);

    // Emitter should have no events yet (no run called)
    assert_eq!(emitter.count(), 0);
}

/// Verify RuntimeBuilder with pause_flag passes through.
#[test]
fn test_runtime_builder_with_pause_flag() {
    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();
    let pause = Arc::new(AtomicBool::new(false));

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .pause_flag(pause.clone())
        .build()
        .expect("build should succeed");

    assert_eq!(runtime.mode(), Mode::Leader);
}

/// Verify tool_registry registered mock tool can execute.
#[tokio::test]
async fn test_mock_tool_registered_and_executable() {
    let registry = make_mock_tool_registry();

    // Verify registration
    let def = registry.get("mock_echo");
    assert!(def.is_some());
    assert_eq!(def.unwrap().name, "mock_echo");

    // Execute the tool
    let result = registry
        .execute("mock_echo", &serde_json::json!({"text": "hello world"}))
        .await;
    assert!(result.is_ok());
    let tool_result = result.unwrap();
    assert!(tool_result.success);
    let output = tool_result.output.expect("tool should have output");
    assert!(output.contains("echo: hello world"));
}

// ── Config/RuntimeConfig tests ────────────────────────────────────────────

/// Verify RuntimeConfig default values.
#[test]
fn test_runtime_config_defaults() {
    let config = RuntimeConfig::default();
    assert_eq!(config.mode, Mode::Leader);
    assert!(!config.agent_config.model.is_empty());
    assert!(config.agent_config.max_iterations > 0);
}

/// Verify RuntimeConfig with AgentConfig flows through to Runtime.
#[test]
fn test_runtime_config_custom_max_iterations() {
    let agent_config = crate::agent::AgentConfig {
        max_iterations: 42,
        ..Default::default()
    };

    let config = RuntimeConfig {
        mode: Mode::Leader,
        agent_config,
        refine_threshold: 0.5,
        tool_permissions: Arc::new(Mutex::new(crate::permissions::ToolPermissions::default())),
    };

    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();

    let runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .config(config)
        .build()
        .expect("build should succeed");

    assert_eq!(runtime.config().max_iterations, 42);
}
