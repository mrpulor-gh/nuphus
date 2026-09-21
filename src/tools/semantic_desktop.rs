//! Accessibility/UIA-first desktop tools.
//!
//! The model can only observe an opaque candidate set and select a candidate
//! id. Native handles, coordinates and platform locators stay in the adapter.

use super::registry::ToolRegistry;
use crate::desktop_automation::{
    ActionCandidate, ActionClass, CandidateBuilder, CandidateKind, ComputerExecutor,
    ComputerObserver, DecisionInput, DecisionProvider, ExecutionGrant, ExecutionInput, JevClient,
    LocalPolicy, Observation, ObservationScope, Policy, PolicyDecision, RecentAction, Verification,
};
use crate::ToolResult;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const ACTION_SPACE_TTL: Duration = Duration::from_secs(120);
const AMBIGUITY_FLOOR: f64 = 0.20;
const HARD_STEP_LIMIT: u32 = 100;
const STALL_LIMIT: u32 = 3;

#[derive(Clone)]
pub(super) struct SemanticDesktopBackend {
    observer: Arc<dyn ComputerObserver>,
    candidates: Arc<dyn CandidateBuilder>,
    executor: Arc<dyn ComputerExecutor>,
    last_space: Arc<tokio::sync::Mutex<Option<ActionSpace>>>,
    loop_state: Arc<tokio::sync::Mutex<EnhancedLoopState>>,
}

#[derive(Clone)]
struct ActionSpace {
    token: String,
    created_at: Instant,
    observation: Observation,
    candidates: Vec<ActionCandidate>,
}

#[derive(Clone, Default)]
struct EnhancedLoopState {
    goal: String,
    steps: u32,
    consecutive_stalls: u32,
    recent_actions: Vec<RecentAction>,
}

impl SemanticDesktopBackend {
    pub(super) fn new<T>(adapter: Arc<T>) -> Self
    where
        T: ComputerObserver + CandidateBuilder + ComputerExecutor + 'static,
    {
        Self {
            observer: adapter.clone(),
            candidates: adapter.clone(),
            executor: adapter,
            last_space: Arc::new(tokio::sync::Mutex::new(None)),
            loop_state: Arc::new(tokio::sync::Mutex::new(EnhancedLoopState::default())),
        }
    }

    async fn observe(&self, goal: &str) -> Result<ActionSpace, String> {
        let observation = self
            .observer
            .observe(&ObservationScope::default())
            .await
            .map_err(|error| error.to_string())?;
        let candidates = self
            .candidates
            .build(goal, &observation)
            .map_err(|error| error.to_string())?;
        validate_action_space(&observation, &candidates)?;
        let space = ActionSpace {
            token: format!("obs:{}", Uuid::new_v4().simple()),
            created_at: Instant::now(),
            observation,
            candidates,
        };
        *self.last_space.lock().await = Some(space.clone());
        Ok(space)
    }

    async fn clear_space(&self) {
        *self.last_space.lock().await = None;
    }

    async fn begin_enhanced_step(&self, goal: &str) -> Result<Vec<RecentAction>, String> {
        let mut state = self.loop_state.lock().await;
        if state.goal != goal {
            *state = EnhancedLoopState {
                goal: goal.to_string(),
                ..Default::default()
            };
        }
        if state.steps >= HARD_STEP_LIMIT {
            return Err(format!(
                "增强桌面任务已达到 {HARD_STEP_LIMIT} 步硬上限，请检查目标或重新开始"
            ));
        }
        if state.consecutive_stalls >= STALL_LIMIT {
            return Err(format!(
                "连续 {STALL_LIMIT} 个语义动作未产生界面变化，已停止自动重试"
            ));
        }
        Ok(state.recent_actions.clone())
    }

