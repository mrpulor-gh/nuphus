//! Accessibility/UIA-first desktop tools.
//!
//! The model can only observe an opaque candidate set and select a candidate
//! id. Native handles, coordinates and platform locators stay in the adapter.

use super::desktop_verification::{
    CompletionPolicy, DesktopExpectation, StateCondition, StateStatus,
};
use super::registry::ToolRegistry;
use crate::desktop_automation::{
    ActionCandidate, ActionClass, CandidateBuilder, CandidateKind, ComputerExecutor,
    ComputerObserver, DecisionInput, DecisionProvider, ExecutionGrant, ExecutionInput, JevClient,
    LayaClient, LayaConfig, LocalPolicy, NativeAction, Observation, ObservationScope, Policy,
    PolicyDecision, RecentAction, SemanticLocator, Verification,
};
use crate::desktop_automation::{ActionEffect, DeliveryMode, DesktopActionError, DispatchState};
use crate::ToolResult;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const ACTION_SPACE_TTL: Duration = Duration::from_secs(120);
const STALL_LIMIT: u32 = 3;
const CANDIDATE_PAGE_SIZE: usize = 20;
const CANDIDATE_PAGE_CHARS: usize = 7_000;
const PRIMARY_HANDOFF_ID: &str = "control:primary-decision";

#[derive(Clone)]
pub(super) struct SemanticDesktopBackend {
    observer: Arc<dyn ComputerObserver>,
    candidates: Arc<dyn CandidateBuilder>,
    executor: Arc<dyn ComputerExecutor>,
    last_space: Arc<tokio::sync::Mutex<Option<ActionSpace>>>,
    loop_state: Arc<tokio::sync::Mutex<EnhancedLoopState>>,
    approval_signals: Option<crate::state::SharedSignals>,
}

#[derive(Clone)]
struct ActionSpace {
    token: String,
    created_at: Instant,
    observation: Observation,
    candidates: Vec<ActionCandidate>,
    persistent_locators: std::collections::HashMap<String, SemanticLocator>,
    enhanced_goal_key: Option<String>,
    owner: Option<String>,
    scope: ObservationScope,
    launch_ref: Option<String>,
}

#[derive(Clone, Default)]
struct EnhancedLoopState {
    owner: Option<String>,
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
            approval_signals: None,
        }
    }

    #[cfg(test)]
    async fn observe(
        &self,
        goal: &str,
        enhanced_goal_key: Option<String>,
    ) -> Result<ActionSpace, String> {
        self.observe_scoped(goal, enhanced_goal_key, ObservationScope::default(), None)
            .await
    }

    async fn observe_scoped(
        &self,
        goal: &str,
        enhanced_goal_key: Option<String>,
        scope: ObservationScope,
        launch_ref: Option<String>,
    ) -> Result<ActionSpace, String> {
        let observation = self
            .observer
            .observe(&scope)
            .await
            .map_err(|error| error.to_string())?;
        self.store_observation(goal, enhanced_goal_key, scope, launch_ref, observation)
            .await
    }

    async fn store_observation(
        &self,
        goal: &str,
        enhanced_goal_key: Option<String>,
        scope: ObservationScope,
        launch_ref: Option<String>,
        observation: Observation,
    ) -> Result<ActionSpace, String> {
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
            owner: crate::automation_gate::current_execution_owner(),
            scope,
            launch_ref,
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
        let owner = crate::automation_gate::current_execution_owner();
        if state.goal_key.is_empty()
            || state.owner != owner
            || (owner.is_none() && !same_bounded_goal(&state.goal_key, &goal_key))
        {
            *state = EnhancedLoopState {
                goal_key: goal_key.clone(),
                owner,
                ..Default::default()
            };
        }
        // The owning agent already supplies iteration, cancellation and timeout
        // bounds. A single-step decision tool must not permanently lock that run.
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
            let repeated = state.recent_actions.last().is_some_and(|previous| {
                previous.target_summary == candidate.public_description
                    && previous.action_class == candidate.action_class()
                    && previous.verification == Verification::NoChange
            });
            state.consecutive_stalls = if repeated {
                state.consecutive_stalls.saturating_add(1)
            } else {
                1
            };
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
    pub(super) async fn semantic_input_window(
        &self,
        params: &serde_json::Value,
    ) -> Result<i32, String> {
        if params.get("hwnd").is_some() {
            return Err("target_locator 与 hwnd 不能同时指定".into());
        }
        let locator: SemanticLocator = serde_json::from_value(params["target_locator"].clone())
            .map_err(|_| "target_locator 必须是成功语义动作返回的稳定定位器")?;
        let backend = self
            .semantic_desktop
            .as_ref()
            .ok_or("当前平台没有语义目标适配器")?;
        self.target_service()?
            .ensure_saved(&locator, params.get("launch_ref").and_then(|v| v.as_str()))
            .await?;
        let client = self.desktop_client().ok_or("本地桌面服务尚未连接")?;
        let current_hwnd = || async {
            let value = client.foreground_hwnd().await.map_err(|e| e.to_string())?;
            value["result"]["hwnd"]
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .filter(|v| *v != 0)
                .ok_or_else(|| "前台窗口不可用".to_string())
        };
        let hwnd = current_hwnd().await?;
        // Validate the stable app/window/element through the native adapter.
        // No guessed handle or model-generated selector enters the input call.
        backend
            .observer
            .observe_locator(&locator)
            .await
            .map_err(|e| e.to_string())?;
        if current_hwnd().await? != hwnd {
            return Err("目标窗口在定位期间变化，请重新观察；尚未发送键盘事件".into());
        }
        backend.clear_space().await;
        client.invalidate_captures().map_err(|e| e.to_string())?;
        Ok(hwnd)
    }

    async fn semantic_scope(
        &self,
        params: &serde_json::Value,
    ) -> Result<(ObservationScope, Option<String>), String> {
        let mut scope = ObservationScope {
            delivery: delivery_mode(params)?,
            ..Default::default()
        };
        let mut launch_ref = None;
        if let Some(token) = params.get("target_token").and_then(|v| v.as_str()) {
            (scope, launch_ref) = self
                .target_service()?
                .bound_scope(token, delivery_mode(params)?)
                .await?;
        }
        scope.subtree_id = params
            .get("subtree_id")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        if scope.subtree_id.is_none() {
            scope.subtree_id = match params.get("scope").and_then(|v| v.as_str()) {
                None | Some("window") => None,
                Some("menu") => Some("@menu".into()),
                Some(_) => return Err("invalid_scope: scope 必须是 window 或 menu".into()),
            };
        }
        Ok((scope, launch_ref))
    }

    pub(super) async fn execute_semantic_desktop_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<ToolResult, String> {
        let Some(mut backend) = self.semantic_desktop.clone() else {
            return Ok(ToolResult::failure(
                "当前平台尚未提供 Accessibility/UIA 语义桌面适配器",
            ));
        };
        backend.approval_signals = Some(self.signals().clone());
        if matches!(
            tool_name,
            "desktop_target_bind"
                | "desktop_semantic_execute"
                | "desktop_semantic_action"
                | "desktop_agent_step"
        ) {
            if let Some(client) = self.desktop_client() {
                client.invalidate_captures().map_err(|e| e.to_string())?;
            }
        }
        let result = match tool_name {
            "desktop_targets_list" => {
                self.target_service()?
                    .list(
                        params.get("query").and_then(|v| v.as_str()),
                        params.get("cursor").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                    )
                    .await
            }
            "desktop_target_bind" => {
                let result = self
                    .target_service()?
                    .bind_with_delivery(
                        required_string(params, "app_ref")?,
                        params.get("window_ref").and_then(|v| v.as_str()),
                        delivery_mode(params)?,
                    )
                    .await;
                backend.clear_space().await;
                result
            }
            "desktop_semantic_observe" => {
                if let Some(cursor) = params.get("tree_cursor").and_then(|v| v.as_str()) {
                    let space = backend
                        .last_space
                        .lock()
                        .await
                        .clone()
                        .ok_or("stale_observation: 请重新观察")?;
                    validate_space_token(&space, &space.token)?;
                    if next_tree_cursor(&space).as_deref() != Some(cursor) {
                        return Err("invalid_tree_cursor: 请使用最近一次观察返回的续读令牌".into());
                    }
                    let mut scope = space.scope.clone();
                    scope.tree_offset = scope
                        .tree_offset
                        .saturating_add(space.observation.nodes.len());
                    let next = backend
                        .observe_scoped(
                            params.get("goal").and_then(|v| v.as_str()).unwrap_or(""),
                            None,
                            scope,
                            space.launch_ref,
                        )
                        .await?;
                    return Ok(ToolResult::success(action_space_json(&next).to_string()));
                }
                if let Some(token) = params.get("observation_token").and_then(|v| v.as_str()) {
                    let space = backend
                        .last_space
                        .lock()
                        .await
                        .clone()
                        .ok_or("stale_observation: 没有可继续查询的观察，请重新观察")?;
                    validate_space_token(&space, token)?;
                    let cursor =
                        params.get("cursor").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let region_view =
                        params.get("view").and_then(|v| v.as_str()) == Some("regions");
                    if cursor
                        > if region_view {
                            space.observation.nodes.len()
                        } else {
                            space.candidates.len()
                        }
                    {
                        return Ok(ToolResult::failure("invalid_cursor: 候选分页位置无效"));
                    }
                    let page = if region_view {
                        region_page(&space, cursor)
                    } else {
                        action_space_page(&space, cursor)
                    };
                    return Ok(ToolResult::success(page.to_string()));
                }
                let goal = params
                    .get("goal")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let (scope, launch_ref) = self.semantic_scope(params).await?;
                backend
                    .observe_scoped(goal, None, scope, launch_ref)
                    .await
                    .map(|space| action_space_json(&space))
            }
            "desktop_semantic_candidate" => {
                let token = required_string(params, "observation_token")?;
                let id = required_string(params, "candidate_id")?;
                let space = backend
                    .last_space
                    .lock()
                    .await
                    .clone()
                    .ok_or("stale_observation: 请重新观察")?;
                validate_space_token(&space, token)?;
                let candidate = space
                    .candidates
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or("candidate_id 不属于本次观察")?;
                Ok(json!({"observation_token": token, "candidate": candidate,
                    "workflow_step": workflow_step(&space, candidate)}))
            }
            "desktop_semantic_execute" => {
                let observation_token = required_string(params, "observation_token")?;
                let candidate_id = required_string(params, "candidate_id")?;
                let mut input = execution_input(params)?;
                if params.get("delivery_mode").is_none() {
                    if let Some(space) = backend.last_space.lock().await.as_ref() {
                        input.delivery = space.scope.delivery;
                    }
                }
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
                let options = ActionOptions::parse(params)?;
                let scope = if let Ok(targets) = self.target_service() {
                    targets
                        .ensure_saved_scope(
                            &locator,
                            params.get("launch_ref").and_then(|v| v.as_str()),
                            input.delivery,
                        )
                        .await?
                } else {
                    ObservationScope::default()
                };
                execute_persistent_action_with_options(
                    &backend,
                    locator,
                    action,
                    input,
                    scope,
                    options,
                    self.target_service().ok(),
                )
                .await
            }
            "desktop_verify_state" => {
                let expectation: DesktopExpectation = serde_json::from_value(
                    params
                        .get("expectation")
                        .cloned()
                        .unwrap_or_else(|| params.clone()),
                )
                .map_err(|e| format!("expectation 无效: {e}"))?;
                expectation.validate()?;
                let scope = ObservationScope {
                    app_id: Some(expectation.locator.app_id.clone()),
                    delivery: DeliveryMode::Auto,
                    ..Default::default()
                };
                backend.clear_space().await;
                verify_expected_state(
                    &backend,
                    &expectation,
                    &scope,
                    self.target_service().ok().as_deref(),
                )
                .await
            }
            "desktop_agent_step" => {
                if !self.enhanced_mode {
                    Err(
                        "增强模式未启用；请使用 desktop_semantic_observe/desktop_semantic_execute"
                            .into(),
                    )
                } else {
                    let goal = required_string(params, "goal")?;
                    let (scope, launch_ref) = self.semantic_scope(params).await?;
                    execute_jev_step(&backend, goal, scope, launch_ref).await
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
    let mut input = ExecutionInput {
        delivery: delivery_mode(params)?,
        checked: params
            .get("checked")
            .map(|v| v.as_bool().ok_or("checked 必须为布尔值"))
            .transpose()?,
        direction: params
            .get("direction")
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| "direction 无效"))
            .transpose()?,
        amount: params
            .get("amount")
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| "amount 必须为 small/page"))
            .transpose()?,
        ..Default::default()
    };
    let Some(value) = params.get("value") else {
        return Ok(input);
    };
    if value.is_null() {
        return Ok(input);
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
    input.value = Some(value.to_string());
    Ok(input)
}

fn delivery_mode(params: &serde_json::Value) -> Result<DeliveryMode, String> {
    params
        .get("delivery_mode")
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|_| "delivery_mode 必须为 foreground/auto/background".into())
        })
        .unwrap_or(Ok(DeliveryMode::Foreground))
}

