//! One-shot host approval for the small set of critical desktop actions.
//!
//! The tool call waits here; approval resumes that exact call, not a model retry.
//! Binding data stays local, is never serialized into UI events, and cannot be
//! supplied through an `approved=true` tool argument or session-tool allowance.

use crate::agent::events::{EventEmitter, NuphusEvent, RiskLevel};
use crate::security::approval;
use crate::state::{SharedSignals, SignalState};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub const KIND: &str = "desktop_action";
pub const EVENT_TOOL: &str = "desktop_action_approval";
const TIMEOUT: Duration = Duration::from_secs(300);

tokio::task_local! {
    static CALL_CANCEL: Arc<AtomicBool>;
}

/// Workflow runs have their own stop flag, independent of the main conversation.
/// Scope a tool future with that flag so a stopped workflow cannot later resume
/// an approval dialog and dispatch the action.
pub async fn with_cancellation<F: std::future::Future>(
    cancel: Arc<AtomicBool>,
    future: F,
) -> F::Output {
    CALL_CANCEL.scope(cancel, future).await
}

fn cancelled(host: &ApprovalHost) -> bool {
    CALL_CANCEL
        .try_with(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or_else(|_| host.cancel.load(Ordering::SeqCst))
}

/// Reuse the same execution cancellation scope for read-only desktop waits.
pub fn cancellation_requested(signals: &SharedSignals) -> bool {
    CALL_CANCEL
        .try_with(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or_else(|_| {
            SignalState::read(signals)
                .security
                .desktop_approvals
                .host
                .as_ref()
                .is_some_and(|host| host.cancel.load(Ordering::SeqCst))
        })
}

/// Built from the locally resolved target/action and actual input, not the
/// decision model's output. An unscoped manual call gets its own request nonce.
#[derive(Clone, PartialEq, Eq)]
pub struct ApprovalBinding {
    owner: Option<String>,
    invocation: String,
    target_and_action: Value,
    parameters: Value,
}

impl ApprovalBinding {
    pub fn new(target_and_action: Value, parameters: Value) -> Self {
        Self {
            owner: crate::automation_gate::current_execution_owner(),
            invocation: uuid::Uuid::new_v4().to_string(),
            target_and_action,
            parameters,
        }
    }
}

#[derive(Clone)]
struct ApprovalHost {
    emitter: Arc<dyn EventEmitter>,
    cancel: Arc<AtomicBool>,
}

struct Ticket {
    binding: ApprovalBinding,
    created_at: Instant,
    response: oneshot::Sender<(ApprovalBinding, bool)>,
}

/// Lives in SharedSignals, so registries cloned for a workflow share only their
/// own host and approvals. Debug deliberately excludes text input and locators.
#[derive(Default)]
pub struct DesktopApprovalState {
    host: Option<ApprovalHost>,
    tickets: HashMap<String, Ticket>,
}

impl std::fmt::Debug for DesktopApprovalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopApprovalState")
            .field("host_installed", &self.host.is_some())
            .field("pending_count", &self.tickets.len())
            .finish()
    }
}

/// Install once from the trusted application host, not a tool/model endpoint.
pub fn install_host(
    signals: &SharedSignals,
    emitter: Arc<dyn EventEmitter>,
    cancel: Arc<AtomicBool>,
) {
    SignalState::write(signals).security.desktop_approvals.host =
        Some(ApprovalHost { emitter, cancel });
}

/// A pending request is only resolvable by a host UI action with its opaque ID.
/// Ordinary security result flags and session-level allowances are ignored.
pub fn resolve(signals: &SharedSignals, action_id: &str, approved: bool) -> Result<(), String> {
    let mut state = SignalState::write(signals);
    let valid_pending = state
        .security
        .pending_approvals
        .get(action_id)
        .is_some_and(|(pending, at)| pending.kind == KIND && at.elapsed() < TIMEOUT);
    let valid_ticket = state
        .security
        .desktop_approvals
        .tickets
        .get(action_id)
        .is_some_and(|ticket| ticket.created_at.elapsed() < TIMEOUT);
    if !valid_pending || !valid_ticket {
        if state
            .security
            .pending_approvals
            .get(action_id)
            .is_some_and(|(pending, _)| pending.kind == KIND)
        {
            state.security.pending_approvals.remove(action_id);
        }
        state.security.desktop_approvals.tickets.remove(action_id);
        return Err("桌面操作确认已过期或已结束，请重新观察目标".into());
    }
    let ticket = state
        .security
        .desktop_approvals
        .tickets
        .remove(action_id)
        .ok_or("桌面操作确认已结束")?;
    state.security.pending_approvals.remove(action_id);
    drop(state);
    ticket
        .response
        .send((ticket.binding, approved))
        .map_err(|_| "桌面操作已取消，未执行动作".to_string())
}