    async fn record_enhanced_result(
        &self,
        candidate: &ActionCandidate,
        verification: Verification,
    ) {
        let mut state = self.loop_state.lock().await;
        state.steps = state.steps.saturating_add(1);
        if verification == Verification::NoChange {
            state.consecutive_stalls = state.consecutive_stalls.saturating_add(1);
        } else {
            state.consecutive_stalls = 0;
        }
        state.recent_actions.push(RecentAction {
            action_class: candidate.action_class(),
            target_summary: candidate.public_description.clone(),
            verification,
        });
        if state.recent_actions.len() > 8 {
            state.recent_actions.remove(0);
        }
    }

    async fn finish_enhanced_goal(&self) {
        *self.loop_state.lock().await = EnhancedLoopState::default();
    }
}

impl ToolRegistry {
    pub(super) async fn execute_semantic_desktop_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<ToolResult, String> {
        let Some(backend) = self.semantic_desktop.clone() else {
            return Ok(ToolResult::failure(
                "当前平台尚未提供 Accessibility/UIA 语义桌面适配器",
            ));
        };
        let result = match tool_name {
            "desktop_semantic_observe" => {
                let goal = params
                    .get("goal")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                backend
                    .observe(goal)
                    .await
                    .map(|space| action_space_json(&space))
            }
            "desktop_semantic_execute" => {
                let observation_token = required_string(params, "observation_token")?;
                let candidate_id = required_string(params, "candidate_id")?;
                let input = execution_input(params)?;
                execute_cached_candidate(&backend, observation_token, candidate_id, input).await
            }
            "desktop_agent_step" => {
                if !self.enhanced_mode {
                    Err("Jev 增强模式未启用；请使用 desktop_semantic_observe/desktop_semantic_execute".into())
                } else {
                    let goal = required_string(params, "goal")?;
                    execute_jev_step(&backend, goal).await
                }
            }
            _ => Err(format!("未知语义桌面工具: {tool_name}")),
        };
        Ok(match result {
            Ok(value) => ToolResult::success(value.to_string()),
            Err(error) => ToolResult::failure(error),
        })
    }
}

fn required_string<'a>(params: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    params
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} 不能为空"))
}

fn execution_input(params: &serde_json::Value) -> Result<ExecutionInput, String> {
    let Some(value) = params.get("value") else {
        return Ok(ExecutionInput::default());
    };
    if value.is_null() {
        return Ok(ExecutionInput::default());
    }
    let value = value
        .as_str()
        .ok_or_else(|| "value 必须是字符串".to_string())?;
    if value.chars().count() > 16_384 {
        return Err("value 不得超过 16384 个字符".into());
    }
    if value.contains('\0') {
        return Err("value 不得包含 NUL 字符".into());
    }
    Ok(ExecutionInput {
        value: Some(value.to_string()),
    })
}

async fn execute_cached_candidate(
    backend: &SemanticDesktopBackend,
    observation_token: &str,
    candidate_id: &str,
    input: ExecutionInput,
) -> Result<serde_json::Value, String> {
    let space = backend
        .last_space
        .lock()
        .await
        .clone()
        .ok_or_else(|| "没有可执行的语义观察；请先调用 desktop_semantic_observe".to_string())?;
    validate_space_token(&space, observation_token)?;
    let candidate = space
        .candidates
        .iter()
        .find(|candidate| candidate.id == candidate_id)
        .cloned()
        .ok_or_else(|| "candidate_id 不属于最近一次语义观察".to_string())?;
    execute_candidate(backend, &space.observation, &candidate, &input, None).await
}