#[derive(Default)]
struct ActionOptions {
    completion: CompletionPolicy,
    expectation: Option<DesktopExpectation>,
}
impl ActionOptions {
    fn parse(params: &serde_json::Value) -> Result<Self, String> {
        let completion = params
            .get("completion_policy")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("completion_policy 无效: {e}"))?
            .unwrap_or_default();
        let expectation: Option<DesktopExpectation> = params
            .get("expectation")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("expectation 无效: {e}"))?;
        if let Some(expected) = &expectation {
            expected.validate()?;
        }
        Ok(Self {
            completion,
            expectation,
        })
    }
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
    let mut result =
        execute_candidate(backend, &space.observation, &candidate, &input, None).await?;
    result["workflow_step"] = workflow_step(&space, &candidate);
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
    scope: ObservationScope,
    launch_ref: Option<String>,
) -> Result<serde_json::Value, String> {
    let (goal_key, recent_actions) = backend.begin_enhanced_step(goal).await?;
    let space = backend
        .observe_scoped(goal, Some(goal_key.clone()), scope, launch_ref)
        .await?;
    if backend.loop_state.lock().await.consecutive_stalls >= STALL_LIMIT {
        return Ok(json!({"status":"needs_primary_decision",
            "reason":"同一语义动作连续无进展；请检查新观察或换用其他能力，取得进展后可继续增强判断",
            "action_space":action_space_json(&space)}));
    }
    // Jev and Laya are separate models sharing a wire protocol; the registry
    // decides which one is configured. Neither configured → stay on the primary
    // model, unchanged from before Laya existed.
    let registry = crate::config::load_registry()
        .map_err(|error| format!("读取增强判断模型配置失败: {error}"))?;
    let decision_input = DecisionInput {
        goal: goal.to_string(),
        observation: space.observation.clone(),
        candidates: decision_candidates(&space),
        recent_actions,
    };

    let (client, provider_name, fallback_to_primary_model): (
        std::sync::Arc<dyn DecisionProvider>,
        &'static str,
        bool,
    ) = match registry.decision_backend() {
        Some(crate::config::DecisionBackend::Jev) => {
            let mut config = registry.jev.clone();
            let fallback_to_primary_model = config.fallback_to_primary_model;
            config.enabled = true;
            let client = JevClient::from_config(config).map_err(|error| error.to_string())?;
            (
                std::sync::Arc::new(client),
                "jev",
                fallback_to_primary_model,
            )
        }
        Some(crate::config::DecisionBackend::Laya) => {
            let config = registry.laya.clone();
            let fallback_to_primary_model = config.fallback_to_primary_model;
            let client = LayaClient::from_config(to_laya_config(config))
                .map_err(|error| error.to_string())?;
            (
                std::sync::Arc::new(client),
                "laya",
                fallback_to_primary_model,
            )
        }
        None => {
            return Ok(json!({
                "status": "needs_primary_decision",
                "reason": "增强判断模型未配置，当前由主模型从同一候选动作空间选择；效果可能较差并消耗更多 Token",
                "action_space": action_space_json(&space),
            }));
        }
    };

    let decision = match client.choose(decision_input).await {
        Ok(decision) => decision,
        Err(error) if fallback_to_primary_model => {
            tracing::warn!(provider = provider_name, error = %error, "Enhanced semantic choice failed; falling back to primary model");
            return Ok(json!({
                "status": "needs_primary_decision",
                "reason": "增强判断模型暂时不可用，已回退当前主模型选择；详细网络或协议错误仅记录在本地诊断中",
                "action_space": action_space_json(&space),
            }));
        }
        Err(error) => return Err(format!("增强判断模型决策失败: {error}")),
    };
    apply_enhanced_decision(backend, &space, decision, provider_name).await
}

