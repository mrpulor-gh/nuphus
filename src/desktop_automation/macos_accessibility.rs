//! macOS Accessibility adapter. Native references never leave the AX worker.
//!
//! Only capabilities actually advertised by a control become candidates. Saved
//! locators use bundle identifiers and semantic ancestry, not pid/AX pointers.

use super::types::*;
use super::{CandidateBuilder, ComputerExecutor, ComputerObserver};
use async_trait::async_trait;

#[cfg(any(target_os = "macos", test))]
mod semantic {
    use super::*;
    use std::hash::{Hash, Hasher};

    #[derive(Debug, Clone)]
    pub(super) struct Metadata {
        pub node: UiNode,
        pub identifier: Option<String>,
        pub raw_name: Option<String>,
        pub ancestors: Vec<SemanticContext>,
    }

    pub(super) fn hash(value: &str) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    pub(super) fn redact(value: &str) -> String {
        let lower = value.to_lowercase();
        if value.contains('@')
            || value
                .split(|c: char| !c.is_ascii_digit())
                .any(|part| part.len() >= 8)
            || ["apikey_", "api_key", "bearer ", "password", "密码"]
                .iter()
                .any(|s| lower.contains(s))
        {
            "redacted control".into()
        } else {
            value.chars().take(80).collect()
        }
    }

    pub(super) fn role(value: &str) -> UiRole {
        match value {
            "AXWindow" | "AXSheet" => UiRole::Window,
            "AXButton" | "AXPopUpButton" | "AXLink" => UiRole::Button,
            "AXTextField" | "AXTextArea" | "AXComboBox" => UiRole::TextField,
            "AXCheckBox" | "AXSwitch" => UiRole::CheckBox,
            "AXRadioButton" => UiRole::RadioButton,
            "AXList" | "AXOutline" | "AXTable" => UiRole::List,
            "AXRow" | "AXCell" => UiRole::ListItem,
            "AXMenu" | "AXMenuBar" => UiRole::Menu,
            "AXMenuItem" | "AXMenuBarItem" => UiRole::MenuItem,
            "AXTabGroup" => UiRole::Tab,
            "AXDocument" | "AXWebArea" => UiRole::Document,
            _ => UiRole::Other,
        }
    }

    pub(super) fn actions(
        role: &UiRole,
        secure: bool,
        names: &[String],
        focused_settable: bool,
        value_settable: bool,
        selected_settable: bool,
        expanded_settable: bool,
        expanded: Option<bool>,
    ) -> Vec<NativeAction> {
        let mut result = Vec::new();
        if names.iter().any(|name| name == "AXPress") {
            result.push(match role {
                UiRole::CheckBox => NativeAction::Toggle,
                UiRole::RadioButton => NativeAction::Select,
                _ => NativeAction::Invoke,
            });
        }
        if selected_settable && !result.contains(&NativeAction::Select) {
            result.push(NativeAction::Select);
        }
        if expanded_settable {
            match expanded {
                Some(true) => result.push(NativeAction::Collapse),
                Some(false) => result.push(NativeAction::Expand),
                None => {}
            }
        }
        if focused_settable {
            result.push(NativeAction::Focus);
        }
        // Values of password fields remain local and require the existing
        // explicit secret-input path, never an ordinary model value slot.
        if value_settable && !secure && *role == UiRole::TextField {
            result.push(NativeAction::SetValue);
        }
        result
    }

    pub(super) fn kind(action: &NativeAction) -> Result<CandidateKind, AutomationError> {
        Ok(match action {
            NativeAction::Invoke => CandidateKind::Invoke,
            NativeAction::Toggle => CandidateKind::Toggle,
            NativeAction::Select => CandidateKind::Select,
            NativeAction::Expand => CandidateKind::Expand,
            NativeAction::Collapse => CandidateKind::Collapse,
            NativeAction::Focus => CandidateKind::Focus,
            NativeAction::SetValue => CandidateKind::SetValue {
                slot_id: "value".into(),
            },
            _ => return Err(AutomationError::Candidates("unsupported AX action".into())),
        })
    }