struct PendingGuard {
    signals: SharedSignals,
    action_id: String,
    emitter: Arc<dyn EventEmitter>,
    dismiss: bool,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut state = SignalState::write(&self.signals);
        state.security.pending_approvals.remove(&self.action_id);
        state
            .security
            .desktop_approvals
            .tickets
            .remove(&self.action_id);
        drop(state);
        if self.dismiss {
            self.emitter.emit(NuphusEvent::PromptTimeout {
                action_id: self.action_id.clone(),
            });
        }
    }
}

/// Approve only this pending invocation. Caller must then freshly resolve the
/// same locator and verify capability/risk before dispatch (the UI may change
/// while the user reads the prompt). This function does not run any action.
pub async fn authorize(
    signals: &SharedSignals,
    binding: ApprovalBinding,
    summary: &str,
) -> Result<(), String> {
    let host = SignalState::read(signals)
        .security
        .desktop_approvals
        .host
        .clone()
        .ok_or("此桌面操作需要一次确认，但当前运行入口未连接确认界面")?;
    if cancelled(&host) {
        return Err("任务已取消，未请求桌面操作确认".into());
    }
    if binding.owner != crate::automation_gate::current_execution_owner() {
        return Err("桌面操作所属执行轮次已变化".into());
    }
    let title = "确认本次桌面操作";
    let action_id = approval::add(signals, KIND, title, summary, serde_json::json!({}));
    let (tx, mut rx) = oneshot::channel();
    SignalState::write(signals)
        .security
        .desktop_approvals
        .tickets
        .insert(
            action_id.clone(),
            Ticket {
                binding: binding.clone(),
                created_at: Instant::now(),
                response: tx,
            },
        );
    let mut guard = PendingGuard {
        signals: signals.clone(),
        action_id: action_id.clone(),
        emitter: host.emitter.clone(),
        dismiss: true,
    };
    host.emitter.emit(NuphusEvent::SecurityCheck {
        action_id,
        tool: EVENT_TOOL.into(),
        params: serde_json::json!({"title": title, "content": summary}).to_string(),
        risk: RiskLevel::Critical,
        reason: "关键操作需要确认一次，批准后继续当前步骤".into(),
    });
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if cancelled(&host) {
            return Err("任务已取消，未执行桌面动作".into());
        }
        if Instant::now() >= deadline {
            return Err("桌面操作确认超时，未执行动作".into());
        }
        tokio::select! {
            response = &mut rx => {
                let (approved_binding, approved) = response
                    .map_err(|_| "桌面操作确认已过期或已取消")?;
                if approved_binding != binding
                    || binding.owner != crate::automation_gate::current_execution_owner()
                    || cancelled(&host)
                    || Instant::now() >= deadline
                {
                    return Err("桌面操作确认已失效，未执行动作".into());
                }
                guard.dismiss = false;
                return if approved { Ok(()) } else { Err("用户拒绝了本次桌面操作".into()) };
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEmitter(tokio::sync::mpsc::UnboundedSender<NuphusEvent>);
    impl EventEmitter for TestEmitter {
        fn emit(&self, event: NuphusEvent) {
            let _ = self.0.send(event);
        }
    }

    fn setup() -> (
        SharedSignals,
        Arc<AtomicBool>,
        tokio::sync::mpsc::UnboundedReceiver<NuphusEvent>,
    ) {
        let signals = crate::state::new_shared_signals();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        install_host(&signals, Arc::new(TestEmitter(tx)), cancel.clone());
        (signals, cancel, rx)
    }

    fn start(signals: &SharedSignals) -> tokio::task::JoinHandle<Result<(), String>> {
        let signals = signals.clone();
        tokio::spawn(crate::automation_gate::with_execution_owner(
            "test-owner".into(),
            async move {
                authorize(
                    &signals,
                    ApprovalBinding::new(
                        serde_json::json!({"locator":"local-target", "action":"delete"}),
                        serde_json::json!({"value":"private-input"}),
                    ),
                    "删除临时条目",
                )
                .await
            },
        ))
    }

    #[tokio::test]
    async fn read_only_wait_cancellation_uses_execution_scope_before_host() {
        let (signals, host_cancel, _rx) = setup();
        assert!(!cancellation_requested(&signals));
        host_cancel.store(true, Ordering::SeqCst);
        assert!(cancellation_requested(&signals));
        with_cancellation(Arc::new(AtomicBool::new(false)), async {
            assert!(!cancellation_requested(&signals));
        })
        .await;
        host_cancel.store(false, Ordering::SeqCst);
        with_cancellation(Arc::new(AtomicBool::new(true)), async {
            assert!(cancellation_requested(&signals));
        })
        .await;
    }

    async fn next_id(rx: &mut tokio::sync::mpsc::UnboundedReceiver<NuphusEvent>) -> String {
        match rx.recv().await.unwrap() {
            NuphusEvent::SecurityCheck {
                action_id,
                params,
                tool,
                ..
            } => {
                assert_eq!(tool, EVENT_TOOL);
                assert!(!params.contains("private-input"));
                assert!(!params.contains("local-target"));
                action_id
            }
            event => panic!("unexpected {event:?}"),
        }
    }

    #[tokio::test]
    async fn only_host_resolution_resumes_the_same_call_once() {
        let (signals, _, mut events) = setup();
        let task = start(&signals);
        let id = next_id(&mut events).await;
        crate::security::set_security_result(&signals, &id, true);
        crate::security::approve_session_tool(&signals, EVENT_TOOL);
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        assert!(resolve(&crate::state::new_shared_signals(), &id, true).is_err());
        resolve(&signals, &id, true).unwrap();
        assert!(resolve(&signals, &id, true).is_err());
        assert!(task.await.unwrap().is_ok());
        assert!(approval::get(&signals, &id).is_none());
    }

    #[tokio::test]
    async fn rejection_cancellation_and_dropped_future_leave_no_grant() {
        let (signals, cancel, mut events) = setup();
        let task = start(&signals);
        let id = next_id(&mut events).await;
        resolve(&signals, &id, false).unwrap();
        assert!(task.await.unwrap().unwrap_err().contains("拒绝"));

        let task = start(&signals);
        let id = next_id(&mut events).await;
        cancel.store(true, Ordering::SeqCst);
        assert!(task.await.unwrap().unwrap_err().contains("取消"));
        assert!(approval::get(&signals, &id).is_none());
        assert!(resolve(&signals, &id, true).is_err());
        assert!(matches!(
            events.recv().await,
            Some(NuphusEvent::PromptTimeout { .. })
        ));

        cancel.store(false, Ordering::SeqCst);
        let task = start(&signals);
        let id = next_id(&mut events).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(approval::get(&signals, &id).is_none());
        assert!(resolve(&signals, &id, true).is_err());
    }

    #[tokio::test]
    async fn expired_ticket_cannot_be_approved() {
        let (signals, _, mut events) = setup();
        let task = start(&signals);
        let id = next_id(&mut events).await;
        SignalState::write(&signals)
            .security
            .desktop_approvals
            .tickets
            .get_mut(&id)
            .unwrap()
            .created_at = Instant::now() - TIMEOUT;
        assert!(resolve(&signals, &id, true).is_err());
        assert!(task.await.unwrap().is_err());
        assert!(approval::get(&signals, &id).is_none());
    }

    #[tokio::test]
    async fn binding_cannot_move_between_owners_or_change_parameters() {
        let (signals, _, _) = setup();
        let binding = crate::automation_gate::with_execution_owner("first".into(), async {
            ApprovalBinding::new(serde_json::json!({"target":"one"}), serde_json::json!({}))
        })
        .await;
        let mut changed = binding.clone();
        changed.parameters = serde_json::json!({"changed":true});
        assert!(changed != binding);
        changed = binding.clone();
        changed.target_and_action = serde_json::json!({"target":"two"});
        assert!(changed != binding);
        assert!(crate::automation_gate::with_execution_owner(
            "second".into(),
            authorize(&signals, binding, "test")
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn workflow_uses_its_own_stop_flag() {
        let (signals, host_cancel, mut events) = setup();
        // A previously stopped conversation must not block a new workflow run.
        host_cancel.store(true, Ordering::SeqCst);
        let workflow_cancel = Arc::new(AtomicBool::new(false));
        let task_signals = signals.clone();
        let task = tokio::spawn(with_cancellation(workflow_cancel.clone(), async move {
            authorize(
                &task_signals,
                ApprovalBinding::new(serde_json::json!({}), serde_json::json!({})),
                "workflow action",
            )
            .await
        }));
        let id = next_id(&mut events).await;
        workflow_cancel.store(true, Ordering::SeqCst);
        // Even a simultaneous approval cannot revive the stopped workflow.
        let _ = resolve(&signals, &id, true);
        assert!(task.await.unwrap().is_err());
        assert!(approval::get(&signals, &id).is_none());
    }
}