/// Bridge the registry's `LayaConfig` into the provider's own config type.
///
/// `LayaClient` deliberately does not depend on `crate::config`, so that the
/// provider stays a plain decision backend testable without a registry.
fn to_laya_config(config: crate::config::LayaConfig) -> LayaConfig {
    LayaConfig {
        base_url: config.base_url,
        model: config.model,
        timeout_ms: config.timeout_ms,
        max_retries: config.max_retries,
        api_key: config.api_key,
    }
}

async fn apply_enhanced_decision(
    backend: &SemanticDesktopBackend,
    space: &ActionSpace,
    decision: crate::desktop_automation::Decision,
    provider_name: &str,
) -> Result<serde_json::Value, String> {
    validate_space_token(space, &space.token)?;
    if decision.candidate_id == PRIMARY_HANDOFF_ID {
        return Ok(json!({
            "status": "needs_primary_decision",
            "reason": "增强判断模型请求主模型结合目标与界面上下文继续判断；无需再次调用增强模型或询问用户",
            "action_space": action_space_json(space),
            "decision": {"provider": provider_name, "model": decision.actual_model,
                "confidence": decision.confidence, "usage": decision.usage},
        }));
    }
    let candidate = space
        .candidates
        .iter()
        .find(|candidate| candidate.id == decision.candidate_id)
        .cloned()
        .ok_or_else(|| "增强判断模型返回了候选集合之外的 ID".to_string())?;
    if matches!(candidate.kind, CandidateKind::Done) {
        backend.clear_space().await;
        return Ok(json!({
            "status": "needs_primary_completion_check",
            "reason": "增强判断模型只提出任务可能已完成；请由当前主模型结合业务目标确认是否结束，不会把该判断直接当作本地完成事实",
            "candidate_id": candidate.id,
            "decision": {
                "provider": provider_name,
                "model": decision.actual_model,
                "confidence": decision.confidence,
                "usage": decision.usage,
            },
        }));
    }
    if matches!(
        candidate.kind,
        CandidateKind::SetValue { .. }
            | CandidateKind::SetSecret { .. }
            | CandidateKind::SetRangeValue { .. }
    ) {
        return Ok(json!({
            "status": "needs_input_value",
            "reason": "增强判断模型已选择文本目标；请由当前主模型提供业务文本并调用 desktop_semantic_execute。文本不会发送给增强判断模型",
            "observation_token": space.token,
            "candidate_id": candidate.id,
            "description": candidate.public_description,
            "decision": {
                "provider": provider_name,
                "model": decision.actual_model,
                "confidence": decision.confidence,
                "usage": decision.usage,
            },
        }));
    }
    let mut result = execute_candidate(
        backend,
        &space.observation,
        &candidate,
        &ExecutionInput {
            delivery: space.scope.delivery,
            ..Default::default()
        },
        Some(json!({
            "provider": provider_name,
            "model": decision.actual_model,
            "confidence": decision.confidence,
            "usage": decision.usage,
        })),
    )
    .await?;
    result["workflow_step"] = workflow_step(space, &candidate);
    let verification = result
        .get("verification")
        .and_then(|value| serde_json::from_value::<Verification>(value.clone()).ok());
    if let (Some(verification), Some(goal_key)) = (verification, space.enhanced_goal_key.as_deref())
    {
        backend
            .record_enhanced_result(goal_key, &candidate, verification)
            .await;
    }
    Ok(result)
}

fn validate_space_token(space: &ActionSpace, observation_token: &str) -> Result<(), String> {
    if space.owner != crate::automation_gate::current_execution_owner() {
        return Err("stale_observation: 观察属于其他执行任务，请重新观察".into());
    }
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
        CandidateKind::SetChecked { checked }
            if input.checked.is_some_and(|value| value != checked) =>
        {
            return Err("checked 参数与本地候选的目标状态不一致，请重新选择候选".into());
        }
        CandidateKind::Scroll { direction, amount }
            if input.direction.is_some_and(|value| value != direction)
                || input.amount.is_some_and(|value| value != amount) =>
        {
            return Err("滚动参数与本地候选不一致，请重新选择候选".into());
        }
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
            let signals = backend
                .approval_signals
                .as_ref()
                .ok_or_else(|| DesktopActionError::refused("当前运行入口未连接关键操作确认"))?;
            let binding = super::desktop_approval::ApprovalBinding::new(
                json!({"app":before.app.id,"window":before.window.id,"candidate":candidate}),
                json!({"value":input.value,"checked":input.checked,"direction":input.direction,"amount":input.amount,"delivery_mode":input.delivery}),
            );
            let summary = format!(
                "{} · {}\n{}",
                before.app.display_name, before.window.title, reason
            );
            super::desktop_approval::authorize(signals, binding, &summary)
                .await
                .map_err(DesktopActionError::refused)?;
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
    let mut scope = backend
        .last_space
        .lock()
        .await
        .as_ref()
        .filter(|s| {
            s.observation.app.id == before.app.id && s.observation.window.id == before.window.id
        })
        .map(|s| s.scope.clone())
        .unwrap_or_default();
    scope.delivery = input.delivery;
    let fresh = backend
        .observer
        .observe(&scope)
        .await
        .map_err(|error| error.to_string())?;
    if fresh.app.id != before.app.id || fresh.window.id != before.window.id {
        backend.clear_space().await;
        return Err("前台应用或窗口已变化，候选动作已过期；请重新观察后再选择".into());
    }
    let verification_candidate = rebind_verification_target(before, &fresh, candidate)?;
    if let CandidateKind::SetChecked { checked } = candidate.kind {
        if verification_candidate
            .target
            .as_ref()
            .and_then(|id| fresh.nodes.iter().find(|n| &n.opaque_id == id))
            .is_some_and(|node| node.toggled == Some(checked))
        {
            backend.clear_space().await;
            backend.loop_state.lock().await.consecutive_stalls = 0;
            return Ok(
                json!({"status":"already_satisfied","candidate_id":candidate.id,
                "dispatch_state":DispatchState::NotSent,"effect":ActionEffect::Confirmed,
                "verification":Verification::Achieved,"business_goal_confirmed":false,"retry_action":false}),
            );
        }
    }
    // An execution error may mean a native timeout after dispatch. Consume the
    // token before attempting it; callers must observe rather than replay.
    backend.clear_space().await;
    let receipt = match backend.executor.execute(&fresh, candidate, input).await {
        Ok(receipt) => receipt,
        Err(error) => {
            if let crate::desktop_automation::AutomationError::Execution(ref message) = error {
                if DesktopActionError::decode(message).is_some() {
                    return Err(message.clone());
                }
            }
            return Ok(json!({
                "status": "needs_observation", "candidate_id": candidate.id,
                "verification": Verification::Unknown, "dispatch_state": "unknown", "effect":ActionEffect::Unverifiable,
                "reason": error.to_string(), "retry_action": false, "decision": decision,
            }));
        }
    };
    if receipt.candidate_id != candidate.id {
        return Err(DesktopActionError::encode(
            DispatchState::Unknown,
            "执行回执与候选动作不一致",
        ));
    }
    // Consume the observation once dispatch may have happened. A failed read
    // afterwards must never leave a replayable click/send candidate cached.
    backend.clear_space().await;
    let (after, verification) = match verify_with_settle(
        backend,
        &fresh,
        &verification_candidate,
        input,
        &scope,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            return Ok(json!({
                "status": "needs_observation", "candidate_id": candidate.id,
                "receipt": receipt, "verification": Verification::Unknown,
                "dispatch_state": if receipt.dispatched { DispatchState::Sent } else { DispatchState::NotSent },
                "effect":ActionEffect::Unverifiable,
                "reason": error, "retry_action": false, "decision": decision,
            }))
        }
    };
    if matches!(
        verification,
        Verification::Achieved | Verification::Progress
    ) {
        backend.loop_state.lock().await.consecutive_stalls = 0;
    }
    Ok(json!({
        "status": if matches!(verification, Verification::Achieved | Verification::Progress) { "executed" } else { "needs_observation" },
        "candidate_id": candidate.id,
        "description": candidate.public_description,
        "receipt": receipt,
        "verification": verification,
        "dispatch_state": if receipt.dispatched { DispatchState::Sent } else { DispatchState::NotSent },
        "effect": if verification == Verification::Achieved { ActionEffect::Confirmed } else if verification == Verification::NoChange { ActionEffect::SuspectedNoop } else { ActionEffect::Unverifiable },
        "before_revision": fresh.revision,
        "after_revision": after.revision,
        "after_window": after.window,
        "after_app": after.app,
        "business_goal_confirmed": false,
        "retry_action": false,
        "decision": decision,
    }))
}