    pub(super) fn risk(action: &NativeAction, name: Option<&str>) -> RiskClass {
        if *action == NativeAction::SetValue {
            return RiskClass::BoundedWrite;
        }
        if *action != NativeAction::Invoke {
            return RiskClass::Reversible;
        }
        let name = name.unwrap_or_default().to_lowercase();
        if [
            "permanently delete",
            "永久删除",
            "delete account",
            "删除账号",
            "删除账户",
            "注销账号",
            "注销账户",
            "delete all data",
            "erase all data",
            "wipe all data",
            "删除所有数据",
            "清除所有数据",
            "factory reset",
            "恢复出厂设置",
            "format drive",
            "format disk",
            "格式化磁盘",
            "purchase",
            "place order",
            "buy now",
            "支付",
            "付款",
            "提交订单",
            "transfer funds",
            "wire transfer",
            "转账",
            "grant permission",
            "allow access",
            "授予权限",
            "允许访问",
            "security settings",
            "安全设置",
        ]
        .iter()
        .any(|word| name.contains(word))
        {
            RiskClass::DestructiveCritical
        } else if ["send", "发送", "publish", "发布", "submit", "提交"]
            .iter()
            .any(|word| name.contains(word))
        {
            RiskClass::ExternalCommit
        } else if ["save", "保存", "apply", "应用"]
            .iter()
            .any(|word| name.contains(word))
        {
            RiskClass::BoundedWrite
        } else {
            RiskClass::Reversible
        }
    }

    pub(super) fn matches(
        meta: &Metadata,
        locator: &SemanticLocator,
        action: &NativeAction,
    ) -> bool {
        meta.node.enabled
            && meta.node.visible
            && locator.role.as_ref().is_none_or(|r| r == &meta.node.role)
            && locator
                .automation_id
                .as_ref()
                .is_none_or(|id| Some(id) == meta.identifier.as_ref())
            && locator
                .accessible_name
                .as_ref()
                .is_none_or(|name| Some(name) == meta.raw_name.as_ref())
            && (locator.ancestor_chain.is_empty() || locator.ancestor_chain == meta.ancestors)
            && meta.node.supported_actions.contains(action)
    }

