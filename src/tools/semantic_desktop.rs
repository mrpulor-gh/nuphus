//! Accessibility/UIA-first desktop tools.
//!
//! The model can only observe an opaque candidate set and select a candidate
//! id. Native handles, coordinates and platform locators stay in the adapter.

use super::registry::ToolRegistry;
use crate::desktop_automation::{
    ActionCandidate, ActionClass, CandidateBuilder, CandidateKind, ComputerExecutor,
    ComputerObserver, DecisionInput, DecisionProvider, ExecutionGrant, ExecutionInput, JevClient,
    LocalPolicy, NativeAction, Observation, ObservationScope, Policy, PolicyDecision, RecentAction,
    SemanticLocator, Verification,
};
use crate::ToolResult;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const ACTION_SPACE_TTL: Duration = Duration::from_secs(120);
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
    persistent_locators: std::collections::HashMap<String, SemanticLocator>,
    enhanced_goal_key: Option<String>,
}

#[derive(Clone, Default)]
struct EnhancedLoopState {
    goal_key: String,
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

    async fn observe(
        &self,
        goal: &str,
        enhanced_goal_key: Option<String>,
    ) -> Result<ActionSpace, String> {
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
        let persistent_locators = candidates
            .iter()
            .filter_map(|candidate| {
                self.candidates
                    .semantic_locator(candidate)
                    .map(|locator| (candidate.id.clone(), locator))
            })
            .collect();
        let space = ActionSpace {
            token: format!("obs:{}", Uuid::new_v4().simple()),
            created_at: Instant::now(),
            observation,
            candidates,
            persistent_locators,
            enhanced_goal_key,
        };
        *self.last_space.lock().await = Some(space.clone());
        Ok(space)
    }

    async fn clear_space(&self) {
        *self.last_space.lock().await = None;
    }