fn rebind_verification_target(
    before: &Observation,
    fresh: &Observation,
    candidate: &ActionCandidate,
) -> Result<ActionCandidate, String> {
    let mut rebound = candidate.clone();
    if let Some(target_id) = candidate.target.as_deref() {
        let original = before
            .nodes
            .iter()
            .find(|node| node.opaque_id == target_id)
            .ok_or_else(|| "候选动作目标不属于原始观察".to_string())?;
        let TargetResolution::Unique(target) = resolve_target(fresh, original) else {
            return Err("目标控件已消失或匹配不唯一，请重新观察".into());
        };
        rebound.target = Some(target.opaque_id.clone());
    }
    Ok(rebound)
}

async fn execute_persistent_action_with_options(
    backend: &SemanticDesktopBackend,
    locator: SemanticLocator,
    action: NativeAction,
    input: ExecutionInput,
    scope: ObservationScope,
    options: ActionOptions,
    targets: Option<Arc<crate::desktop::targets::DesktopTargetService>>,
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
    let (observation, scope) = backend
        .observer
        .observe_locator_scoped(&locator, &scope)
        .await
        .map_err(|e| e.to_string())?;
    // Persist only stable ancestry; the adapter reconstructs a fresh local
    // region after restart. Reuse it for dispatch and verification as well.
    let before = backend
        .store_observation("", None, scope.clone(), None, observation)
        .await?
        .observation;
    let candidate = backend
        .candidates
        .rebuild_semantic_candidate_with_input(&locator, action, &before, &input)
        .map_err(|error| error.to_string())?;
    let mut result = execute_candidate(backend, &before, &candidate, &input, None).await?;
    if let Some(expectation) = &options.expectation {
        let dispatch = serde_json::from_value(result["dispatch_state"].clone())
            .unwrap_or(DispatchState::Unknown);
        let checked = verify_expected_state(backend, expectation, &scope, targets.as_deref())
            .await
            .map_err(|error| DesktopActionError::encode(dispatch, error))?;
        if checked["status"] != "satisfied" {
            return Err(DesktopActionError::encode(
                dispatch,
                format!("后置条件尚未满足，不重发原动作: {}", checked),
            ));
        }
        result["postcondition"] = checked;
        result["effect"] = json!(ActionEffect::Confirmed);
        result["verification"] = json!(Verification::Achieved);
        result["status"] = json!("executed");
        return Ok(result);
    }
    let verification = result
        .get("verification")
        .cloned()
        .and_then(|value| serde_json::from_value::<Verification>(value).ok())
        .unwrap_or(Verification::Unknown);
    if verification != Verification::Achieved {
        let sent = result["dispatch_state"] == "sent";
        let ordinary = !matches!(
            candidate.local_risk,
            crate::desktop_automation::RiskClass::ExternalCommit
                | crate::desktop_automation::RiskClass::DestructiveCritical
                | crate::desktop_automation::RiskClass::Restricted
        );
        if sent
            && ordinary
            && (options.completion == CompletionPolicy::Dispatched
                || (options.completion == CompletionPolicy::Auto
                    && verification == Verification::Unknown))
        {
            result["status"] = json!("dispatched_unverified");
            result["note"] = json!("已发送，效果未验证；可继续后续定位，不代表业务目标完成");
        } else {
            return Err(DesktopActionError::encode(if sent { DispatchState::Sent } else { DispatchState::Unknown },
                format!("desktop_needs_observation: 请先检查结果，不自动重发。执行后验证: {verification:?}")));
        }
    }
    Ok(result)
}

async fn verify_expected_state(
    backend: &SemanticDesktopBackend,
    expected: &DesktopExpectation,
    scope: &ObservationScope,
    targets: Option<&crate::desktop::targets::DesktopTargetService>,
) -> Result<serde_json::Value, String> {
    expected.validate()?;
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_millis(expected.timeout_ms);
    let mut stable = 0_u8;
    let mut last: StateStatus;
    let mut reason = "condition_not_observed".to_string();
    loop {
        if backend
            .approval_signals
            .as_ref()
            .is_some_and(super::desktop_approval::cancellation_requested)
        {
            return Err("已取消桌面状态检查；不重发原动作".into());
        }
        let mut actual_scope = scope.clone();
        let mut missing_window = false;
        let mut can_observe = true;
        if let Some(targets) = targets {
            match targets.verification_scope(&expected.locator).await {
                Ok(Some(resolved)) => actual_scope = resolved,
                Ok(None) => missing_window = true,
                Err(error) => {
                    reason = error;
                    can_observe = false;
                }
            }
        }
        if missing_window {
            last = if matches!(expected.condition, StateCondition::WindowAbsent) {
                StateStatus::Satisfied
            } else {
                StateStatus::Unsatisfied
            };
            reason = "window_not_present".into();
        } else if can_observe {
            actual_scope.tree_offset = 0;
            actual_scope.subtree_id = None;
            let read_deadline = if expected.timeout_ms == 0 {
                started + Duration::from_secs(5)
            } else {
                deadline
            };
            match tokio::time::timeout_at(read_deadline, backend.observer.observe(&actual_scope))
                .await
            {
                Ok(Ok(observation)) => {
                    let candidates = backend
                        .candidates
                        .build("", &observation)
                        .map_err(|e| e.to_string())?;
                    let mut ids = std::collections::HashSet::new();
                    for candidate in &candidates {
                        if let Some(locator) = backend.candidates.semantic_locator(candidate) {
                            if locator_matches(&expected.locator, &locator) {
                                if let Some(target) = &candidate.target {
                                    ids.insert(target.clone());
                                }
                            }
                        }
                    }
                    let mut all_locators_available = true;
                    let matches: Vec<_> = observation
                        .nodes
                        .iter()
                        .filter(|node| {
                            if let Some(locator) =
                                backend.candidates.node_locator(&observation, node)
                            {
                                return locator_matches(&expected.locator, &locator);
                            }
                            all_locators_available = false;
                            ids.contains(&node.opaque_id)
                                || (expected.locator.automation_id.is_none()
                                    && expected.locator.ancestor_chain.is_empty()
                                    && expected
                                        .locator
                                        .accessible_name
                                        .as_ref()
                                        .is_none_or(|v| node.name.as_ref() == Some(v))
                                    && expected
                                        .locator
                                        .role
                                        .as_ref()
                                        .is_none_or(|v| &node.role == v))
                        })
                        .collect();
                    let mut resolved_expected = expected.clone();
                    // Platform adapters know both durable and live window IDs.
                    // A durable identity takes priority over mutable title hints.
                    resolved_expected.locator.window_id = None;
                    resolved_expected.locator.window_title = None;
                    last = if backend
                        .candidates
                        .matches_window(&expected.locator, &observation)
                    {
                        resolved_expected.evaluate(&observation, &matches)
                    } else {
                        StateStatus::Unknown
                    };
                    if expected.condition == StateCondition::ValueEquals
                        && matches
                            .iter()
                            .any(|node| !backend.candidates.value_readback_reliable(node))
                    {
                        last = StateStatus::Unknown;
                    }
                    // A locator not represented by the public tree cannot establish absence.
                    if !all_locators_available
                        && matches.is_empty()
                        && (expected.locator.automation_id.is_some()
                            || !expected.locator.ancestor_chain.is_empty())
                    {
                        last = StateStatus::Unknown;
                    }
                    reason = if observation.truncated {
                        "partial_tree"
                    } else {
                        "state_readback"
                    }
                    .into();
                }
                Ok(Err(error)) => {
                    last = StateStatus::Unknown;
                    reason = error.to_string();
                }
                Err(_) => {
                    last = StateStatus::Unknown;
                    reason = "observation_timeout".into();
                }
            }
        } else {
            last = StateStatus::Unknown;
        }
        stable = if last == StateStatus::Satisfied {
            stable.saturating_add(1)
        } else {
            0
        };
        if stable >= expected.stable_samples {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            if last == StateStatus::Satisfied {
                last = StateStatus::Unknown;
                reason = "not_yet_stable".into();
            }
            break;
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(125)).min(deadline),
        )
        .await;
    }
    Ok(
        json!({"status":last,"stable_samples":stable,"elapsed_ms":started.elapsed().as_millis(),"reason":reason,"read_only":true}),
    )
}