async fn execute_jev_step(
    backend: &SemanticDesktopBackend,
    goal: &str,
) -> Result<serde_json::Value, String> {
    let recent_actions = backend.begin_enhanced_step(goal).await?;
    let space = backend.observe(goal).await?;
    let mut config = crate::config::load_registry()
        .map_err(|error| format!("读取 Jev 配置失败: {error}"))?
        .jev;
    if config.api_key.trim().is_empty() {
        return Err("Jev API Key 未配置".into());
    }
    let fallback_to_primary_model = config.fallback_to_primary_model;
    config.enabled = true;
    let client = JevClient::from_config(config).map_err(|error| error.to_string())?;
    let decision_input = DecisionInput {
        goal: goal.to_string(),
        observation: space.observation.clone(),
        candidates: space.candidates.clone(),
        recent_actions,
    };
    let decision = match client.choose(decision_input).await {
        Ok(decision) => decision,
        Err(error) if fallback_to_primary_model => {
            tracing::warn!(error = %error, "Jev semantic choice failed; falling back to primary model");
            return Ok(json!({
                "status": "needs_primary_decision",
                "reason": "Jev 暂时不可用，已回退当前主模型选择；详细网络或协议错误仅记录在本地诊断中",
                "action_space": action_space_json(&space),
            }));
        }
        Err(error) => return Err(format!("Jev 决策失败: {error}")),
    };
    if fallback_to_primary_model && decision.confidence.unwrap_or(1.0) < AMBIGUITY_FLOOR {
        // Keep the exact observed action space alive so the primary model can
        // execute one of the returned ids with the included token. Re-running
        // observe here would invalidate the adapter's opaque locator map.
        return Ok(json!({
            "status": "needs_primary_decision",
            "reason": "Jev 候选分布较分散，已回退当前主模型选择；这不是权限判断",
            "confidence": decision.confidence,
            "suggested_candidate_id": decision.candidate_id,
            "usage": decision.usage,
            "action_space": action_space_json(&space),
        }));
    }
    let candidate = space
        .candidates
        .iter()
        .find(|candidate| candidate.id == decision.candidate_id)
        .cloned()
        .ok_or_else(|| "Jev 返回了候选集合之外的 ID".to_string())?;
    if matches!(
        candidate.kind,
        CandidateKind::SetValue { .. } | CandidateKind::SetSecret { .. }
    ) {
        return Ok(json!({
            "status": "needs_input_value",
            "reason": "Jev 已选择文本目标；请由当前主模型提供业务文本并调用 desktop_semantic_execute。文本不会发送给 Jev",
            "observation_token": space.token,
            "candidate_id": candidate.id,
            "description": candidate.public_description,
            "decision": {
                "provider": "jev",
                "model": decision.actual_model,
                "confidence": decision.confidence,
                "usage": decision.usage,
            },
        }));
    }
    let result = execute_candidate(
        backend,
        &space.observation,
        &candidate,
        &ExecutionInput::default(),
        Some(json!({
            "provider": "jev",
            "model": decision.actual_model,
            "confidence": decision.confidence,
            "usage": decision.usage,
        })),
    )
    .await?;
    let verification = result
        .get("verification")
        .and_then(|value| serde_json::from_value::<Verification>(value.clone()).ok());
    if let Some(verification) = verification {
        backend
            .record_enhanced_result(&candidate, verification)
            .await;
    } else if matches!(candidate.kind, CandidateKind::Done) {
        backend.finish_enhanced_goal().await;
    }
    Ok(result)
}

fn validate_space_token(space: &ActionSpace, observation_token: &str) -> Result<(), String> {
    if space.token != observation_token {
        return Err("observation_token 不属于最近一次语义观察".into());
    }
    if space.created_at.elapsed() > ACTION_SPACE_TTL {
        return Err("语义观察已过期，请重新调用 desktop_semantic_observe".into());
    }
    Ok(())
}

