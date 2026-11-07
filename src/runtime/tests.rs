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
use serial_test::serial;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

// ── Mock types ────────────────────────────────────────────────────────────

/// Programmable mock ApiClient returning preset AssistantEvent sequences.
struct MockApiClient {
    responses: Vec<AssistantEvent>,
    model_name: String,
    provider_kind: ProviderKind,
    requests: Mutex<Vec<MessageRequest>>,
}

impl MockApiClient {
    fn new(responses: Vec<AssistantEvent>) -> Self {
        Self {
            responses,
            model_name: "mock-model".to_string(),
            provider_kind: ProviderKind::MiniMax,
            requests: Mutex::new(vec![]),
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
    async fn stream(&self, request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        self.requests.lock().unwrap().push(request);
        Ok(self.responses.clone())
    }

    async fn stream_with_cancellation(
        &self,
        request: MessageRequest,
        _cancel_flag: &AtomicBool,
    ) -> crate::Result<Vec<AssistantEvent>> {
        self.requests.lock().unwrap().push(request);
        Ok(self.responses.clone())
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn provider_kind(&self) -> ProviderKind {
        self.provider_kind
    }

    fn provider_name(&self) -> &str {
        ""
    }
}

/// 固定返回非重试 LLM 错误的 mock——验证首刷语义：
/// 首个 LLM 请求即失败（400 非重试），react_loop 立即返回，LLM 从未成功过。
struct FailingApiClient;

#[async_trait]
impl ApiClient for FailingApiClient {
    async fn stream(&self, _request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        Err(crate::NuphusError::LLM(crate::LLMError::ApiError {
            status: 400,
            body: "injected non-retryable failure".to_string(),
        }))
    }

    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::MiniMax
    }

    fn provider_name(&self) -> &str {
        ""
    }
}

/// 每轮都发起一次 mock_echo 工具调用（参数含轮次序号——避开防死循环同参拦截），
/// 把 react 循环推进到 10+ 轮，验证轮中快照的「10 迭代」触发时机。
struct ToolCallingApiClient {
    calls: Mutex<usize>,
}

impl Default for ToolCallingApiClient {
    fn default() -> Self {
        Self {
            calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl ApiClient for ToolCallingApiClient {
    async fn stream(&self, _request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        Ok(vec![
            AssistantEvent::ToolUse {
                id: format!("snapshot-test-call-{n}"),
                name: "mock_echo".to_string(),
                input: format!(r#"{{"text": "iteration-{n}"}}"#),
            },
            AssistantEvent::MessageStop,
        ])
    }

    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::MiniMax
    }

    fn provider_name(&self) -> &str {
        ""
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

/** 复用同一会话时，视觉开关必须按精确 provider 绑定影响实际发出的图片消息。 */
#[tokio::test]
async fn reused_runtime_refreshes_vision_before_serializing_images() {
    let dir = std::env::temp_dir().join(format!("nuphus-视觉-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("providers.toml");
    let write_config = |supports_vision: bool| {
        let config = format!(
            r#"model = "shared-model"
[[providers]]
name = "other-account"
provider_type = "custom"
api_key = "test-only"
base_url = "https://example.invalid/v1"
[[providers.models]]
id = "shared-model"
supports_vision = true
[[providers]]
name = "selected-account"
provider_type = "custom"
api_key = "test-only"
base_url = "https://example.invalid/v1"
[[providers.models]]
id = "shared-model"
supports_vision = {supports_vision}
"#
        );
        std::fs::write(&path, config).unwrap();
    };
    write_config(false);
    let llm = Arc::new(MockApiClient::text("已收到"));
    let mut runtime = RuntimeBuilder::new()
        .llm(llm.clone())
        .tools(make_mock_tool_registry())
        .config(RuntimeConfig {
            agent_config: crate::agent::AgentConfig {
                model: "shared-model".into(),
                provider: "selected-account".into(),
                enable_memory: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .client_factory(crate::llm::ClientFactory::live_at(&path))
        .build()
        .unwrap();
    let session_id = runtime.session().id.clone();
    let image = "data:image/png;base64,test-fixture";
    let cancel = AtomicBool::new(false);
    for enabled in [false, true, false] {
        write_config(enabled);
        let output = runtime
            .run("识别图片", &Some(vec![image.into()]), &cancel)
            .await
            .unwrap();
        assert!(output.success);
        assert_eq!(runtime.session().id, session_id);
        assert_eq!(runtime.agent().config().supports_vision, enabled);
        assert_eq!(
            runtime.agent().config().vision_model.as_deref(),
            enabled.then_some("shared-model")
        );
        let requests = llm.requests.lock().unwrap();
        let request = requests.last().expect("模型必须收到请求");
        assert!(request
            .merged_system
            .as_ref()
            .unwrap()
            .contains(&format!("supports_vision: {enabled}")));
        let has_image = request.messages.iter().any(|message| {
            message["content"].as_array().is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block["type"] == "image_url" && block["image_url"]["url"] == image)
            })
        });
        assert_eq!(has_image, enabled, "图片必须跟随当前账号的视觉开关");
    }
    std::fs::remove_dir_all(dir).unwrap();
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
        large_force_refine_threshold: crate::agent::distill::LARGE_FORCE_DEFAULT,
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

/// 首刷语义：用户消息 push 之后、第一次 LLM 请求之前，快照已含该消息。
///
/// mock 首个请求即返回非重试错误（400）——LLM 从未成功过，快照依然落盘 →
/// 证明落盘先于首次 LLM 请求、且不依赖请求成功。
#[test]
#[serial]
fn react_loop_first_flush_persists_user_message_before_first_llm_call() {
    let mut runtime = RuntimeBuilder::new()
        .llm(Arc::new(FailingApiClient))
        .tools(make_mock_tool_registry())
        .config(RuntimeConfig {
            agent_config: crate::agent::AgentConfig {
                enable_memory: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .expect("build should succeed");
    let session_id = runtime.session().id.clone();

    let output =
        tokio_test::block_on(runtime.run("轮中快照首刷验证输入", &None, &AtomicBool::new(false)))
            .expect("非重试 LLM 错误应正常返回而非传播");
    assert!(!output.success, "首个请求即失败应标记为未成功");

    let (mode, json) = crate::store::session::get_snapshot(&session_id)
        .unwrap()
        .expect("首刷应已落盘（用户消息 push 之后、首次 LLM 请求之前）");
    assert_eq!(mode, "leader");
    let restored: Session = serde_json::from_str(&json).expect("快照 JSON 应可反序列化");
    assert!(
        restored
            .messages()
            .iter()
            .any(|m| m.text_content().contains("轮中快照首刷验证输入")),
        "首刷快照应已含用户消息"
    );

    crate::store::session::delete_session(&session_id).unwrap();
}

/// 轮中增量快照的 10 迭代触发：max_iterations=12 的工具循环跑完后，
/// 落盘快照应停在「第 10 轮开头」——含 iteration-1..10 的工具调用，
/// 不含 iteration-11/12（执行中崩溃最多丢失当前迭代的增量）。
#[test]
#[serial]
fn react_loop_in_loop_snapshot_captures_mid_loop_state() {
    let mut runtime = RuntimeBuilder::new()
        .llm(Arc::new(ToolCallingApiClient::default()))
        .tools(make_mock_tool_registry())
        .config(RuntimeConfig {
            agent_config: crate::agent::AgentConfig {
                max_iterations: 12,
                enable_memory: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .expect("build should succeed");
    let session_id = runtime.session().id.clone();

    let output =
        tokio_test::block_on(runtime.run("轮中快照多轮触发验证", &None, &AtomicBool::new(false)))
            .expect("工具循环应正常跑完 max_iterations");
    assert!(!output.success, "达到最大迭代次数应标记为未成功完成");

    let (mode, json) = crate::store::session::get_snapshot(&session_id)
        .unwrap()
        .expect("轮中快照应已落盘");
    assert_eq!(mode, "leader");
    assert!(json.contains("iteration-10"), "快照应含第 10 轮的工具调用");
    assert!(
        !json.contains("iteration-11"),
        "快照不应含第 11 轮——崩溃最多丢失当前迭代的增量"
    );

    let restored: Session = serde_json::from_str(&json).expect("快照 JSON 应可反序列化");
    assert_eq!(restored.id, session_id);
    assert!(
        restored
            .messages()
            .iter()
            .any(|m| m.text_content().contains("轮中快照多轮触发验证")),
        "快照应含用户消息"
    );

    // 终态（内存）领先快照恰好 2 个迭代——验证「增量」语义而非全量覆盖
    let final_json = serde_json::to_string(&runtime.session()).unwrap();
    assert!(final_json.contains("iteration-12"), "终态应含第 12 轮");
    assert!(
        restored.len() < runtime.session().len(),
        "快照应落后于终态（崩溃最多丢当前迭代的增量）"
    );

    crate::store::session::delete_session(&session_id).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────
// 提炼轮 / 任务轮身份分离回归（栈溢出 0xc00000fd）
//
// 旧机制：refine_in_loop → resume → react_loop 进入提炼轮后，85% 硬止损
// 仍然生效 → 再次 refine_in_loop → 无界嵌套（每层实占一个栈帧）。
// 新机制：react_loop 接 RunKind，RunKind::Refine 整段跳过上下文预算，
// 提炼轮在结构上不可能再触发提炼。
// ─────────────────────────────────────────────────────────────────────────

/// 构造一个能解析出上下文窗口的 runtime（builtin registry 的 minimax/
/// MiniMax-M3 = 250K；ctx 解析失败时测试会明确失败而不是静默跳过）。
fn runtime_for_refine_tests() -> Runtime {
    RuntimeBuilder::new()
        .llm(make_mock_llm())
        .tools(make_mock_tool_registry())
        .config(RuntimeConfig {
            agent_config: crate::agent::AgentConfig {
                model: "MiniMax-M3".into(),
                provider: "minimax".into(),
                enable_memory: false,
                max_iterations: 3,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .expect("build should succeed")
}

/// 10M 官方 input tokens：对任何常规窗口都远超 85% 硬线。
fn high_watermark_session() -> Session {
    let mut session = Session::new();
    session.push_user("长会话上下文".to_string());
    session.api_input_tokens = 10_000_000;
    session
}

#[test]
#[serial]
fn refine_run_kind_skips_context_budget_and_never_reenters() {
    assert!(
        crate::agent::goal_types::try_get_context_window_for("MiniMax-M3", Some("minimax"))
            .is_some(),
        "测试前提：MiniMax-M3/minimax 必须能解析上下文窗口"
    );

    let mut runtime = runtime_for_refine_tests();
    runtime.set_session(high_watermark_session());

    let output = tokio_test::block_on(runtime.resume(
        "提炼提示",
        &AtomicBool::new(false),
        RunKind::Refine,
    ))
    .expect("resume 应正常返回");

    assert!(output.success, "提炼轮应正常产出摘要");
    assert_eq!(
        runtime.agent().refine_count,
        0,
        "RunKind::Refine 必须跳过上下文预算：提炼轮不得再触发 in-loop refine"
    );
}

#[test]
#[serial]
fn task_run_at_high_watermark_refines_exactly_once() {
    let mut runtime = runtime_for_refine_tests();
    runtime.set_session(high_watermark_session());

    let output = tokio_test::block_on(runtime.run("接着干", &None, &AtomicBool::new(false)))
        .expect("run 应正常返回");

    assert!(output.success, "任务轮应正常完成");
    assert_eq!(
        runtime.agent().refine_count,
        1,
        "任务轮高水位应只提炼一次；>1 说明提炼轮又触发了提炼（无界嵌套）"
    );
}

#[test]
#[serial]
fn refine_in_loop_is_not_capped_by_refine_count() {
    let mut runtime = runtime_for_refine_tests();
    runtime.set_session(high_watermark_session());

    // 旧上限为 10；这里用一个远超它的计数，确认已无次数闸门。
    runtime.agent_mut().refine_count = 10_000;

    let refined = tokio_test::block_on(runtime.refine_in_loop(&AtomicBool::new(false)));

    assert!(refined, "refine_count 不再作为闸门：高计数时仍应正常提炼");
    assert!(
        runtime.agent().refine_count > 10_000,
        "计数器仍递增（现仅用于日志诊断）"
    );
}

/// 一次 run 内触发两次提炼的 mock：
/// - 调用 1、3 是提炼轮 → 返回摘要文本
/// - 调用 2 是任务轮 → 返回「超大 assistant 文本 + tool_use」，把上下文重新推过 85%
/// - 调用 4 是任务轮 → 返回最终文本
struct MultiRefineMock {
    calls: Mutex<usize>,
}

#[async_trait]
impl ApiClient for MultiRefineMock {
    async fn stream(&self, _request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        let n = {
            let mut c = self.calls.lock().unwrap();
            *c += 1;
            *c
        };
        Ok(match n {
            1 | 3 => vec![
                AssistantEvent::TextDelta(format!("摘要{n}")),
                AssistantEvent::MessageStop,
            ],
            2 => vec![
                // 2MB 文本：refine 提交后官方 token 被清零，estimate_token_usage
                // 按会话 JSON 字符数估算，足以再次越过 85% 硬线。
                AssistantEvent::TextDelta("x".repeat(2_000_000)),
                AssistantEvent::ToolUse {
                    id: "inflate-1".to_string(),
                    name: "mock_echo".to_string(),
                    input: serde_json::json!({ "text": "x" }).to_string(),
                },
                AssistantEvent::MessageStop,
            ],
            _ => vec![
                AssistantEvent::TextDelta("任务完成".to_string()),
                AssistantEvent::MessageStop,
            ],
        })
    }

    fn model_name(&self) -> &str {
        "MiniMax-M3"
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::MiniMax
    }

    fn provider_name(&self) -> &str {
        "minimax"
    }
}

#[test]
#[serial]
fn leader_refines_again_after_successful_refine_in_same_run() {
    let mut runtime = RuntimeBuilder::new()
        .llm(Arc::new(MultiRefineMock {
            calls: Mutex::new(0),
        }))
        .tools(make_mock_tool_registry())
        .config(RuntimeConfig {
            agent_config: crate::agent::AgentConfig {
                model: "MiniMax-M3".into(),
                provider: "minimax".into(),
                enable_memory: false,
                max_iterations: 6,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .expect("build should succeed");
    runtime.set_session(high_watermark_session());

    let output = tokio_test::block_on(runtime.run("长任务", &None, &AtomicBool::new(false)))
        .expect("run 应正常返回");

    assert!(output.success, "任务轮应正常完成");
    assert_eq!(
        runtime.agent().refine_count,
        2,
        "同一 run 内第一次提炼成功后，上下文再次越线应能继续提炼；旧的 ctx_hard_stop_done 会永久挡住第二次"
    );
}
