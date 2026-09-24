//! In-memory workflow commentary contract tests: no desktop, network or user DB writes.
use super::WorkflowAgent;
use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::api::{ApiClient, AssistantEvent, MessageRequest, ProviderKind};
use crate::session::ContentBlock;
use crate::tools::ToolRegistry;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct ScriptedClient {
    responses: Mutex<VecDeque<Vec<AssistantEvent>>>,
    requests: Mutex<Vec<MessageRequest>>,
}

#[async_trait::async_trait]
impl ApiClient for ScriptedClient {
    async fn stream(&self, request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        self.requests.lock().unwrap().push(request);
        let mut response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra model request");
        response.push(AssistantEvent::MessageStop);
        Ok(response)
    }

    fn model_name(&self) -> &str {
        "workflow-progress-test"
    }
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::DeepSeek
    }
    fn provider_name(&self) -> &str {
        ""
    }
}

#[derive(Default)]
struct CapturedEvents {
    events: Mutex<Vec<NuphusEvent>>,
    cancel_after_stream: Option<Arc<AtomicBool>>,
}

impl EventEmitter for CapturedEvents {
    fn emit(&self, event: NuphusEvent) {
        if matches!(event, NuphusEvent::ExecutionProgress { .. }) {
            if let Some(cancel) = &self.cancel_after_stream {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        self.events.lock().unwrap().push(event);
    }
}

impl CapturedEvents {
    fn progress(&self) -> Vec<NuphusEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, NuphusEvent::AssistantProgress { .. }))
            .cloned()
            .collect()
    }
}

fn report(id: &str, message: serde_json::Value) -> AssistantEvent {
    AssistantEvent::ToolUse {
        id: id.into(),
        name: "workflow_report_progress".into(),
        input: serde_json::json!({"message":message}).to_string(),
    }
}

fn fixture(
    responses: Vec<Vec<AssistantEvent>>,
    emitter: Option<Arc<CapturedEvents>>,
) -> (WorkflowAgent, Arc<ScriptedClient>) {
    let client = Arc::new(ScriptedClient {
        responses: Mutex::new(responses.into()),
        requests: Mutex::new(Vec::new()),
    });
    let mut tools = ToolRegistry::new();
    tools.register_workflow_only_tools();
    let emitter = emitter.map(|emitter| emitter as Arc<dyn EventEmitter>);
    let mut agent = WorkflowAgent::new(
        client.clone(),
        tools,
        emitter,
        None,
        "workflow-progress-test".into(),
        "user".into(),
        "test".into(),
        Default::default(),
        1.0,
    );
    // Skip user memory-file injection; cached prompt avoids loading user personalization.
    agent
        .session
        .push_user_internal("isolated test seed".into());
    agent.cached_prompt = Some("test workflow commentary".into());
    agent.config.max_iterations = 5;
    (agent, client)
}

fn stored_result(agent: &WorkflowAgent, id: &str) -> Option<(String, bool)> {
    agent
        .session
        .messages()
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if tool_use_id == id => Some((content.clone(), *is_error)),
            _ => None,
        })
}

#[test]
fn report_tool_is_discoverable_but_cannot_be_saved_as_workflow_step() {
    let mut tools = ToolRegistry::new();
    tools.register_workflow_only_tools();
    assert!(tools
        .tool_names()
        .iter()
        .any(|name| name == "workflow_report_progress"));
    assert!(!crate::tools::registry::is_workflow_step_tool(
        "workflow_report_progress"
    ));
}

#[tokio::test]
async fn report_continues_to_final_without_business_actions() {
    let capture = Arc::new(CapturedEvents::default());
    let (mut agent, client) = fixture(
        vec![
            vec![report("report-1", "正在检查临时目录。".into())],
            vec![report("report-2", "检查完成，准备汇总。".into())],
            vec![AssistantEvent::TextDelta("已完成。".into())],
        ],
        Some(capture.clone()),
    );
    let output = agent.run("", &None, &AtomicBool::new(false)).await.unwrap();
    assert!(output.success);
    assert_eq!(output.message, "已完成。");
    assert_eq!(client.requests.lock().unwrap().len(), 3);
    assert_eq!(capture.progress().len(), 2);
    assert_eq!(agent.tool_call_count, 0);
    assert!(agent.tools_used_this_turn.is_empty());
    let (result, failed) = stored_result(&agent, "report-1").unwrap();
    assert!(!failed);
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["delivered"], true);
    assert_eq!(result["message_id"], "report-1");
    assert_eq!(result["message"], "正在检查临时目录。");
    if let NuphusEvent::AssistantProgress { timestamp, .. } = &capture.progress()[0] {
        assert_eq!(result["timestamp"].as_u64(), Some(*timestamp));
    }
    let events = capture.events.lock().unwrap();
    assert!(
        events
            .iter()
            .position(|e| matches!(e, NuphusEvent::AssistantProgress { .. }))
            .unwrap()
            < events
                .iter()
                .position(|e| matches!(e, NuphusEvent::ExecutionCompleted { .. }))
                .unwrap()
    );
}