    pub(super) fn effects(action: &NativeAction, target: &str) -> Vec<Predicate> {
        let name = match action {
            NativeAction::Invoke => "invoke_target_changed_or_disappeared",
            NativeAction::Toggle => "target_toggle_changed",
            NativeAction::Select => "target_selected",
            NativeAction::Expand => "target_expanded",
            NativeAction::Collapse => "target_collapsed",
            NativeAction::Focus => "target_focused",
            NativeAction::SetValue => "target_value_changed",
            _ => return vec![],
        };
        vec![Predicate {
            name: name.into(),
            arguments: [("target".into(), target.into())].into(),
        }]
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod native;

#[cfg(target_os = "macos")]
mod platform {
    use super::semantic::*;
    use super::*;
    use std::collections::HashMap;
    use std::sync::{mpsc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::oneshot;

    #[derive(Clone)]
    struct BoundAction {
        candidate: ActionCandidate,
        locator: SemanticLocator,
        action: NativeAction,
    }

    #[derive(Default)]
    struct State {
        revision: u64,
        fingerprint: String,
        observation: Option<Observation>,
        metadata: Vec<Metadata>,
        actions: HashMap<String, BoundAction>,
    }

    enum Request {
        Observe(
            ObservationScope,
            Instant,
            oneshot::Sender<Result<native::Snapshot, AutomationError>>,
        ),
        Execute(
            BoundAction,
            ExecutionInput,
            Instant,
            oneshot::Sender<Result<ActionReceipt, AutomationError>>,
        ),
    }

    pub struct MacosAccessibilityAdapter {
        worker: mpsc::SyncSender<Request>,
        state: Mutex<State>,
    }

    impl Default for MacosAccessibilityAdapter {
        fn default() -> Self {
            Self::new(200)
        }
    }

    impl MacosAccessibilityAdapter {
        pub fn new(max_elements: usize) -> Self {
            let (sender, receiver) = mpsc::sync_channel(1);
            // A failed spawn disconnects the sender; observe then reports a
            // regular adapter error instead of crashing the application.
            let _ = std::thread::Builder::new()
                .name("nuphus-ax".into())
                .spawn(move || {
                    while let Ok(request) = receiver.recv() {
                        match request {
                            Request::Observe(scope, deadline, reply) => {
                                if !reply.is_closed() {
                                    let result = native::capture(
                                        max_elements.clamp(1, 200),
                                        &scope,
                                        deadline,
                                    );
                                    let _ = reply.send(result);
                                }
                            }
                            Request::Execute(bound, input, deadline, reply) => {
                                if !reply.is_closed() {
                                    let result = native::execute(
                                        max_elements.clamp(1, 200),
                                        &bound.locator,
                                        &bound.action,
                                        &input,
                                        deadline,
                                        || reply.is_closed(),
                                    )
                                    .map(|()| ActionReceipt {
                                        candidate_id: bound.candidate.id,
                                        dispatched: true,
                                        detail: Some(
                                            "macOS Accessibility action dispatched".into(),
                                        ),
                                    });
                                    let _ = reply.send(result);
                                }
                            }
                        }
                    }
                });
            Self {
                worker: sender,
                state: Mutex::new(State::default()),
            }
        }

        fn bind(
            state: &mut State,
            observation: &Observation,
            meta: &Metadata,
            action: NativeAction,
        ) -> Result<ActionCandidate, AutomationError> {
            let locator = SemanticLocator {
                app_id: observation.app.id.clone(),
                window_id: Some(observation.window.id.clone()),
                window_title: Some(observation.window.title.clone()),
                role: Some(meta.node.role.clone()),
                automation_id: meta.identifier.clone(),
                accessible_name: meta.raw_name.clone(),
                ancestor_chain: meta.ancestors.clone(),
                supported_action: Some(action.clone()),
                ordinal_hint: None,
            };
            let candidate = ActionCandidate {
                id: format!("ax:{}", uuid::Uuid::new_v4().simple()),
                observation_revision: observation.revision,
                target: Some(meta.node.opaque_id.clone()),
                kind: kind(&action)?,
                public_description: format!(
                    "{action:?} {:?} '{}'",
                    meta.node.role,
                    meta.node.name.as_deref().unwrap_or("unnamed control")
                ),
                local_risk: risk(&action, meta.raw_name.as_deref()),
                preconditions: vec![Predicate {
                    name: "semantic_element_exists".into(),
                    arguments: [("target".into(), meta.node.opaque_id.clone())].into(),
                }],
                expected_effects: effects(&action, &meta.node.opaque_id),
            };
            state.actions.insert(
                candidate.id.clone(),
                BoundAction {
                    candidate: candidate.clone(),
                    locator,
                    action,
                },
            );
            Ok(candidate)
        }

        fn current(state: &State, observation: &Observation) -> Result<(), AutomationError> {
            if state.observation.as_ref() != Some(observation) {
                return Err(AutomationError::Candidates(
                    "AX observation is stale or was not created by this adapter".into(),
                ));
            }
            Ok(())
        }
    }

    #[async_trait]
    impl ComputerObserver for MacosAccessibilityAdapter {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                supported: vec![
                    PlatformCapability::ReadSemanticTree,
                    PlatformCapability::NativeAction,
                    PlatformCapability::WriteValue,
                ],
                accessibility_permission: native::trusted(),
            }
        }

        async fn observe(&self, scope: &ObservationScope) -> Result<Observation, AutomationError> {
            let (reply, receive) = oneshot::channel();
            self.worker
                .try_send(Request::Observe(
                    scope.clone(),
                    Instant::now() + Duration::from_secs(5),
                    reply,
                ))
                .map_err(|_| {
                    AutomationError::Observation("AX worker is busy or unavailable".into())
                })?;
            let snapshot = tokio::time::timeout(Duration::from_secs(6), receive)
                .await
                .map_err(|_| AutomationError::Observation("AX observation timed out".into()))?
                .map_err(|_| AutomationError::Observation("AX worker stopped".into()))??;
            let mut state = self
                .state
                .lock()
                .map_err(|_| AutomationError::Observation("AX state unavailable".into()))?;
            if state.fingerprint != snapshot.fingerprint {
                state.revision = state.revision.saturating_add(1).max(1);
                state.fingerprint = snapshot.fingerprint.clone();
            }
            let observation = Observation {
                revision: state.revision,
                fingerprint: snapshot.fingerprint,
                app: snapshot.app,
                window: snapshot.window,
                nodes: snapshot.metadata.iter().map(|m| m.node.clone()).collect(),
                captured_at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            };
            state.metadata = snapshot.metadata;
            state.observation = Some(observation.clone());
            Ok(observation)
        }
    }

    impl CandidateBuilder for MacosAccessibilityAdapter {
        fn build(
            &self,
            goal: &str,
            observation: &Observation,
        ) -> Result<Vec<ActionCandidate>, AutomationError> {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AutomationError::Candidates("AX state unavailable".into()))?;
            Self::current(&state, observation)?;
            let mut ranked = Vec::new();
            let goal = goal.to_lowercase();
            for meta in &state.metadata {
                if !meta.node.enabled || !meta.node.visible {
                    continue;
                }
                for action in &meta.node.supported_actions {
                    let locator = SemanticLocator {
                        app_id: observation.app.id.clone(),
                        window_id: None,
                        window_title: None,
                        role: Some(meta.node.role.clone()),
                        automation_id: meta.identifier.clone(),
                        accessible_name: meta.raw_name.clone(),
                        ancestor_chain: meta.ancestors.clone(),
                        supported_action: Some(action.clone()),
                        ordinal_hint: None,
                    };
                    if state
                        .metadata
                        .iter()
                        .filter(|m| matches(m, &locator, action))
                        .count()
                        != 1
                    {
                        continue;
                    }
                    let name = meta.node.name.as_deref().unwrap_or_default().to_lowercase();
                    let relevance = i32::from(!name.is_empty() && goal.contains(&name)) * 120
                        + i32::from(meta.node.focused) * 30
                        + i32::from(matches!(
                            action,
                            NativeAction::SetValue | NativeAction::Invoke
                        )) * 18;
                    ranked.push((relevance, meta.clone(), action.clone()));
                }
            }
            ranked.sort_by_key(|item| std::cmp::Reverse(item.0));
            ranked.truncate(37);
            state.actions.clear();
            let mut result = Vec::new();
            for (_, meta, action) in ranked {
                result.push(Self::bind(&mut state, observation, &meta, action)?);
            }
            for (kind, description) in [
                (CandidateKind::Done, "The bounded goal is complete"),
                (
                    CandidateKind::AskUser,
                    "Ask the user for information needed to continue",
                ),
                (
                    CandidateKind::CannotProceed,
                    "No offered semantic action can advance the goal",
                ),
            ] {
                result.push(ActionCandidate {
                    id: format!("ax:{}", uuid::Uuid::new_v4().simple()),
                    observation_revision: observation.revision,
                    target: None,
                    kind,
                    public_description: description.into(),
                    local_risk: RiskClass::ReadOnly,
                    preconditions: vec![],
                    expected_effects: vec![],
                });
            }
            Ok(result)
        }

        fn semantic_locator(&self, candidate: &ActionCandidate) -> Option<SemanticLocator> {
            let state = self.state.lock().ok()?;
            let bound = state.actions.get(&candidate.id)?;
            (bound.candidate == *candidate).then(|| bound.locator.clone())
        }

        fn rebuild_semantic_candidate(
            &self,
            locator: &SemanticLocator,
            action: NativeAction,
            observation: &Observation,
        ) -> Result<ActionCandidate, AutomationError> {
            kind(&action)?;
            native::validate_locator_window(locator, observation)?;
            if locator
                .supported_action
                .as_ref()
                .is_some_and(|a| a != &action)
            {
                return Err(AutomationError::Candidates(
                    "saved action does not match AX locator capability".into(),
                ));
            }
            let mut state = self
                .state
                .lock()
                .map_err(|_| AutomationError::Candidates("AX state unavailable".into()))?;
            Self::current(&state, observation)?;
            let found: Vec<_> = state
                .metadata
                .iter()
                .filter(|m| matches(m, locator, &action))
                .cloned()
                .collect();
            let [meta] = found.as_slice() else {
                return Err(AutomationError::Candidates("AX locator is missing or ambiguous; refresh the observation or specify ancestor context".into()));
            };
            Self::bind(&mut state, observation, meta, action)
        }
    }