async fn execute_candidate(
    backend: &SemanticDesktopBackend,
    before: &Observation,
    candidate: &ActionCandidate,
    input: &ExecutionInput,
    decision: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    match candidate.kind {
        CandidateKind::Done => {
            backend.clear_space().await;
            return Ok(json!({
                "status": "completed",
                "candidate_id": candidate.id,
                "decision": decision,
            }));
        }
        CandidateKind::AskUser => {
            return Ok(json!({
                "status": "needs_user_input",
                "candidate_id": candidate.id,
                "decision": decision,
            }));
        }
        CandidateKind::CannotProceed => {
            return Ok(json!({
                "status": "cannot_proceed",
                "candidate_id": candidate.id,
                "decision": decision,
            }));
        }
        _ => {}
    }

    let grant = grant_for(before);
    match LocalPolicy.evaluate(before, candidate, &grant) {
        PolicyDecision::Allow => {}
        PolicyDecision::NeedsConfirmation(reason) => {
            return Err(format!("该候选动作需要用户确认: {reason}"));
        }
        PolicyDecision::NeedsIncrementalGrant(reason) | PolicyDecision::Deny(reason) => {
            return Err(format!("本地策略拒绝执行: {reason}"));
        }
    }

    // Candidate IDs are observation-bound, but unrelated dynamic content must
    // not invalidate an otherwise stable semantic target. Re-read immediately
    // before dispatch and keep only the app/window boundary here; the adapter
    // re-resolves the local locator and verifies that the target still exposes
    // the requested native action.
    let fresh = backend
        .observer
        .observe(&ObservationScope::default())
        .await
        .map_err(|error| error.to_string())?;
    if fresh.app.id != before.app.id || fresh.window.id != before.window.id {
        backend.clear_space().await;
        return Err("前台应用或窗口已变化，候选动作已过期；请重新观察后再选择".into());
    }
    let receipt = backend
        .executor
        .execute(&fresh, candidate, input)
        .await
        .map_err(|error| error.to_string())?;
    if receipt.candidate_id != candidate.id {
        return Err("执行回执与候选动作不一致".into());
    }
    let (after, verification) = verify_with_settle(backend, &fresh, candidate).await?;
    backend.clear_space().await;
    Ok(json!({
        "status": "executed",
        "candidate_id": candidate.id,
        "description": candidate.public_description,
        "receipt": receipt,
        "verification": verification,
        "before_revision": fresh.revision,
        "after_revision": after.revision,
        "after_window": after.window,
        "decision": decision,
    }))
}

async fn verify_with_settle(
    backend: &SemanticDesktopBackend,
    before: &Observation,
    candidate: &ActionCandidate,
) -> Result<(Observation, Verification), String> {
    const ATTEMPTS: usize = 6;
    const SETTLE_MS: u64 = 125;

    let mut last = backend
        .observer
        .observe(&ObservationScope::default())
        .await
        .map_err(|error| error.to_string())?;
    for attempt in 0..ATTEMPTS {
        let verification = verify_observation(before, candidate, &last);
        if verification != Verification::NoChange || attempt + 1 == ATTEMPTS {
            return Ok((last, verification));
        }
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        last = backend
            .observer
            .observe(&ObservationScope::default())
            .await
            .map_err(|error| error.to_string())?;
    }
    unreachable!("bounded verification loop always returns")
}

fn verify_observation(
    before: &Observation,
    candidate: &ActionCandidate,
    after: &Observation,
) -> Verification {
    if before.app.id != after.app.id || before.window.id != after.window.id {
        return Verification::Progress;
    }
    let target_before = candidate
        .target
        .as_deref()
        .and_then(|id| before.nodes.iter().find(|node| node.opaque_id == id));
    let target_after = candidate
        .target
        .as_deref()
        .and_then(|id| after.nodes.iter().find(|node| node.opaque_id == id));
    let achieved = match (&candidate.kind, target_before, target_after) {
        (CandidateKind::Toggle, Some(old), Some(new)) => {
            old.toggled.zip(new.toggled).is_some_and(|(a, b)| a != b)
        }
        (CandidateKind::Select, _, Some(new)) => new.selected == Some(true),
        (CandidateKind::Expand, _, Some(new)) => new.expanded == Some(true),
        (CandidateKind::Collapse, _, Some(new)) => new.expanded == Some(false),
        (CandidateKind::Focus, _, Some(new)) => new.focused,
        (CandidateKind::SetValue { .. }, Some(old), Some(new)) => {
            old.value_fingerprint != new.value_fingerprint
        }
        _ => false,
    };
    if achieved {
        Verification::Achieved
    } else if after.fingerprint != before.fingerprint {
        Verification::Progress
    } else {
        Verification::NoChange
    }
}

