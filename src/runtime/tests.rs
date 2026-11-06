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

// ─────────────────────────────────────────────────────────────────────────
// 栈溢出回归（2026-11-06，STATUS_STACK_OVERFLOW 0xc00000fd）
//
// 背景：长会话（948 msgs / 5.35MB）执行「接着干」后 3 秒闪退。dump 证据：
//   - ExceptionCode = EXCEPTION_STACK_OVERFLOW，fault offset 0x3b66507
//   - 栈顶第 2 帧 = tokio::task::spawn::spawn，第 4 帧 = tauri::async_runtime::spawn
//   - 递归帧固定 10 帧周期，refine_in_loop 的 poll 函数出现 120 次
// 根因：in-loop refine 接力路径 refine_in_loop → resume → react_loop →
//   refine_in_loop 的**嵌套深度无界**。Box::pin 只固定 future 的大小，不消除
//   运行时栈帧；层数无界 → tokio worker 栈（2MB）爆 → 进程被杀 =「闪退」。
// 修复：MAX_REFINE_RECURSION_DEPTH 深度闸门 + ctx_hard_stop_done 提升到 Agent 级
//   + refine_count 恢复 per-turn 语义。
// ─────────────────────────────────────────────────────────────────────────

/// 计数型 mock：记录收到的 LLM 请求数（用于断言「闸门拒绝时根本没发请求」）。
struct CountingApiClient {
    requests: Arc<Mutex<usize>>,
}

impl CountingApiClient {
    fn new() -> (Self, Arc<Mutex<usize>>) {
        let requests = Arc::new(Mutex::new(0usize));
        (
            Self {
                requests: requests.clone(),
            },
            requests,
        )
    }
}

#[async_trait]
impl ApiClient for CountingApiClient {
    async fn stream(&self, _request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        *self.requests.lock().unwrap() += 1;
        Ok(vec![
            AssistantEvent::TextDelta("提炼摘要".to_string()),
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

/// 回归：嵌套深度达上限时，refine_in_loop 必须**拒绝**接力，且**不发起任何
/// LLM 请求**。
///
/// 修复前（无深度闸门）：refine_in_loop 会真的调用 resume → 发 LLM 请求 →
/// 嵌套层数无界 → 栈溢出。本测试在修复前会红（断言 false + 请求数为 0）。
#[test]
#[serial]
fn refine_in_loop_refuses_nested_resume_beyond_depth_limit() {
    use crate::agent::distill::MAX_REFINE_RECURSION_DEPTH;

    let (llm, request_count) = CountingApiClient::new();
    let mut runtime = RuntimeBuilder::new()
        .llm(Arc::new(llm))
        .tools(make_mock_tool_registry())
        .build()
        .expect("build should succeed");

    // 模拟「已在一次 refine 接力中」：深度已达上限。
    runtime.agent_mut().refine_depth = MAX_REFINE_RECURSION_DEPTH;

    let before = *request_count.lock().unwrap();
    let accepted = tokio_test::block_on(runtime.refine_in_loop(&AtomicBool::new(false)));
    let after = *request_count.lock().unwrap();

    assert!(
        !accepted,
        "深度达上限时 refine_in_loop 必须拒绝接力（返回 false），否则会无限嵌套 → 栈溢出"
    );
    assert_eq!(
        after - before,
        0,
        "深度闸门拒绝时不得发起任何 LLM 请求（实际发起 {} 次）",
        after - before
    );
    // 深度未被消耗（拒绝路径不应污染计数）。
    assert_eq!(
        runtime.agent().refine_depth,
        MAX_REFINE_RECURSION_DEPTH,
        "拒绝路径不应修改深度计数"
    );
}

/// 回归：refine_count / ctx_hard_stop_done 是 **per-turn** 语义——新轮次必须归零。
///
/// 修复前：refine_count 只在 Agent 构造时置 0，从不重置 → 长会话第 10 次提炼后
/// MAX_IN_LOOP_REFINES 闸门永久关闭，85% 硬线从此只剩软劝导，上下文无限膨胀。
#[test]
#[serial]
fn refine_gates_reset_on_new_turn() {
    use crate::agent::distill::MAX_IN_LOOP_REFINES;

    let llm = make_mock_llm();
    let tools = make_mock_tool_registry();
    let mut runtime = RuntimeBuilder::new()
        .llm(llm)
        .tools(tools)
        .build()
        .expect("build should succeed");

    // 模拟「上一轮已经把提炼预算用光」
    runtime.agent_mut().refine_count = MAX_IN_LOOP_REFINES;
    runtime.agent_mut().ctx_hard_stop_done = true;
    runtime.agent_mut().refine_depth = 3;

    // 新一轮（用户发消息）→ run() 应重置全部跨轮闸门。
    let _ = tokio_test::block_on(runtime.run("新轮次闸门重置验证", &None, &AtomicBool::new(false)));

    assert_eq!(
        runtime.agent().refine_count,
        0,
        "refine_count 应在新轮次归零（per-turn 语义）——否则提炼预算跨轮累积、闸门永久关闭"
    );
    assert!(
        !runtime.agent().ctx_hard_stop_done,
        "ctx_hard_stop_done 应在新轮次复位——否则第二轮起 85% 硬线完全失效"
    );
    assert_eq!(
        runtime.agent().refine_depth,
        0,
        "refine_depth 应在新轮次归零——否则跨轮残留深度会误伤本轮的合法接力"
    );
}

/// 回归：深度常量是**硬上限 1**——允许一次「提炼→压缩→接力续跑」，
/// 但禁止接力轮里再接力（这正是栈溢出的直接成因）。
#[test]
fn refine_recursion_depth_limit_is_one() {
    use crate::agent::distill::MAX_REFINE_RECURSION_DEPTH;
    assert_eq!(
        MAX_REFINE_RECURSION_DEPTH, 1,
        "只允许一层接力：第 2 层起必须降级软劝导，否则 tokio worker 栈会被撑爆"
    );
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