    #[async_trait]
    impl ComputerExecutor for MacosAccessibilityAdapter {
        async fn execute(
            &self,
            fresh: &Observation,
            action: &ActionCandidate,
            input: &ExecutionInput,
        ) -> Result<ActionReceipt, AutomationError> {
            let bound = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| AutomationError::Execution("AX state unavailable".into()))?;
                Self::current(&state, fresh)?;
                let bound = state
                    .actions
                    .get(&action.id)
                    .filter(|bound| &bound.candidate == action)
                    .ok_or_else(|| {
                        AutomationError::Execution(
                            "candidate was not created by this AX adapter or was modified".into(),
                        )
                    })?;
                bound.clone()
            };
            let (reply, receive) = oneshot::channel();
            self.worker
                .try_send(Request::Execute(
                    bound,
                    input.clone(),
                    Instant::now() + Duration::from_secs(5),
                    reply,
                ))
                .map_err(|_| {
                    AutomationError::Execution("AX worker is busy or unavailable".into())
                })?;
            tokio::time::timeout(Duration::from_secs(6), receive)
                .await
                .map_err(|_| {
                    AutomationError::Execution(
                        "AX action timed out; reobserve before retrying".into(),
                    )
                })?
                .map_err(|_| AutomationError::Execution("AX worker stopped".into()))?
        }
    }
}