fn grant_for(observation: &Observation) -> ExecutionGrant {
    ExecutionGrant {
        workflow_id: "interactive-workflow-session".into(),
        workflow_version: "current".into(),
        capability_manifest_digest: "interactive-semantic-desktop".into(),
        allowed_apps: vec![observation.app.id.clone()],
        action_classes: vec![
            ActionClass::NativeAction,
            ActionClass::SetValue,
            ActionClass::SetSecret,
            ActionClass::PressKey,
            ActionClass::Scroll,
            ActionClass::Wait,
            ActionClass::Control,
        ],
        secret_slot_ids: Vec::new(),
        visual_fallback: false,
        unattended: false,
        revoked: false,
    }
}

fn validate_action_space(
    observation: &Observation,
    candidates: &[ActionCandidate],
) -> Result<(), String> {
    if candidates.is_empty() {
        return Err("语义适配器没有生成候选动作".into());
    }
    if candidates.len() > 255 {
        return Err("候选动作超过 Jev Choice 的 255 项上限".into());
    }
    let mut ids = std::collections::HashSet::new();
    for candidate in candidates {
        if candidate.id.trim().is_empty() || !ids.insert(candidate.id.as_str()) {
            return Err("候选动作 ID 必须非空且唯一".into());
        }
        if candidate.observation_revision != observation.revision {
            return Err("候选动作与当前观察版本不一致".into());
        }
    }
    Ok(())
}