    async fn begin_enhanced_step(&self, goal: &str) -> Result<(String, Vec<RecentAction>), String> {
        let goal_key = normalize_goal_key(goal);
        let mut state = self.loop_state.lock().await;
        if state.goal_key.is_empty() || !same_bounded_goal(&state.goal_key, &goal_key) {
            *state = EnhancedLoopState {
                goal_key: goal_key.clone(),
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
        // Count every enhanced decision attempt, including Jev protocol/network
        // failures and attempts that hand the same action space back to the
        // primary model. Delayed local execution records its outcome below but
        // must not consume a second decision step.
        state.steps = state.steps.saturating_add(1);
        Ok((state.goal_key.clone(), state.recent_actions.clone()))
    }

    async fn record_enhanced_result(
        &self,
        goal_key: &str,
        candidate: &ActionCandidate,
        verification: Verification,
    ) {
        let mut state = self.loop_state.lock().await;
        if state.goal_key != goal_key {
            return;
        }
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

fn normalize_goal_key(goal: &str) -> String {
    let mut normalized = String::new();
    let mut pending_separator = false;
    for ch in goal.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(ch);
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }
    if normalized.is_empty() {
        "__nonsemantic_goal__".into()
    } else {
        normalized
    }
}

fn same_bounded_goal(existing: &str, incoming: &str) -> bool {
    if existing == incoming {
        return true;
    }
    if existing.is_empty() || incoming.is_empty() {
        return false;
    }

    let existing_chars: Vec<char> = existing.chars().collect();
    let incoming_chars: Vec<char> = incoming.chars().collect();
    let max_len = existing_chars.len().max(incoming_chars.len());
    let min_len = existing_chars.len().min(incoming_chars.len());
    if (existing.contains(incoming) || incoming.contains(existing))
        && min_len.saturating_mul(4) >= max_len.saturating_mul(3)
    {
        return true;
    }

    let allowed_edits = 3_usize.max(max_len / 10);
    bounded_edit_distance(&existing_chars, &incoming_chars, allowed_edits)
        .is_some_and(|distance| distance <= allowed_edits)
}

fn bounded_edit_distance(left: &[char], right: &[char], limit: usize) -> Option<usize> {
    if left.len().abs_diff(right.len()) > limit {
        return None;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_char) in left.iter().enumerate() {
        current[0] = left_index + 1;
        let mut row_min = current[0];
        for (right_index, right_char) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + usize::from(left_char != right_char));
            row_min = row_min.min(current[right_index + 1]);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    Some(previous[right.len()])
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
                    .observe(goal, None)
                    .await
                    .map(|space| action_space_json(&space))
            }
            "desktop_semantic_execute" => {
                let observation_token = required_string(params, "observation_token")?;
                let candidate_id = required_string(params, "candidate_id")?;
                let input = execution_input(params)?;
                execute_cached_candidate(&backend, observation_token, candidate_id, input).await
            }
            "desktop_semantic_action" => {
                let locator = params
                    .get("locator")
                    .cloned()
                    .ok_or_else(|| "locator 不能为空".to_string())
                    .and_then(|value| {
                        serde_json::from_value::<SemanticLocator>(value)
                            .map_err(|error| format!("locator 格式无效: {error}"))
                    })?;
                let action = params
                    .get("action")
                    .cloned()
                    .ok_or_else(|| "action 不能为空".to_string())
                    .and_then(|value| {
                        serde_json::from_value::<NativeAction>(value)
                            .map_err(|error| format!("action 格式无效: {error}"))
                    })?;
                let input = execution_input(params)?;
                execute_persistent_action(&backend, locator, action, input).await
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
    let result = execute_candidate(backend, &space.observation, &candidate, &input, None).await?;
    if let (Some(goal_key), Some(verification)) = (
        space.enhanced_goal_key.as_deref(),
        result
            .get("verification")
            .and_then(|value| serde_json::from_value::<Verification>(value.clone()).ok()),
    ) {
        backend
            .record_enhanced_result(goal_key, &candidate, verification)
            .await;
    }
    Ok(result)
}

async fn execute_jev_step(
    backend: &SemanticDesktopBackend,
    goal: &str,
) -> Result<serde_json::Value, String> {
    let (goal_key, recent_actions) = backend.begin_enhanced_step(goal).await?;
    let space = backend.observe(goal, Some(goal_key.clone())).await?;
    let mut config = crate::config::load_registry()
        .map_err(|error| format!("读取 Jev 配置失败: {error}"))?
        .jev;
    if config.api_key.trim().is_empty() {
        return Err("Jev API Key 未配置".into());
    }
    let fallback_to_primary_model = config.fallback_to_primary_model;
    let confidence_floor = config.confidence_floor.clamp(0.0, 1.0);
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
    if fallback_to_primary_model && decision.confidence.unwrap_or(1.0) < confidence_floor {
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
    if matches!(candidate.kind, CandidateKind::Done) {
        backend.clear_space().await;
        backend.finish_enhanced_goal().await;
        return Ok(json!({
            "status": "needs_primary_completion_check",
            "reason": "Jev 只提出任务可能已完成；请由当前主模型结合业务目标确认是否结束，不会把 Jev 的 Done 直接当作本地完成事实",
            "candidate_id": candidate.id,
            "decision": {
                "provider": "jev",
                "model": decision.actual_model,
                "confidence": decision.confidence,
                "usage": decision.usage,
            },
        }));
    }
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
            .record_enhanced_result(&goal_key, &candidate, verification)
            .await;
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
                "status": "needs_primary_completion_check",
                "reason": "完成候选不是本地可验证的通用后置条件；请由当前主模型结合业务目标确认是否结束",
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

async fn execute_persistent_action(
    backend: &SemanticDesktopBackend,
    locator: SemanticLocator,
    action: NativeAction,
    input: ExecutionInput,
) -> Result<serde_json::Value, String> {
    if locator.app_id.trim().is_empty() {
        return Err("locator.app_id 不能为空".into());
    }
    if locator.role.is_none()
        && locator.automation_id.as_deref().is_none_or(str::is_empty)
        && locator.accessible_name.as_deref().is_none_or(str::is_empty)
    {
        return Err("locator 至少需要 role、automation_id 或 accessible_name 之一".into());
    }
    let before = backend
        .observer
        .observe(&ObservationScope {
            app_id: Some(locator.app_id.clone()),
            window_id: locator.window_id.clone(),
            subtree_id: None,
        })
        .await
        .map_err(|error| error.to_string())?;
    let candidate = backend
        .candidates
        .rebuild_semantic_candidate(&locator, action, &before)
        .map_err(|error| error.to_string())?;
    let result = execute_candidate(backend, &before, &candidate, &input, None).await?;
    let verification = result
        .get("verification")
        .cloned()
        .and_then(|value| serde_json::from_value::<Verification>(value).ok())
        .unwrap_or(Verification::Unknown);
    if verification != Verification::Achieved {
        return Err(format!(
            "已保存的语义动作没有通过执行后验证: {verification:?}"
        ));
    }
    Ok(result)
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
    let target_before = candidate
        .target
        .as_deref()
        .and_then(|id| before.nodes.iter().find(|node| node.opaque_id == id));
    let target_after = target_before.map(|target| resolve_target(after, target));
    let achieved = match (&candidate.kind, target_before, target_after) {
        (CandidateKind::Invoke, Some(old), Some(TargetResolution::Unique(new))) => {
            invoke_expected(candidate)
                && (old.enabled != new.enabled
                    || old.visible != new.visible
                    || old.focused != new.focused
                    || old.toggled != new.toggled
                    || old.selected != new.selected
                    || old.expanded != new.expanded
                    || old.value_fingerprint != new.value_fingerprint)
        }
        (CandidateKind::Invoke, Some(old), Some(TargetResolution::Missing)) => {
            invoke_expected(candidate)
                && target_is_unique(before, old)
                && before.app.id == after.app.id
                && before.window.id == after.window.id
        }
        (CandidateKind::Invoke, Some(old), _)
            if invoke_expected(candidate)
                && target_is_unique(before, old)
                && before.app.id == after.app.id
                && before.window.id != after.window.id =>
        {
            true
        }
        (CandidateKind::Toggle, Some(old), Some(TargetResolution::Unique(new))) => {
            old.toggled.zip(new.toggled).is_some_and(|(a, b)| a != b)
        }
        (CandidateKind::Select, _, Some(TargetResolution::Unique(new))) => {
            new.selected == Some(true)
        }
        (CandidateKind::Expand, _, Some(TargetResolution::Unique(new))) => {
            new.expanded == Some(true)
        }
        (CandidateKind::Collapse, _, Some(TargetResolution::Unique(new))) => {
            new.expanded == Some(false)
        }
        (CandidateKind::Focus, _, Some(TargetResolution::Unique(new))) => new.focused,
        (CandidateKind::SetValue { .. }, Some(old), Some(TargetResolution::Unique(new))) => {
            old.value_fingerprint != new.value_fingerprint
        }
        _ => false,
    };
    if achieved {
        Verification::Achieved
    } else {
        // A clock, notification badge, spinner or unrelated control may alter
        // the window fingerprint. That is not evidence that this target's
        // action succeeded and must never authorize replay continuation.
        Verification::NoChange
    }
}

#[derive(Debug, Clone, Copy)]
enum TargetResolution<'a> {
    Unique(&'a crate::desktop_automation::UiNode),
    Missing,
    Ambiguous,
}

fn resolve_target<'a>(
    observation: &'a Observation,
    target: &crate::desktop_automation::UiNode,
) -> TargetResolution<'a> {
    let matches: Vec<_> = observation
        .nodes
        .iter()
        .filter(|node| same_public_semantics(node, target))
        .collect();
    match matches.as_slice() {
        [unique] => TargetResolution::Unique(unique),
        [] => TargetResolution::Missing,
        _ => TargetResolution::Ambiguous,
    }
}

fn target_is_unique(observation: &Observation, target: &crate::desktop_automation::UiNode) -> bool {
    observation
        .nodes
        .iter()
        .filter(|node| same_public_semantics(node, target))
        .take(2)
        .count()
        == 1
}

fn same_public_semantics(
    left: &crate::desktop_automation::UiNode,
    right: &crate::desktop_automation::UiNode,
) -> bool {
    let identity_matches = match (
        uia_semantic_key(&left.opaque_id),
        uia_semantic_key(&right.opaque_id),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    };
    identity_matches
        && left.role == right.role
        && left.name == right.name
        && left.secure == right.secure
}

fn uia_semantic_key(opaque_id: &str) -> Option<&str> {
    let suffix = opaque_id.strip_prefix("uie:")?;
    let (semantic_hash, _) = suffix.rsplit_once(':')?;
    (!semantic_hash.is_empty()).then_some(semantic_hash)
}

fn invoke_expected(candidate: &ActionCandidate) -> bool {
    candidate
        .expected_effects
        .iter()
        .any(|predicate| predicate.name == "invoke_target_changed_or_disappeared")
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
        "candidates": space.candidates.iter().map(|candidate| {
            let workflow_step = space.persistent_locators.get(&candidate.id).and_then(|locator| {
                locator.supported_action.as_ref().map(|action| json!({
                    "tool": "desktop_semantic_action",
                    "params": {
                        "locator": locator,
                        "action": action,
                    },
                }))
            });
            json!({
                "id": candidate.id,
                "kind": candidate.kind,
                "description": candidate.public_description,
                "risk": candidate.local_risk,
                "target": candidate.target,
                "workflow_step": workflow_step,
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_automation::Predicate;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct FakeAdapter {
        executions: AtomicUsize,
        candidate_kind: CandidateKind,
        received_value: Mutex<Option<String>>,
        changes_after_execute: bool,
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
            let mut current = observation();
            let executed = self.changes_after_execute && self.executions.load(Ordering::SeqCst) > 0;
            if executed {
                current.fingerprint = "changed-ui".into();
                current.revision = 2;
            }
            if matches!(self.candidate_kind, CandidateKind::SetValue { .. }) {
                current.nodes.push(crate::desktop_automation::UiNode {
                    opaque_id: "fake-target".into(),
                    role: crate::desktop_automation::UiRole::TextField,
                    name: Some("Fake input".into()),
                    short_value: None,
                    enabled: true,
                    visible: true,
                    focused: false,
                    secure: false,
                    toggled: None,
                    selected: None,
                    expanded: None,
                    value_fingerprint: Some(if executed {
                        "value:after".into()
                    } else {
                        "value:before".into()
                    }),
                    supported_actions: vec![NativeAction::SetValue],
                });
            } else if matches!(self.candidate_kind, CandidateKind::Invoke) && !executed {
                current.nodes.push(crate::desktop_automation::UiNode {
                    opaque_id: "fake-target".into(),
                    role: crate::desktop_automation::UiRole::Button,
                    name: Some("Fake Button".into()),
                    short_value: None,
                    enabled: true,
                    visible: true,
                    focused: false,
                    secure: false,
                    toggled: None,
                    selected: None,
                    expanded: None,
                    value_fingerprint: None,
                    supported_actions: vec![NativeAction::Invoke],
                });
            }
            Ok(current)
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
                expected_effects: match self.candidate_kind {
                    CandidateKind::Invoke => vec![Predicate {
                        name: "invoke_target_changed_or_disappeared".into(),
                        arguments: Default::default(),
                    }],
                    _ => vec![],
                },
            }])
        }

        fn semantic_locator(&self, candidate: &ActionCandidate) -> Option<SemanticLocator> {
            Some(SemanticLocator {
                app_id: "fake-app".into(),
                window_id: Some("fake-window".into()),
                window_title: Some("Fake Window".into()),
                role: Some(crate::desktop_automation::UiRole::Button),
                automation_id: Some("fake-button".into()),
                accessible_name: Some("Fake Button".into()),
                ancestor_chain: vec![],
                supported_action: Some(match candidate.kind {
                    CandidateKind::SetValue { .. } => NativeAction::SetValue,
                    _ => NativeAction::Invoke,
                }),
                ordinal_hint: Some(0),
            })
        }

        fn rebuild_semantic_candidate(
            &self,
            locator: &SemanticLocator,
            action: NativeAction,
            observation: &Observation,
        ) -> Result<ActionCandidate, crate::desktop_automation::AutomationError> {
            if locator.app_id != observation.app.id {
                return Err(crate::desktop_automation::AutomationError::Candidates(
                    "wrong app".into(),
                ));
            }
            Ok(ActionCandidate {
                id: "rebuilt-candidate".into(),
                observation_revision: observation.revision,
                target: Some("fake-target".into()),
                kind: match action {
                    NativeAction::SetValue => CandidateKind::SetValue {
                        slot_id: "value".into(),
                    },
                    _ => CandidateKind::Invoke,
                },
                public_description: "Rebuilt fake action".into(),
                local_risk: crate::desktop_automation::RiskClass::Reversible,
                preconditions: vec![],
                expected_effects: match action {
                    NativeAction::Invoke => vec![Predicate {
                        name: "invoke_target_changed_or_disappeared".into(),
                        arguments: Default::default(),
                    }],
                    _ => vec![],
                },
            })
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
            changes_after_execute: true,
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

    #[tokio::test]
    async fn semantic_observe_exposes_replayable_workflow_step() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter);

        let observed = registry
            .execute_semantic_desktop_tool("desktop_semantic_observe", &json!({ "goal": "open" }))
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(observed.output.as_deref().unwrap()).unwrap();
        let step = &payload["candidates"][0]["workflow_step"];

        assert_eq!(step["tool"], "desktop_semantic_action");
        assert_eq!(step["params"]["locator"]["app_id"], "fake-app");
        assert_eq!(step["params"]["action"], "invoke");
        assert!(step.to_string().find("candidate_id").is_none());
        assert!(step.to_string().find("observation_token").is_none());
    }

    #[tokio::test]
    async fn persistent_semantic_action_rebuilds_without_observation_token() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let executed = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_action",
                &json!({
                    "locator": {
                        "app_id": "fake-app",
                        "window_title": "Fake Window",
                        "role": "button",
                        "automation_id": "fake-button",
                        "accessible_name": "Fake Button",
                        "supported_action": "invoke",
                        "ordinal_hint": 0
                    },
                    "action": "invoke"
                }),
            )
            .await
            .unwrap();

        assert!(executed.success, "{:?}", executed.error);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn persistent_set_value_passes_text_only_to_local_executor() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::SetValue {
                slot_id: "value".into(),
            },
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let executed = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_action",
                &json!({
                    "locator": {
                        "app_id": "fake-app",
                        "window_id": "fake-window",
                        "role": "text_field",
                        "automation_id": "search",
                        "supported_action": "set_value"
                    },
                    "action": "set_value",
                    "value": "  workflow value  "
                }),
            )
            .await
            .unwrap();

        assert!(executed.success, "{:?}", executed.error);
        assert_eq!(
            adapter.received_value.lock().unwrap().as_deref(),
            Some("  workflow value  ")
        );
        assert!(!executed
            .output
            .as_deref()
            .unwrap_or_default()
            .contains("workflow value"));
    }