#[cfg(target_os = "macos")]
pub use platform::MacosAccessibilityAdapter;

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct MacosAccessibilityAdapter;

#[cfg(not(target_os = "macos"))]
#[async_trait]
impl ComputerObserver for MacosAccessibilityAdapter {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            supported: vec![],
            accessibility_permission: false,
        }
    }
    async fn observe(&self, _: &ObservationScope) -> Result<Observation, AutomationError> {
        Err(AutomationError::Observation(
            "macOS Accessibility requires macOS".into(),
        ))
    }
}

#[cfg(not(target_os = "macos"))]
impl CandidateBuilder for MacosAccessibilityAdapter {
    fn build(&self, _: &str, _: &Observation) -> Result<Vec<ActionCandidate>, AutomationError> {
        Err(AutomationError::Candidates(
            "macOS Accessibility requires macOS".into(),
        ))
    }
}

#[cfg(not(target_os = "macos"))]
#[async_trait]
impl ComputerExecutor for MacosAccessibilityAdapter {
    async fn execute(
        &self,
        _: &Observation,
        _: &ActionCandidate,
        _: &ExecutionInput,
    ) -> Result<ActionReceipt, AutomationError> {
        Err(AutomationError::Execution(
            "macOS Accessibility requires macOS".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::semantic::*;
    use super::*;

    fn metadata(container: &str) -> Metadata {
        Metadata {
            node: UiNode {
                opaque_id: "axe:button".into(),
                semantic_key: Some("ax:button".into()),
                role: UiRole::Button,
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
                supported_actions: vec![NativeAction::Invoke],
            },
            identifier: Some("open".into()),
            raw_name: Some("Open".into()),
            ancestors: vec![SemanticContext {
                role: Some(UiRole::ListItem),
                automation_id: Some(container.into()),
                accessible_name: None,
            }],
        }
    }

    #[test]
    fn capabilities_are_advertised_only_when_supported_by_ax() {
        assert!(actions(
            &UiRole::Button,
            false,
            &[],
            false,
            false,
            false,
            false,
            None
        )
        .is_empty());
        assert_eq!(
            actions(
                &UiRole::CheckBox,
                false,
                &["AXPress".into()],
                false,
                false,
                false,
                false,
                None
            ),
            vec![NativeAction::Toggle]
        );
        assert_eq!(
            actions(
                &UiRole::RadioButton,
                false,
                &["AXPress".into()],
                false,
                false,
                true,
                false,
                None
            ),
            vec![NativeAction::Select]
        );
        assert_eq!(
            actions(
                &UiRole::ListItem,
                false,
                &[],
                false,
                false,
                false,
                true,
                Some(true)
            ),
            vec![NativeAction::Collapse]
        );
        assert_eq!(
            actions(
                &UiRole::ListItem,
                false,
                &[],
                false,
                false,
                false,
                true,
                Some(false)
            ),
            vec![NativeAction::Expand]
        );
    }

    #[test]
    fn secure_fields_and_read_only_controls_never_offer_ordinary_set_value() {
        assert_eq!(
            actions(
                &UiRole::TextField,
                true,
                &[],
                true,
                true,
                false,
                false,
                None
            ),
            vec![NativeAction::Focus]
        );
        assert!(actions(
            &UiRole::TextField,
            false,
            &[],
            false,
            false,
            false,
            false,
            None
        )
        .is_empty());
        assert_eq!(
            actions(
                &UiRole::TextField,
                false,
                &[],
                false,
                true,
                false,
                false,
                None
            ),
            vec![NativeAction::SetValue]
        );
        assert!(actions(&UiRole::Other, false, &[], false, true, false, false, None).is_empty());
    }

    #[test]
    fn repeated_controls_resolve_using_semantic_ancestry_not_ordinal() {
        let first = metadata("row-first");
        let second = metadata("row-second");
        let locator = SemanticLocator {
            app_id: "com.apple.finder".into(),
            window_id: None,
            window_title: None,
            role: Some(UiRole::Button),
            automation_id: first.identifier.clone(),
            accessible_name: first.raw_name.clone(),
            ancestor_chain: first.ancestors.clone(),
            supported_action: Some(NativeAction::Invoke),
            ordinal_hint: Some(0),
        };
        assert!(matches(&first, &locator, &NativeAction::Invoke));
        assert!(!matches(&second, &locator, &NativeAction::Invoke));
        assert!(!matches(&first, &locator, &NativeAction::SetValue));
        let mut disabled = first.clone();
        disabled.node.enabled = false;
        assert!(!matches(&disabled, &locator, &NativeAction::Invoke));
    }

    #[test]
    fn public_names_redact_secrets_and_bound_length() {
        for value in [
            "person@example.org",
            "Bearer abc",
            "apikey_test",
            "Password",
            "123456789",
        ] {
            assert_eq!(redact(value), "redacted control");
        }
        assert_eq!(redact(&"中".repeat(100)).chars().count(), 80);
        assert_ne!(hash("value one"), hash("value two"));
        assert_eq!(hash("row context"), hash("row context"));
    }

    #[test]
    fn native_roles_and_predicates_match_shared_verifier_contract() {
        assert_eq!(role("AXTextArea"), UiRole::TextField);
        assert_eq!(role("AXRow"), UiRole::ListItem);
        assert_eq!(role("AXUnknown"), UiRole::Other);
        for (action, effect) in [
            (NativeAction::Invoke, "invoke_target_changed_or_disappeared"),
            (NativeAction::Toggle, "target_toggle_changed"),
            (NativeAction::Select, "target_selected"),
            (NativeAction::Focus, "target_focused"),
            (NativeAction::SetValue, "target_value_changed"),
        ] {
            assert!(kind(&action).is_ok());
            let predicates = effects(&action, "target");
            assert_eq!(predicates[0].name, effect);
            assert_eq!(predicates[0].arguments.get("target").unwrap(), "target");
        }
        assert!(kind(&NativeAction::Scroll).is_err());
        assert!(effects(&NativeAction::Scroll, "target").is_empty());
    }

    #[test]
    fn local_risk_does_not_require_confirmation_for_ordinary_workflow_steps() {
        assert_eq!(
            risk(&NativeAction::Invoke, Some("Open")),
            RiskClass::Reversible
        );
        assert_eq!(
            risk(&NativeAction::Invoke, Some("发送")),
            RiskClass::ExternalCommit
        );
        assert_eq!(
            risk(&NativeAction::Invoke, Some("永久删除")),
            RiskClass::DestructiveCritical
        );
        assert_eq!(risk(&NativeAction::SetValue, None), RiskClass::BoundedWrite);
    }
}