fn locator_matches(expected: &SemanticLocator, actual: &SemanticLocator) -> bool {
    expected.app_id == actual.app_id
        && expected
            .role
            .as_ref()
            .is_none_or(|v| actual.role.as_ref() == Some(v))
        && expected
            .automation_id
            .as_ref()
            .is_none_or(|v| actual.automation_id.as_ref() == Some(v))
        && expected
            .accessible_name
            .as_ref()
            .is_none_or(|v| actual.accessible_name.as_ref() == Some(v))
        && (expected.ancestor_chain.is_empty() || expected.ancestor_chain == actual.ancestor_chain)
}

async fn verify_with_settle(
    backend: &SemanticDesktopBackend,
    before: &Observation,
    candidate: &ActionCandidate,
    input: &ExecutionInput,
    scope: &ObservationScope,
) -> Result<(Observation, Verification), String> {
    const ATTEMPTS: usize = 6;
    const SETTLE_MS: u64 = 125;

    let mut last = observe_after_action(backend, scope).await?;
    for attempt in 0..ATTEMPTS {
        let verification =
            verify_observation_with_value(before, candidate, &last, input.value.as_deref());
        if !matches!(verification, Verification::NoChange | Verification::Unknown)
            || attempt + 1 == ATTEMPTS
        {
            return Ok((last, verification));
        }
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        last = observe_after_action(backend, scope).await?;
    }
    unreachable!("bounded verification loop always returns")
}