    #[tokio::test]
    async fn persistent_state_action_requires_target_specific_verification() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Toggle,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let rejected = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_action",
                &json!({
                    "locator": {
                        "app_id": "fake-app",
                        "window_id": "fake-window",
                        "role": "check_box",
                        "accessible_name": "Fake checkbox",
                        "supported_action": "toggle"
                    },
                    "action": "toggle"
                }),
            )
            .await
            .unwrap();

        assert!(!rejected.success);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
        assert!(rejected
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("没有通过执行后验证"));
    }

    #[tokio::test]
    async fn persistent_semantic_action_rejects_scope_mismatch() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let rejected = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_action",
                &json!({
                    "locator": {
                        "app_id": "another-app",
                        "role": "button",
                        "accessible_name": "Fake Button",
                        "supported_action": "invoke"
                    },
                    "action": "invoke"
                }),
            )
            .await
            .unwrap();

        assert!(!rejected.success);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);
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

    fn invoke_observation(fingerprint: &str, target_count: usize) -> Observation {
        let mut observation = observation();
        observation.fingerprint = fingerprint.into();
        observation.nodes = (0..target_count)
            .map(|index| crate::desktop_automation::UiNode {
                opaque_id: format!("invoke-target-{index}"),
                role: crate::desktop_automation::UiRole::Button,
                name: Some("Open".into()),
                short_value: None,
                enabled: true,
                visible: true,
                focused: false,
                secure: false,
                toggled: None,
                selected: None,
                expanded: None,
                value_fingerprint: None,
                supported_actions: vec![crate::desktop_automation::NativeAction::Invoke],
            })
            .collect();
        observation
    }

    fn invoke_candidate(target: &str) -> ActionCandidate {
        ActionCandidate {
            id: "invoke".into(),
            observation_revision: 1,
            target: Some(target.into()),
            kind: CandidateKind::Invoke,
            public_description: "Invoke Open".into(),
            local_risk: crate::desktop_automation::RiskClass::Reversible,
            preconditions: vec![],
            expected_effects: vec![Predicate {
                name: "invoke_target_changed_or_disappeared".into(),
                arguments: Default::default(),
            }],
        }
    }

    #[tokio::test]
    async fn semantic_set_value_stays_local_and_preserves_whitespace() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::SetValue {
                slot_id: "value".into(),
            },
            received_value: Mutex::new(None),
            changes_after_execute: true,
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
    async fn done_candidate_requires_primary_model_completion_check() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Done,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let mut registry = ToolRegistry::new();
        registry.set_semantic_desktop_adapter(adapter.clone());

        let observed = registry
            .execute_semantic_desktop_tool("desktop_semantic_observe", &json!({ "goal": "done" }))
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(observed.output.as_deref().unwrap()).unwrap();
        let result = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_execute",
                &json!({
                    "observation_token": payload["observation_token"],
                    "candidate_id": "fake-candidate"
                }),
            )
            .await
            .unwrap();
        let output: serde_json::Value =
            serde_json::from_str(result.output.as_deref().unwrap()).unwrap();

        assert_eq!(output["status"], "needs_primary_completion_check");
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn jev_step_is_rejected_when_enhanced_mode_is_off() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
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

    #[tokio::test]
    async fn enhanced_budget_counts_every_decision_attempt_across_minor_goal_edits() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let backend = SemanticDesktopBackend::new(adapter);

        for index in 0..HARD_STEP_LIMIT {
            let goal = if index % 2 == 0 {
                "Open the report"
            } else {
                "  OPEN   the report!!!  "
            };
            backend.begin_enhanced_step(goal).await.unwrap();
        }

        let rejected = backend
            .begin_enhanced_step("open the report.")
            .await
            .unwrap_err();
        assert!(rejected.contains("100 步硬上限"));
        let state = backend.loop_state.lock().await;
        assert_eq!(state.steps, HARD_STEP_LIMIT);
        assert_eq!(state.goal_key, "open the report");
    }

    #[tokio::test]
    async fn enhanced_fallback_execution_records_stall_and_recent_action_without_double_counting() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::SetValue {
                slot_id: "value".into(),
            },
            received_value: Mutex::new(None),
            changes_after_execute: false,
        });
        let backend = SemanticDesktopBackend::new(adapter);
        for _ in 0..STALL_LIMIT {
            let (goal_key, _) = backend.begin_enhanced_step("Fill field").await.unwrap();
            let space = backend.observe("Fill field", Some(goal_key)).await.unwrap();
            let result = execute_cached_candidate(
                &backend,
                &space.token,
                "fake-candidate",
                ExecutionInput {
                    value: Some("local text".into()),
                },
            )
            .await
            .unwrap();
            assert_eq!(result["verification"], "no_change");
        }

        let state = backend.loop_state.lock().await;
        assert_eq!(
            state.steps, STALL_LIMIT,
            "delayed execution is not a new decision"
        );
        assert_eq!(state.consecutive_stalls, STALL_LIMIT);
        assert_eq!(state.recent_actions.len(), STALL_LIMIT as usize);
        assert_eq!(state.recent_actions[0].verification, Verification::NoChange);
        drop(state);

        let rejected = backend
            .begin_enhanced_step(" fill field! ")
            .await
            .unwrap_err();
        assert!(rejected.contains("未产生界面变化"));
    }

    #[tokio::test]
    async fn enhanced_primary_fallback_execution_records_recent_action() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        });
        let backend = SemanticDesktopBackend::new(adapter);
        let (goal_key, _) = backend.begin_enhanced_step("Open report").await.unwrap();
        let space = backend
            .observe("Open report", Some(goal_key))
            .await
            .unwrap();

        let result = execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default(),
        )
        .await
        .unwrap();

        assert_eq!(result["verification"], "no_change");
        let state = backend.loop_state.lock().await;
        assert_eq!(state.steps, 1);
        assert_eq!(state.consecutive_stalls, 1);
        assert_eq!(state.recent_actions.len(), 1);
        assert_eq!(state.recent_actions[0].verification, Verification::NoChange);
    }

    #[tokio::test]
    async fn ordinary_semantic_execution_does_not_join_enhanced_budget() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let backend = SemanticDesktopBackend::new(adapter);
        let space = backend.observe("Open", None).await.unwrap();

        execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default(),
        )
        .await
        .unwrap();

        let state = backend.loop_state.lock().await;
        assert_eq!(state.steps, 0);
        assert_eq!(state.consecutive_stalls, 0);
        assert!(state.recent_actions.is_empty());
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

    #[test]
    fn unrelated_fingerprint_change_does_not_verify_invoke() {
        let before = invoke_observation("before", 1);
        let after = invoke_observation("unrelated-change", 1);
        let candidate = invoke_candidate("invoke-target-0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::NoChange
        );
    }

    #[test]
    fn unique_invoke_target_disappearance_is_verified() {
        let before = invoke_observation("before", 1);
        let after = invoke_observation("after", 0);
        let candidate = invoke_candidate("invoke-target-0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Achieved
        );
    }

    #[test]
    fn duplicate_invoke_targets_cannot_be_verified_by_disappearance() {
        let before = invoke_observation("before", 2);
        let after = invoke_observation("after", 0);
        let candidate = invoke_candidate("invoke-target-0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::NoChange
        );
    }

    #[test]
    fn unrelated_application_switch_does_not_verify_invoke() {
        let before = invoke_observation("before", 1);
        let mut after = invoke_observation("after", 0);
        after.app.id = "another-app".into();
        after.window.id = "another-window".into();
        let candidate = invoke_candidate("invoke-target-0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::NoChange
        );
    }

    #[test]
    fn uia_row_semantic_key_verifies_the_intended_repeated_control() {
        let mut before = invoke_observation("before", 2);
        before.nodes[0].opaque_id = "uie:row-a-open:0".into();
        before.nodes[1].opaque_id = "uie:row-b-open:1".into();
        let mut after = before.clone();
        after.fingerprint = "after".into();
        // Row A disappeared while Row B remained and moved to index 0.
        after.nodes.remove(0);
        after.nodes[0].opaque_id = "uie:row-b-open:0".into();
        let candidate = invoke_candidate("uie:row-a-open:0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Achieved
        );
    }
}