fn action_space_json(space: &ActionSpace) -> serde_json::Value {
    json!({
        "observation_token": space.token,
        "expires_in_ms": ACTION_SPACE_TTL.as_millis() as u64,
        "observation": {
            "revision": space.observation.revision,
            "app": space.observation.app,
            "window": space.observation.window,
            "element_count": space.observation.nodes.len(),
            "captured_at_ms": space.observation.captured_at_ms,
        },
        "candidates": space.candidates.iter().map(|candidate| json!({
            "id": candidate.id,
            "kind": candidate.kind,
            "description": candidate.public_description,
            "risk": candidate.local_risk,
            "target": candidate.target,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct FakeAdapter {
        executions: AtomicUsize,
        candidate_kind: CandidateKind,
        received_value: Mutex<Option<String>>,
    }

    fn observation() -> Observation {
        Observation {
            revision: 1,
            fingerprint: "stable-ui".into(),
            app: crate::desktop_automation::AppIdentity {
                id: "fake-app".into(),
                display_name: "Fake App".into(),
            },
            window: crate::desktop_automation::WindowIdentity {
                id: "fake-window".into(),
                title: "Fake Window".into(),
            },
            nodes: vec![],
            captured_at_ms: 1,
        }
    }

    #[async_trait]
    impl ComputerObserver for FakeAdapter {
        fn capabilities(&self) -> crate::desktop_automation::PlatformCapabilities {
            crate::desktop_automation::PlatformCapabilities {
                supported: vec![crate::desktop_automation::PlatformCapability::ReadSemanticTree],
                accessibility_permission: true,
            }
        }

        async fn observe(
            &self,
            _scope: &ObservationScope,
        ) -> Result<Observation, crate::desktop_automation::AutomationError> {
            Ok(observation())
        }
    }

    impl CandidateBuilder for FakeAdapter {
        fn build(
            &self,
            _goal: &str,
            observation: &Observation,
        ) -> Result<Vec<ActionCandidate>, crate::desktop_automation::AutomationError> {
            Ok(vec![ActionCandidate {
                id: "fake-candidate".into(),
                observation_revision: observation.revision,
                target: Some("fake-target".into()),
                kind: self.candidate_kind.clone(),
                public_description: "Invoke fake button".into(),
                local_risk: crate::desktop_automation::RiskClass::Reversible,
                preconditions: vec![],
                expected_effects: vec![],
            }])
        }
    }

    #[async_trait]
    impl ComputerExecutor for FakeAdapter {
        async fn execute(
            &self,
            _fresh: &Observation,
            action: &ActionCandidate,
            input: &ExecutionInput,
        ) -> Result<
            crate::desktop_automation::ActionReceipt,
            crate::desktop_automation::AutomationError,
        > {
            self.executions.fetch_add(1, Ordering::SeqCst);
            *self.received_value.lock().unwrap() = input.value.clone();
            Ok(crate::desktop_automation::ActionReceipt {
                candidate_id: action.id.clone(),
                dispatched: true,
                detail: None,
            })
        }
    }

    #[tokio::test]
    async fn semantic_execute_requires_matching_observation_token() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let observed = registry
            .execute_semantic_desktop_tool("desktop_semantic_observe", &json!({ "goal": "test" }))
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(observed.output.as_deref().unwrap()).unwrap();
        let token = payload["observation_token"].as_str().unwrap();

        let rejected = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_execute",
                &json!({
                    "observation_token": "obs:wrong",
                    "candidate_id": "fake-candidate"
                }),
            )
            .await
            .unwrap();
        assert!(!rejected.success);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);

        let executed = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_execute",
                &json!({
                    "observation_token": token,
                    "candidate_id": "fake-candidate"
                }),
            )
            .await
            .unwrap();
        assert!(executed.success);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
        assert_eq!(*adapter.received_value.lock().unwrap(), None);
    }

    fn stateful_observation(fingerprint: &str, toggled: bool) -> Observation {
        let mut observation = observation();
        observation.fingerprint = fingerprint.into();
        observation.nodes = vec![crate::desktop_automation::UiNode {
            opaque_id: "target".into(),
            role: crate::desktop_automation::UiRole::CheckBox,
            name: Some("Option".into()),
            short_value: None,
            enabled: true,
            visible: true,
            focused: false,
            secure: false,
            toggled: Some(toggled),
            selected: None,
            expanded: None,
            value_fingerprint: None,
            supported_actions: vec![crate::desktop_automation::NativeAction::Toggle],
        }];
        observation
    }

    #[tokio::test]
    async fn semantic_set_value_stays_local_and_preserves_whitespace() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::SetValue {
                slot_id: "value".into(),
            },
            received_value: Mutex::new(None),
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let observed = registry
            .execute_semantic_desktop_tool("desktop_semantic_observe", &json!({ "goal": "fill" }))
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(observed.output.as_deref().unwrap()).unwrap();
        let token = payload["observation_token"].as_str().unwrap();
        let executed = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_execute",
                &json!({
                    "observation_token": token,
                    "candidate_id": "fake-candidate",
                    "value": "  local text  "
                }),
            )
            .await
            .unwrap();

        assert!(executed.success);
        assert_eq!(
            adapter.received_value.lock().unwrap().as_deref(),
            Some("  local text  ")
        );
        assert!(!executed
            .output
            .as_deref()
            .unwrap_or_default()
            .contains("local text"));
    }

    #[tokio::test]
    async fn jev_step_is_rejected_when_enhanced_mode_is_off() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter);
        let result = registry
            .execute_semantic_desktop_tool("desktop_agent_step", &json!({ "goal": "test" }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("未启用"));
    }

    #[test]
    fn verification_prefers_target_state_over_generic_window_change() {
        let before = stateful_observation("before", false);
        let after = stateful_observation("after", true);
        let candidate = ActionCandidate {
            id: "toggle".into(),
            observation_revision: 1,
            target: Some("target".into()),
            kind: CandidateKind::Toggle,
            public_description: "Toggle option".into(),
            local_risk: crate::desktop_automation::RiskClass::Reversible,
            preconditions: vec![],
            expected_effects: vec![],
        };

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Achieved
        );
    }
}