async fn observe_after_action(
    backend: &SemanticDesktopBackend,
    scope: &ObservationScope,
) -> Result<Observation, String> {
    let verification_scope = scope.clone();
    match backend.observer.observe(&verification_scope).await {
        Ok(observation) => Ok(observation),
        Err(_) if scope.subtree_id.is_some() => {
            let mut observation = backend
                .observer
                .observe(&ObservationScope {
                    subtree_id: None,
                    tree_offset: 0,
                    ..scope.clone()
                })
                .await
                .map_err(|e| e.to_string())?;
            // A different observation region is not evidence that the original
            // target disappeared. Still report foreground app/window changes.
            observation.truncated = true;
            Ok(observation)
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
fn verify_observation(
    before: &Observation,
    candidate: &ActionCandidate,
    after: &Observation,
) -> Verification {
    verify_observation_with_value(before, candidate, after, None)
}

fn verify_observation_with_value(
    before: &Observation,
    candidate: &ActionCandidate,
    after: &Observation,
    expected_value: Option<&str>,
) -> Verification {
    // The foreground can change while an application processes an action.
    // A similarly named control in another app is not evidence of success.
    if before.app.id != after.app.id {
        return Verification::Unexpected;
    }
    if before.window.id != after.window.id {
        // A different window (even in the same process) is not proof that the
        // target closed. Explicit window postconditions verify such transitions.
        return Verification::Unknown;
    }
    if matches!(
        candidate.kind,
        CandidateKind::SetValue { .. } | CandidateKind::SetRangeValue { .. }
    ) && candidate.expected_effects.iter().any(|predicate| {
        predicate
            .arguments
            .get("readback_trust")
            .is_some_and(|v| v == "web_content")
    }) {
        // Web AX setters may change the accessibility value without dispatching
        // framework input/change events. Require independent application state.
        return Verification::Unknown;
    }
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
                    || old.toggled != new.toggled
                    || old.selected != new.selected
                    || old.expanded != new.expanded
                    || old.value_fingerprint != new.value_fingerprint)
        }
        (CandidateKind::Invoke, Some(old), Some(TargetResolution::Missing)) => {
            invoke_expected(candidate)
                && !after.truncated
                && target_is_unique(before, old)
                && before.app.id == after.app.id
        }
        (CandidateKind::SetChecked { checked }, _, Some(TargetResolution::Unique(new))) => {
            new.toggled == Some(*checked)
        }
        (CandidateKind::ScrollIntoView, _, Some(TargetResolution::Unique(new))) => new.visible,
        (CandidateKind::SetRangeValue { .. }, _, Some(TargetResolution::Unique(new))) => {
            expected_value
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .is_some_and(|v| {
                    new.value_fingerprint.as_deref()
                        == Some(
                            crate::desktop_automation::value_fingerprint(&v.to_string()).as_str(),
                        )
                })
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
        (CandidateKind::SetValue { .. }, _, Some(TargetResolution::Unique(new))) => expected_value
            .is_some_and(|value| {
                new.value_fingerprint.as_deref()
                    == Some(crate::desktop_automation::value_fingerprint(value).as_str())
            }),
        _ => false,
    };
    if achieved {
        Verification::Achieved
    } else if matches!(
        candidate.kind,
        CandidateKind::Invoke | CandidateKind::Scroll { .. }
    ) {
        Verification::Unknown
    } else {
        let readable = match (&candidate.kind, target_before, target_after) {
            (CandidateKind::Toggle, Some(old), Some(TargetResolution::Unique(new))) => {
                old.toggled.is_some() && new.toggled.is_some()
            }
            (CandidateKind::SetChecked { .. }, _, Some(TargetResolution::Unique(new))) => {
                new.toggled.is_some()
            }
            (CandidateKind::Select, _, Some(TargetResolution::Unique(new))) => {
                new.selected.is_some()
            }
            (
                CandidateKind::Expand | CandidateKind::Collapse,
                _,
                Some(TargetResolution::Unique(new)),
            ) => new.expanded.is_some(),
            (
                CandidateKind::Focus | CandidateKind::ScrollIntoView,
                _,
                Some(TargetResolution::Unique(_)),
            ) => true,
            (
                CandidateKind::SetValue { .. } | CandidateKind::SetRangeValue { .. },
                _,
                Some(TargetResolution::Unique(new)),
            ) => expected_value.is_some() && new.value_fingerprint.is_some(),
            _ => false,
        };
        // Unavailable readback is not evidence of a no-op or a stalled task.
        if readable {
            Verification::NoChange
        } else {
            Verification::Unknown
        }
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
    let identity_matches = match (&left.semantic_key, &right.semantic_key) {
        (Some(left), Some(right)) => left == right,
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => match (
            uia_semantic_key(&left.opaque_id),
            uia_semantic_key(&right.opaque_id),
        ) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        },
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
    if candidates.len() > 4096 {
        return Err("本地候选动作超过有界观察容量".into());
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
    action_space_page(space, 0)
}

fn workflow_step(space: &ActionSpace, candidate: &ActionCandidate) -> serde_json::Value {
    space
        .persistent_locators
        .get(&candidate.id)
        .and_then(|locator| {
            locator.supported_action.as_ref().map(|action| {
                let mut step = json!({
                "tool": "desktop_semantic_action",
                "params": {"locator": locator, "action": action, "delivery_mode":"auto"},
                });
                if let Some(launch_ref) = &space.launch_ref {
                    step["params"]["launch_ref"] = json!(launch_ref);
                }
                match candidate.kind {
                    CandidateKind::SetChecked { checked } => {
                        step["params"]["checked"] = json!(checked)
                    }
                    CandidateKind::Scroll { direction, amount } => {
                        step["params"]["direction"] = json!(direction);
                        step["params"]["amount"] = json!(amount);
                    }
                    _ => {}
                }
                step
            })
        })
        .unwrap_or(serde_json::Value::Null)
}

fn candidate_summary(candidate: &ActionCandidate) -> serde_json::Value {
    json!({"id": candidate.id, "kind": candidate.kind,
        "description": candidate.public_description.chars().take(360).collect::<String>(),
        "risk": candidate.local_risk, "target": candidate.target})
}

fn action_space_page(space: &ActionSpace, cursor: usize) -> serde_json::Value {
    let mut result = json!({
        "observation_token": space.token,
        "tree_incomplete": space.observation.truncated,
        "next_tree_cursor": next_tree_cursor(space),
        "expires_in_ms": ACTION_SPACE_TTL.saturating_sub(space.created_at.elapsed()).as_millis() as u64,
        "observation": {
            "revision": space.observation.revision,
            "app": space.observation.app,
            "window": space.observation.window,
            "element_count": space.observation.nodes.len(),
            "captured_at_ms": space.observation.captured_at_ms,
            "truncated": space.observation.truncated,
        },
        "candidate_count": space.candidates.len(),
        "cursor": cursor,
        "next_cursor": null,
        "candidates": [],
        "control_candidates": space.candidates.iter().filter(|c| c.action_class() == ActionClass::Control).map(candidate_summary).collect::<Vec<_>>(),
        "details_tool": "desktop_semantic_candidate",
        "region_count": space.observation.nodes.len(),
        "regions_query": {"tool": "desktop_semantic_observe", "params": {
            "observation_token": space.token, "view": "regions", "cursor": 0
        }},
    });
    let mut end = cursor.min(space.candidates.len());
    for candidate in space.candidates.iter().skip(end).take(CANDIDATE_PAGE_SIZE) {
        result["candidates"]
            .as_array_mut()
            .unwrap()
            .push(candidate_summary(candidate));
        if result.to_string().chars().count() > CANDIDATE_PAGE_CHARS && end > cursor {
            result["candidates"].as_array_mut().unwrap().pop();
            break;
        }
        end += 1;
    }
    if end < space.candidates.len() {
        result["next_cursor"] = json!(end);
    }
    result["remaining_count"] = json!(space.candidates.len().saturating_sub(end));
    result
}

fn next_tree_cursor(space: &ActionSpace) -> Option<String> {
    (space.observation.truncated && space.observation.nodes.len() >= 200).then(|| {
        format!(
            "{}:tree:{}",
            space.token,
            space
                .scope
                .tree_offset
                .saturating_add(space.observation.nodes.len())
        )
    })
}

fn region_page(space: &ActionSpace, cursor: usize) -> serde_json::Value {
    let nodes = &space.observation.nodes;
    let end = (cursor + CANDIDATE_PAGE_SIZE).min(nodes.len());
    json!({
        "observation_token": space.token, "view": "regions",
        "region_count": nodes.len(), "cursor": cursor,
        "next_cursor": (end < nodes.len()).then_some(end),
        "regions": nodes.iter().skip(cursor).take(CANDIDATE_PAGE_SIZE).map(|node| json!({
            "subtree_id": node.opaque_id, "role": node.role,
            "name": node.name.as_ref().map(|s| s.chars().take(80).collect::<String>()),
        })).collect::<Vec<_>>(),
        "hint": "从区域列表选取 subtree_id 发起新的局部观察；新观察会替换旧候选。菜单可通过 scope=menu 观察。",
    })
}

fn decision_candidates(space: &ActionSpace) -> Vec<ActionCandidate> {
    // Keep one control's many action variants from hiding all other targets.
    let actionable: Vec<_> = space
        .candidates
        .iter()
        .filter(|c| c.action_class() != ActionClass::Control)
        .collect();
    let mut targets = std::collections::HashSet::new();
    let mut candidates: Vec<ActionCandidate> = actionable
        .iter()
        .filter(|c| targets.insert(c.target.as_deref().unwrap_or(&c.id)))
        .take(24)
        .map(|c| (*c).clone())
        .collect();
    for candidate in actionable {
        if candidates.len() >= 36 {
            break;
        }
        if !candidates.iter().any(|chosen| chosen.id == candidate.id) {
            candidates.push(candidate.clone());
        }
    }
    candidates.extend(
        space
            .candidates
            .iter()
            .filter(|c| c.action_class() == ActionClass::Control)
            .take(3)
            .cloned(),
    );
    candidates.push(ActionCandidate {
        id: PRIMARY_HANDOFF_ID.into(), observation_revision: space.observation.revision,
        target: None, kind: CandidateKind::CannotProceed,
        public_description: format!("Hand off to the primary reasoning model: choose when target/intent is ambiguous, more context or another candidate page is needed, or no clear action is justified. This batch offers {} of {} local candidates; tree incomplete: {}. No user approval is requested.", candidates.len(), space.candidates.len(), space.observation.truncated),
        local_risk: crate::desktop_automation::RiskClass::ReadOnly,
        preconditions: vec![], expected_effects: vec![],
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_automation::Predicate;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[tokio::test]
    async fn candidate_pages_are_complete_json_and_keep_ids_and_controls() {
        let backend = SemanticDesktopBackend::new(Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        }));
        let mut space = backend.observe("open", None).await.unwrap();
        let template = space.candidates[0].clone();
        space.candidates = (0..80)
            .map(|i| {
                let mut c = template.clone();
                c.id = format!("candidate-{i}");
                c.public_description = "界面内容与上下文".repeat(100);
                c
            })
            .collect();
        let mut control = template;
        control.id = "cannot-proceed".into();
        control.kind = CandidateKind::CannotProceed;
        control.public_description = "No native candidate".into();
        space.candidates.push(control);
        let mut cursor = 0;
        let mut ids = Vec::new();
        loop {
            let page = action_space_page(&space, cursor);
            assert!(page.to_string().chars().count() < 8_000);
            assert_eq!(page["control_candidates"][0]["id"], "cannot-proceed");
            let wire = crate::utils::truncate_tool_output(
                &page.to_string(),
                8000,
                "desktop_semantic_observe",
            );
            let reparsed: serde_json::Value = serde_json::from_str(&wire).unwrap();
            ids.extend(
                reparsed["candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c["id"].as_str().unwrap().to_owned()),
            );
            match page["next_cursor"].as_u64() {
                Some(next) => {
                    assert!(next as usize > cursor);
                    cursor = next as usize;
                }
                None => break,
            }
        }
        assert_eq!(
            ids,
            space
                .candidates
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>()
        );
        let batch = decision_candidates(&space);
        assert!(batch.len() <= 40);
        assert!(batch.iter().any(|c| c.id == PRIMARY_HANDOFF_ID));
        assert_eq!(space.candidates.len(), 81);
    }

    #[tokio::test]
    async fn consumed_action_token_cannot_send_a_second_native_action() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        });
        let backend = SemanticDesktopBackend::new(adapter.clone());
        let space = backend.observe("open", None).await.unwrap();
        let result = execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default(),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "needs_observation");
        assert_eq!(result["retry_action"], false);
        assert!(execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default()
        )
        .await
        .is_err());
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stable_keyboard_target_rejects_mixed_or_incomplete_locators_before_input() {
        let registry = ToolRegistry::new();
        let mixed = registry
            .semantic_input_window(&json!({"hwnd": 42, "target_locator": {"app_id": "test"}}))
            .await
            .unwrap_err();
        assert!(mixed.contains("不能同时"));
        let missing = registry
            .semantic_input_window(&json!({"target_locator": {"role": "button"}}))
            .await
            .unwrap_err();
        assert!(missing.contains("稳定定位器"));
    }

    #[tokio::test]
    async fn native_error_after_dispatch_requires_observation_and_consumes_token() {
        struct AmbiguousDispatch(Arc<AtomicUsize>);
        #[async_trait]
        impl ComputerExecutor for AmbiguousDispatch {
            async fn execute(
                &self,
                _: &Observation,
                _: &ActionCandidate,
                _: &ExecutionInput,
            ) -> Result<
                crate::desktop_automation::ActionReceipt,
                crate::desktop_automation::AutomationError,
            > {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err(crate::desktop_automation::AutomationError::Execution(
                    "provider timed out after dispatch".into(),
                ))
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        let mut backend = SemanticDesktopBackend::new(Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        }));
        backend.executor = Arc::new(AmbiguousDispatch(count.clone()));
        let space = backend.observe("send", None).await.unwrap();
        let result = execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default(),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "needs_observation");
        assert_eq!(result["retry_action"], false);
        assert_eq!(result["dispatch_state"], "unknown");
        assert!(execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default()
        )
        .await
        .is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_execution_owner_cannot_reuse_observation() {
        let backend = SemanticDesktopBackend::new(Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        }));
        let space = crate::automation_gate::with_execution_owner(
            "task-a".into(),
            backend.observe("open", None),
        )
        .await
        .unwrap();
        crate::automation_gate::with_execution_owner("task-b".into(), async {
            assert!(validate_space_token(&space, &space.token).is_err());
        })
        .await;
    }

    #[tokio::test]
    async fn enhanced_handoff_keeps_the_same_executable_space_without_dispatch() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let backend = SemanticDesktopBackend::new(adapter.clone());
        let space = backend.observe("open", Some("open".into())).await.unwrap();
        let result = apply_enhanced_decision(
            &backend,
            &space,
            crate::desktop_automation::Decision {
                candidate_id: PRIMARY_HANDOFF_ID.into(),
                confidence: Some(0.99),
                probabilities: Default::default(),
                actual_model: None,
                usage: None,
            },
            "laya",
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "needs_primary_decision");
        assert_eq!(result["action_space"]["observation_token"], space.token);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);
        let executed = execute_cached_candidate(
            &backend,
            &space.token,
            "fake-candidate",
            ExecutionInput::default(),
        )
        .await
        .unwrap();
        assert_eq!(executed["status"], "executed");
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn enhanced_choice_does_not_add_a_confidence_floor() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: true,
        });
        let backend = SemanticDesktopBackend::new(adapter.clone());
        let space = backend.observe("open", Some("open".into())).await.unwrap();
        let result = apply_enhanced_decision(
            &backend,
            &space,
            crate::desktop_automation::Decision {
                candidate_id: "fake-candidate".into(),
                confidence: Some(0.27),
                probabilities: Default::default(),
                actual_model: None,
                usage: None,
            },
            "laya",
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "executed");
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn enhanced_budget_is_not_reset_by_goal_rewording_in_one_task() {
        let backend = SemanticDesktopBackend::new(Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        }));
        crate::automation_gate::with_execution_owner("one-task".into(), async {
            for n in 0..100 {
                backend
                    .begin_enhanced_step(&format!("different step {n}"))
                    .await
                    .unwrap();
            }
            backend
                .begin_enhanced_step("entirely different wording")
                .await
                .unwrap();
            assert_eq!(backend.loop_state.lock().await.steps, 101);
        })
        .await;
    }

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
            truncated: false,
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
                    semantic_key: None,
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
                        crate::desktop_automation::value_fingerprint(
                            self.received_value
                                .lock()
                                .unwrap()
                                .as_deref()
                                .unwrap_or_default(),
                        )
                    } else {
                        crate::desktop_automation::value_fingerprint("before")
                    }),
                    supported_actions: vec![NativeAction::SetValue],
                });
            } else if matches!(
                self.candidate_kind,
                CandidateKind::Toggle | CandidateKind::SetChecked { .. }
            ) {
                // This fixture changes the window fingerprint, but deliberately
                // leaves the target checkbox unchanged after dispatch.
                let mut node = stateful_observation("checkbox", false).nodes.remove(0);
                node.opaque_id = "fake-target".into();
                node.name = Some("Fake checkbox".into());
                if let CandidateKind::SetChecked { checked } = self.candidate_kind {
                    node.toggled = Some(checked);
                }
                current.nodes.push(node);
            } else if matches!(self.candidate_kind, CandidateKind::Invoke) && !executed {
                current.nodes.push(crate::desktop_automation::UiNode {
                    opaque_id: "fake-target".into(),
                    semantic_key: None,
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
                    NativeAction::Toggle => CandidateKind::Toggle,
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
                delivery_mode: None,
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
        assert!(payload["candidates"][0].get("workflow_step").is_none());
        let detail = registry
            .execute_semantic_desktop_tool(
                "desktop_semantic_candidate",
                &json!({
                    "observation_token": payload["observation_token"],
                    "candidate_id": payload["candidates"][0]["id"],
                }),
            )
            .await
            .unwrap();
        let detail: serde_json::Value =
            serde_json::from_str(detail.output.as_deref().unwrap()).unwrap();
        let step = &detail["workflow_step"];

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
            .contains("desktop_needs_observation:"));
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
            semantic_key: None,
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
                semantic_key: None,
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

    #[test]
    fn stable_semantic_identity_verifies_only_the_target_container_after_reorder() {
        let mut before = invoke_observation("before", 2);
        before.nodes[0].semantic_key = Some("ax:row-a:open".into());
        before.nodes[1].semantic_key = Some("ax:row-b:open".into());
        let candidate = invoke_candidate("invoke-target-0");
        let mut after = before.clone();
        after.nodes.reverse();
        // A redraw replaces runtime ids; another row's change is not success.
        after.nodes[0].opaque_id = "new-runtime-b".into();
        after.nodes[1].opaque_id = "new-runtime-a".into();
        after.nodes[0].focused = true;
        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Unknown
        );
        after.nodes[1].focused = true;
        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Unknown
        );
        after.nodes[1].enabled = false;
        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Achieved
        );
    }

    #[test]
    fn verification_target_is_rebound_when_runtime_ids_change_before_dispatch() {
        let mut before = invoke_observation("before", 1);
        before.nodes[0].semantic_key = Some("ax:document:open".into());
        let candidate = invoke_candidate("invoke-target-0");
        let mut fresh = before.clone();
        fresh.nodes[0].opaque_id = "new-runtime-id".into();
        let rebound = rebind_verification_target(&before, &fresh, &candidate).unwrap();
        assert_eq!(rebound.target.as_deref(), Some("new-runtime-id"));
        assert_eq!(candidate.target.as_deref(), Some("invoke-target-0"));
        let mut after = fresh.clone();
        after.nodes[0].enabled = false;
        assert_eq!(
            verify_observation(&fresh, &rebound, &after),
            Verification::Achieved
        );
        fresh.nodes.clear();
        assert!(rebind_verification_target(&before, &fresh, &candidate).is_err());
    }

    #[test]
    fn missing_or_ambiguous_semantic_identity_never_guesses_a_target() {
        let mut snapshot = invoke_observation("before", 2);
        let mut target = snapshot.nodes[0].clone();
        target.semantic_key = Some("ax:row-a:open".into());
        assert!(matches!(
            resolve_target(&snapshot, &target),
            TargetResolution::Missing
        ));
        for node in &mut snapshot.nodes {
            node.semantic_key = target.semantic_key.clone();
        }
        assert!(matches!(
            resolve_target(&snapshot, &target),
            TargetResolution::Ambiguous
        ));
    }

    #[test]
    fn foreground_app_switch_cannot_verify_an_unrelated_control() {
        let before = invoke_observation("before", 1);
        let mut after = before.clone();
        after.app.id = "other-application".into();
        after.nodes[0].focused = true;
        let candidate = invoke_candidate("invoke-target-0");
        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Unexpected
        );
    }

    #[test]
    fn observations_without_semantic_key_remain_deserializable() {
        let node = invoke_observation("before", 1).nodes.remove(0);
        let serialized = serde_json::to_value(&node).unwrap();
        assert!(serialized.get("semantic_key").is_none());
        let restored: crate::desktop_automation::UiNode =
            serde_json::from_value(serialized).unwrap();
        assert_eq!(restored, node);
    }

    #[test]
    fn partial_observation_does_not_prove_target_disappearance() {
        let before = invoke_observation("before", 1);
        let mut after = invoke_observation("partial", 0);
        after.truncated = true;
        assert_eq!(
            verify_observation(&before, &invoke_candidate("invoke-target-0"), &after),
            Verification::Unknown
        );
        after.truncated = false;
        assert_eq!(
            verify_observation(&before, &invoke_candidate("invoke-target-0"), &after),
            Verification::Achieved
        );
    }

    #[test]
    fn set_value_verifies_expected_content_and_accepts_idempotent_replay() {
        let mut before = invoke_observation("before", 1);
        before.nodes[0].role = crate::desktop_automation::UiRole::TextField;
        before.nodes[0].value_fingerprint =
            Some(crate::desktop_automation::value_fingerprint("  目标文字  "));
        let mut candidate = invoke_candidate("invoke-target-0");
        candidate.kind = CandidateKind::SetValue {
            slot_id: "value".into(),
        };
        assert_eq!(
            verify_observation_with_value(&before, &candidate, &before, Some("  目标文字  ")),
            Verification::Achieved
        );
        assert_eq!(
            verify_observation_with_value(&before, &candidate, &before, None),
            Verification::Unknown
        );
        let mut after = before.clone();
        after.nodes[0].value_fingerprint =
            Some(crate::desktop_automation::value_fingerprint("目标文字"));
        assert_eq!(
            verify_observation_with_value(&before, &candidate, &after, Some("  目标文字  ")),
            Verification::NoChange
        );
        after.nodes[0].value_fingerprint = None;
        assert_eq!(
            verify_observation_with_value(&before, &candidate, &after, Some("  目标文字  ")),
            Verification::Unknown
        );
    }

    #[test]
    fn web_ax_value_does_not_prove_framework_input_events() {
        let mut before = invoke_observation("before", 1);
        before.nodes[0].value_fingerprint =
            Some(crate::desktop_automation::value_fingerprint("typed"));
        let mut candidate = invoke_candidate("invoke-target-0");
        candidate.kind = CandidateKind::SetValue {
            slot_id: "value".into(),
        };
        candidate.expected_effects[0]
            .arguments
            .insert("readback_trust".into(), "web_content".into());
        assert_eq!(
            verify_observation_with_value(&before, &candidate, &before, Some("typed")),
            Verification::Unknown
        );
    }

    #[tokio::test]
    async fn cancelled_postcondition_wait_does_not_send_actions() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        });
        let mut backend = SemanticDesktopBackend::new(adapter.clone());
        backend.approval_signals = Some(crate::state::new_shared_signals());
        let expected: DesktopExpectation = serde_json::from_value(json!({
            "locator":{"app_id":"fake-app","automation_id":"fake-button"},
            "condition":"exists", "timeout_ms":1000, "stable_samples":2
        }))
        .unwrap();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let scope = ObservationScope::default();
        let result = super::super::desktop_approval::with_cancellation(
            cancelled,
            verify_expected_state(&backend, &expected, &scope, None),
        )
        .await;
        assert!(result.unwrap_err().contains("已取消"));
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn postcondition_wait_is_read_only_and_requires_stability() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        });
        let backend = SemanticDesktopBackend::new(adapter.clone());
        let mut expected: DesktopExpectation = serde_json::from_value(json!({
            "locator":{"app_id":"fake-app","automation_id":"fake-button"},
            "condition":"exists", "timeout_ms":1000, "stable_samples":2
        }))
        .unwrap();
        let result = verify_expected_state(&backend, &expected, &ObservationScope::default(), None)
            .await
            .unwrap();
        assert_eq!(result["status"], "satisfied");
        assert_eq!(result["stable_samples"], 2);
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);
        expected.timeout_ms = 0;
        let result = verify_expected_state(&backend, &expected, &ObservationScope::default(), None)
            .await
            .unwrap();
        assert_eq!(result["status"], "unknown");
        assert_eq!(result["reason"], "not_yet_stable");
        expected.stable_samples = 1;
        expected.locator.automation_id = Some("missing-static-label".into());
        expected.condition = StateCondition::Absent;
        let result = verify_expected_state(&backend, &expected, &ObservationScope::default(), None)
            .await
            .unwrap();
        assert_eq!(
            result["status"], "unknown",
            "an adapter without static-node locators cannot prove absence"
        );
    }

    #[tokio::test]
    async fn ordinary_navigation_continues_but_verified_policy_never_replays() {
        for (completion, success) in [("auto", true), ("dispatched", true), ("verified", false)] {
            let adapter = Arc::new(FakeAdapter {
                executions: AtomicUsize::new(0),
                candidate_kind: CandidateKind::Invoke,
                received_value: Mutex::new(None),
                changes_after_execute: false,
            });
            let mut registry = ToolRegistry::new();
            registry.set_semantic_desktop_adapter(adapter.clone());
            let result = registry.execute_semantic_desktop_tool("desktop_semantic_action", &json!({
                "locator":{"app_id":"fake-app","role":"button"}, "action":"invoke", "completion_policy":completion
            })).await.unwrap();
            assert_eq!(result.success, success);
            if success {
                let output: serde_json::Value =
                    serde_json::from_str(result.output.as_deref().unwrap()).unwrap();
                assert_eq!(output["status"], "dispatched_unverified");
                assert_eq!(output["business_goal_confirmed"], false);
            } else {
                assert!(
                    !DesktopActionError::decode(result.error.as_deref().unwrap())
                        .unwrap()
                        .may_retry()
                );
            }
            assert_eq!(adapter.executions.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn desired_checked_state_does_not_toggle_again_and_recovers_stall() {
        let adapter = Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::SetChecked { checked: true },
            received_value: Mutex::new(None),
            changes_after_execute: false,
        });
        let backend = SemanticDesktopBackend::new(adapter.clone());
        backend.loop_state.lock().await.consecutive_stalls = STALL_LIMIT;
        for _ in 0..2 {
            let space = backend.observe("enable", None).await.unwrap();
            let result = execute_cached_candidate(
                &backend,
                &space.token,
                "fake-candidate",
                ExecutionInput::default(),
            )
            .await
            .unwrap();
            assert_eq!(result["status"], "already_satisfied");
            assert_eq!(result["dispatch_state"], "not_sent");
            assert_eq!(result["effect"], "confirmed");
        }
        assert_eq!(adapter.executions.load(Ordering::SeqCst), 0);
        assert_eq!(backend.loop_state.lock().await.consecutive_stalls, 0);
    }

    #[tokio::test]
    async fn decision_batch_reserves_room_for_distinct_targets() {
        let backend = SemanticDesktopBackend::new(Arc::new(FakeAdapter {
            executions: AtomicUsize::new(0),
            candidate_kind: CandidateKind::Invoke,
            received_value: Mutex::new(None),
            changes_after_execute: false,
        }));
        let mut space = backend.observe("open", None).await.unwrap();
        let template = space.candidates[0].clone();
        space.candidates = (0..80)
            .map(|i| {
                let mut candidate = template.clone();
                candidate.id = format!("c-{i}");
                candidate.target = Some(format!("target-{}", i / 10));
                candidate
            })
            .collect();
        let chosen = decision_candidates(&space);
        assert!(chosen.len() <= 40);
        for target in 0..8 {
            assert!(chosen
                .iter()
                .any(|c| c.target.as_deref() == Some(format!("target-{target}").as_str())));
        }
        assert!(chosen.iter().any(|c| c.id == PRIMARY_HANDOFF_ID));
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

        for index in 0..100 {
            let goal = if index % 2 == 0 {
                "Open the report"
            } else {
                "  OPEN   the report!!!  "
            };
            backend.begin_enhanced_step(goal).await.unwrap();
        }

        backend
            .begin_enhanced_step("open the report.")
            .await
            .unwrap();
        let state = backend.loop_state.lock().await;
        assert_eq!(state.steps, 101);
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
                    ..Default::default()
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

        let handoff =
            execute_jev_step(&backend, " fill field! ", ObservationScope::default(), None)
                .await
                .unwrap();
        assert_eq!(handoff["status"], "needs_primary_decision");
        assert!(handoff["action_space"]["observation_token"].is_string());
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

        assert_eq!(result["verification"], "unknown");
        let state = backend.loop_state.lock().await;
        assert_eq!(state.steps, 1);
        assert_eq!(state.consecutive_stalls, 0);
        assert_eq!(state.recent_actions.len(), 1);
        assert_eq!(state.recent_actions[0].verification, Verification::Unknown);
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
            Verification::Unknown
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
    fn switching_to_another_app_window_requires_explicit_postcondition() {
        let before = invoke_observation("before", 1);
        let mut after = invoke_observation("after", 0);
        after.window.id = "main-window".into();
        after.window.title = "Document - Notepad".into();
        let candidate = invoke_candidate("invoke-target-0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Unknown
        );
    }

    #[test]
    fn duplicate_invoke_targets_cannot_be_verified_by_disappearance() {
        let before = invoke_observation("before", 2);
        let after = invoke_observation("after", 0);
        let candidate = invoke_candidate("invoke-target-0");

        assert_eq!(
            verify_observation(&before, &candidate, &after),
            Verification::Unknown
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
            Verification::Unexpected
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