#[tokio::test]
async fn native_text_with_tool_is_promoted_once_and_deduplicated() {
    let capture = Arc::new(CapturedEvents::default());
    let (mut agent, _) = fixture(
        vec![
            vec![
                AssistantEvent::TextDelta("先检查当前状态。".into()),
                report("same-text", "先检查当前状态。".into()),
            ],
            vec![AssistantEvent::TextDelta("完成。".into())],
        ],
        Some(capture.clone()),
    );
    agent.run("", &None, &AtomicBool::new(false)).await.unwrap();
    let progress = capture.progress();
    assert_eq!(progress.len(), 1);
    let persisted_index = agent
        .session
        .messages()
        .iter()
        .position(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolUse { id, .. } if id == "same-text"))
        })
        .unwrap();
    match &progress[0] {
        NuphusEvent::AssistantProgress {
            message_id,
            text,
            replaces_draft,
            timestamp,
            ..
        } => {
            assert_eq!(
                message_id,
                &format!("{}:text:{}", agent.session.id, persisted_index)
            );
            assert_eq!(text, "先检查当前状态。");
            assert!(*replaces_draft);
            assert_eq!(
                Some(*timestamp),
                agent.session.messages()[persisted_index].timestamp
            );
        }
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn invalid_reports_are_errors_not_delivered_history() {
    let capture = Arc::new(CapturedEvents::default());
    let (mut agent, _) = fixture(
        vec![
            vec![
                report("empty", "  ".into()),
                report("wrong-type", 42.into()),
                report("long", "字".repeat(4001).into()),
            ],
            vec![AssistantEvent::TextDelta("完成。".into())],
        ],
        Some(capture.clone()),
    );
    agent.run("", &None, &AtomicBool::new(false)).await.unwrap();
    assert!(capture.progress().is_empty());
    for id in ["empty", "wrong-type", "long"] {
        let (content, failed) = stored_result(&agent, id).unwrap();
        assert!(failed);
        assert!(!content.contains("\"delivered\":true"));
    }
}

#[tokio::test]
async fn internal_refinement_does_not_publish_progress() {
    let capture = Arc::new(CapturedEvents::default());
    let (mut agent, _) = fixture(
        vec![
            vec![
                AssistantEvent::TextDelta("内部摘要过程".into()),
                report("internal", "不应公开".into()),
            ],
            vec![AssistantEvent::TextDelta("提炼摘要".into())],
        ],
        Some(capture.clone()),
    );
    agent.set_internal_input(true);
    agent.run("", &None, &AtomicBool::new(false)).await.unwrap();
    assert!(capture.progress().is_empty());
    assert!(stored_result(&agent, "internal").unwrap().1);
}

#[tokio::test]
async fn cancellation_before_run_skips_model_and_progress() {
    let capture = Arc::new(CapturedEvents::default());
    let (mut agent, client) = fixture(vec![], Some(capture.clone()));
    let output = agent.run("", &None, &AtomicBool::new(true)).await.unwrap();
    assert!(!output.success);
    assert!(client.requests.lock().unwrap().is_empty());
    assert!(capture.progress().is_empty());
}

#[tokio::test]
async fn cancellation_after_stream_does_not_promote_or_deliver_progress() {
    let cancel = Arc::new(AtomicBool::new(false));
    let capture = Arc::new(CapturedEvents {
        events: Mutex::new(Vec::new()),
        cancel_after_stream: Some(cancel.clone()),
    });
    let (mut agent, _) = fixture(
        vec![vec![
            AssistantEvent::TextDelta("取消后不能继续汇报".into()),
            report("cancelled-report", "取消后不能投递".into()),
        ]],
        Some(capture.clone()),
    );
    let output = agent.run("", &None, &cancel).await.unwrap();
    assert!(!output.success);
    assert!(capture.progress().is_empty());
    assert!(stored_result(&agent, "cancelled-report").is_none());
}

#[tokio::test]
async fn absent_emitter_cannot_claim_successful_delivery() {
    let (mut agent, _) = fixture(
        vec![
            vec![report("unavailable", "没有接收方".into())],
            vec![AssistantEvent::TextDelta("结束。".into())],
        ],
        None,
    );
    agent.run("", &None, &AtomicBool::new(false)).await.unwrap();
    assert!(stored_result(&agent, "unavailable").unwrap().1);
}
