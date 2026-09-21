//! Windows UI Automation adapter for bounded semantic desktop actions.
//!
//! Native window handles, process identifiers, COM elements and geometry stay
//! inside this module. Callers receive only redacted semantic observations and
//! may execute only candidate ids created from the latest observation.

use super::runner::{CandidateBuilder, ComputerExecutor, ComputerObserver};
use super::types::*;

const DEFAULT_MAX_ELEMENTS: usize = 200;
const MAX_DECISION_ACTIONS: usize = 37;

#[cfg(windows)]
mod platform {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;
    use windows::core::{Interface, BSTR};
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
        IUIAutomationInvokePattern, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
        IUIAutomationValuePattern, TreeScope_Descendants, UIA_ButtonControlTypeId,
        UIA_CheckBoxControlTypeId, UIA_DataItemControlTypeId, UIA_DocumentControlTypeId,
        UIA_EditControlTypeId, UIA_ExpandCollapsePatternId, UIA_HyperlinkControlTypeId,
        UIA_InvokePatternId, UIA_ListControlTypeId, UIA_ListItemControlTypeId,
        UIA_MenuControlTypeId, UIA_MenuItemControlTypeId, UIA_RadioButtonControlTypeId,
        UIA_SelectionItemPatternId, UIA_TabControlTypeId, UIA_TabItemControlTypeId,
        UIA_TextControlTypeId, UIA_TogglePatternId, UIA_TreeControlTypeId,
        UIA_TreeItemControlTypeId, UIA_ValuePatternId, UIA_WindowControlTypeId, UIA_CONTROLTYPE_ID,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct NodeMetadata {
        opaque_id: String,
        role: UiRole,
        name: Option<String>,
        automation_id: Option<String>,
        enabled: bool,
        visible: bool,
        focused: bool,
        secure: bool,
        value_fingerprint: Option<u64>,
        toggled: Option<bool>,
        selected: Option<bool>,
        expanded: Option<bool>,
        supported_actions: Vec<NativeAction>,
    }

    impl NodeMetadata {
        fn public_node(&self) -> UiNode {
            UiNode {
                opaque_id: self.opaque_id.clone(),
                role: self.role.clone(),
                // Password controls may expose provider-specific labels or
                // values through Name. Keep the raw locator local, but never
                // publish it to either decision provider.
                name: if self.secure { None } else { self.name.clone() },
                // UI values can contain private document content. The first
                // UIA slice intentionally exposes names, not arbitrary values.
                short_value: None,
                enabled: self.enabled,
                visible: self.visible,
                focused: self.focused,
                secure: self.secure,
                toggled: self.toggled,
                selected: self.selected,
                expanded: self.expanded,
                value_fingerprint: self
                    .value_fingerprint
                    .map(|value| format!("value:{value:016x}")),
                supported_actions: self.supported_actions.clone(),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct InternalLocator {
        app_id: String,
        window_id: String,
        target_opaque_id: String,
        role: UiRole,
        automation_id: Option<String>,
        accessible_name: Option<String>,
        action: NativeAction,
        risk: RiskClass,
        ordinal: usize,
    }

    struct NativeNode {
        metadata: NodeMetadata,
        element: IUIAutomationElement,
    }

    struct NativeSnapshot {
        app: AppIdentity,
        window: WindowIdentity,
        nodes: Vec<NativeNode>,
        fingerprint: String,
    }

    #[derive(Default)]
    struct AdapterState {
        last_fingerprint: Option<String>,
        revision: u64,
        metadata_by_node: HashMap<String, NodeMetadata>,
        locators_by_candidate: HashMap<String, InternalLocator>,
    }

    /// UIA-first adapter for the current Windows foreground window.
    pub struct WindowsUiaAdapter {
        max_elements: usize,
        state: Mutex<AdapterState>,
    }

    impl Default for WindowsUiaAdapter {
        fn default() -> Self {
            Self::new(DEFAULT_MAX_ELEMENTS)
        }
    }

    impl WindowsUiaAdapter {
        pub fn new(max_elements: usize) -> Self {
            Self {
                max_elements: max_elements.clamp(1, DEFAULT_MAX_ELEMENTS),
                state: Mutex::new(AdapterState::default()),
            }
        }

        fn capture_observation(
            &self,
            scope: &ObservationScope,
        ) -> Result<Observation, AutomationError> {
            let snapshot = capture_native(self.max_elements)?;
            validate_scope(scope, &snapshot)?;
            self.observation_from_snapshot(snapshot)
        }

        fn observation_from_snapshot(
            &self,
            snapshot: NativeSnapshot,
        ) -> Result<Observation, AutomationError> {
            let mut state = self
                .state
                .lock()
                .map_err(|_| AutomationError::Observation("UIA state lock was poisoned".into()))?;
            if state.last_fingerprint.as_deref() != Some(snapshot.fingerprint.as_str()) {
                state.revision = state.revision.saturating_add(1).max(1);
                state.last_fingerprint = Some(snapshot.fingerprint.clone());
            }
            state.metadata_by_node = snapshot
                .nodes
                .iter()
                .map(|node| (node.metadata.opaque_id.clone(), node.metadata.clone()))
                .collect();

            Ok(Observation {
                revision: state.revision,
                fingerprint: snapshot.fingerprint,
                app: snapshot.app,
                window: snapshot.window,
                nodes: snapshot
                    .nodes
                    .into_iter()
                    .map(|node| node.metadata.public_node())
                    .collect(),
                captured_at_ms: now_ms(),
            })
        }

        fn locator_for(
            &self,
            observation: &Observation,
            node: &UiNode,
            action: NativeAction,
        ) -> Result<InternalLocator, AutomationError> {
            let state = self
                .state
                .lock()
                .map_err(|_| AutomationError::Candidates("UIA state lock was poisoned".into()))?;
            let metadata = state.metadata_by_node.get(&node.opaque_id).ok_or_else(|| {
                AutomationError::Candidates("observation metadata is no longer available".into())
            })?;
            let ordinal = observation
                .nodes
                .iter()
                .take_while(|current| current.opaque_id != node.opaque_id)
                .filter_map(|current| state.metadata_by_node.get(&current.opaque_id))
                .filter(|current| semantic_match(current, metadata, &action))
                .count();
            Ok(InternalLocator {
                app_id: observation.app.id.clone(),
                window_id: observation.window.id.clone(),
                target_opaque_id: node.opaque_id.clone(),
                role: metadata.role.clone(),
                automation_id: metadata.automation_id.clone(),
                accessible_name: metadata.name.clone(),
                risk: classify_risk(&action, metadata.name.as_deref()),
                action,
                ordinal,
            })
        }

        fn execute_candidate(
            &self,
            _fresh: &Observation,
            action: &ActionCandidate,
            input: &ExecutionInput,
        ) -> Result<ActionReceipt, AutomationError> {
            let locator = self
                .state
                .lock()
                .map_err(|_| AutomationError::Execution("UIA state lock was poisoned".into()))?
                .locators_by_candidate
                .get(&action.id)
                .cloned()
                .ok_or_else(|| {
                    AutomationError::Execution(
                        "candidate was not created by this UIA adapter".into(),
                    )
                })?;
            if action.target.as_deref() != Some(locator.target_opaque_id.as_str())
                || action.kind != candidate_kind(&locator.action)
                || action.local_risk != locator.risk
            {
                return Err(AutomationError::Execution(
                    "candidate fields do not match the locally constructed UIA action".into(),
                ));
            }

            // Re-read the foreground tree immediately before dispatch. No COM
            // element or HWND from a previous observation is ever reused.
            let snapshot = capture_native(self.max_elements)?;
            if snapshot.app.id != locator.app_id || snapshot.window.id != locator.window_id {
                return Err(AutomationError::Execution(
                    "foreground UI changed before native dispatch".into(),
                ));
            }
            let element = resolve_unique(&snapshot.nodes, &locator)?;
            dispatch(element, &locator.action, input)?;
            Ok(ActionReceipt {
                candidate_id: action.id.clone(),
                dispatched: true,
                detail: Some(format!("Windows UIA {:?} dispatched", locator.action)),
            })
        }
    }

    #[async_trait]
    impl ComputerObserver for WindowsUiaAdapter {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                supported: vec![
                    PlatformCapability::ReadSemanticTree,
                    PlatformCapability::NativeAction,
                    PlatformCapability::WriteValue,
                ],
                accessibility_permission: true,
            }
        }

        async fn observe(&self, scope: &ObservationScope) -> Result<Observation, AutomationError> {
            self.capture_observation(scope)
        }
    }

    impl CandidateBuilder for WindowsUiaAdapter {
        fn build(
            &self,
            goal: &str,
            observation: &Observation,
        ) -> Result<Vec<ActionCandidate>, AutomationError> {
            let mut ranked = Vec::new();

            for node in &observation.nodes {
                if !node.enabled || !node.visible {
                    continue;
                }
                for native_action in &node.supported_actions {
                    let locator = self.locator_for(observation, node, native_action.clone())?;
                    let id = format!("uia:{}", Uuid::new_v4().simple());
                    let kind = candidate_kind(native_action);
                    let candidate = ActionCandidate {
                        id: id.clone(),
                        observation_revision: observation.revision,
                        target: Some(node.opaque_id.clone()),
                        kind,
                        public_description: describe_action(native_action, node),
                        local_risk: classify_risk(native_action, node.name.as_deref()),
                        preconditions: vec![predicate(
                            "semantic_element_exists",
                            [("target", node.opaque_id.as_str())],
                        )],
                        expected_effects: vec![predicate(
                            "ui_state_reobserved_after_action",
                            std::iter::empty::<(&str, &str)>(),
                        )],
                    };
                    ranked.push((
                        candidate_relevance(goal, node, native_action),
                        candidate,
                        locator,
                    ));
                }
            }
            ranked.sort_by_key(|item| std::cmp::Reverse(item.0));
            ranked.truncate(MAX_DECISION_ACTIONS);
            let mut candidate_locators = HashMap::new();
            let mut candidates = Vec::with_capacity(ranked.len() + 3);
            for (_, candidate, locator) in ranked {
                candidate_locators.insert(candidate.id.clone(), locator);
                candidates.push(candidate);
            }

            for (suffix, kind, description) in [
                ("done", CandidateKind::Done, "The bounded goal is complete"),
                (
                    "ask-user",
                    CandidateKind::AskUser,
                    "Ask the user for information needed to continue",
                ),
                (
                    "cannot-proceed",
                    CandidateKind::CannotProceed,
                    "No offered semantic action can advance the goal",
                ),
            ] {
                candidates.push(ActionCandidate {
                    id: format!("uia:{suffix}:{}", Uuid::new_v4().simple()),
                    observation_revision: observation.revision,
                    target: None,
                    kind,
                    public_description: description.into(),
                    local_risk: RiskClass::ReadOnly,
                    preconditions: vec![],
                    expected_effects: vec![],
                });
            }

            self.state
                .lock()
                .map_err(|_| AutomationError::Candidates("UIA state lock was poisoned".into()))?
                .locators_by_candidate = candidate_locators;
            Ok(candidates)
        }
    }

    #[async_trait]
    impl ComputerExecutor for WindowsUiaAdapter {
        async fn execute(
            &self,
            fresh: &Observation,
            action: &ActionCandidate,
            input: &ExecutionInput,
        ) -> Result<ActionReceipt, AutomationError> {
            self.execute_candidate(fresh, action, input)
        }
    }

    fn capture_native(max_elements: usize) -> Result<NativeSnapshot, AutomationError> {
        let _com = ComApartment::initialize()?;
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0 == 0 {
            return Err(AutomationError::Observation(
                "Windows has no foreground window".into(),
            ));
        }
        let title = window_title(hwnd);
        let window_class = window_class(hwnd);
        let automation: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| uia_observation_error("create UI Automation client", error))?
        };
        let root = unsafe {
            automation
                .ElementFromHandle(hwnd)
                .map_err(|error| uia_observation_error("read foreground window", error))?
        };
        let framework = unsafe { root.CurrentFrameworkId() }
            .ok()
            .map(|value| truncate(&value.to_string(), 64))
            .filter(|value| !value.is_empty());
        let root_name = element_text(unsafe { root.CurrentName() }.ok(), 128);
        let display_name = root_name
            .clone()
            .or_else(|| nonempty(title.clone()))
            .or_else(|| nonempty(window_class.clone()))
            .unwrap_or_else(|| "Windows application".into());
        let app_seed = format!(
            "{}|{}",
            framework.as_deref().unwrap_or("win32"),
            window_class.to_lowercase()
        );
        let app = AppIdentity {
            id: format!("windows-app:{:016x}", stable_hash(&app_seed)),
            display_name,
        };
        let window_title = nonempty(title)
            .or(root_name)
            .unwrap_or_else(|| "Untitled".into());
        let root_automation_id = element_text(unsafe { root.CurrentAutomationId() }.ok(), 256);
        let window = WindowIdentity {
            id: format!(
                "windows-window:{:016x}",
                stable_hash(&format!(
                    "{}|{}|{}",
                    app.id,
                    window_class,
                    root_automation_id.as_deref().unwrap_or_default()
                ))
            ),
            title: window_title,
        };

        let mut nodes = Vec::with_capacity(max_elements);
        nodes.push(native_node(root, 0)?);
        if nodes.len() < max_elements {
            let condition = unsafe {
                automation.ControlViewCondition().map_err(|error| {
                    uia_observation_error("create UIA control-view condition", error)
                })?
            };
            let elements = unsafe {
                nodes[0]
                    .element
                    .FindAll(TreeScope_Descendants, &condition)
                    .map_err(|error| uia_observation_error("enumerate UIA control tree", error))?
            };
            let count = unsafe { elements.Length() }
                .map_err(|error| uia_observation_error("read UIA element count", error))?;
            for index in 0..count.min((max_elements - 1) as i32) {
                let Ok(element) = (unsafe { elements.GetElement(index) }) else {
                    continue;
                };
                if let Ok(node) = native_node(element, index as usize + 1) {
                    nodes.push(node);
                }
            }
        }

        let fingerprint = fingerprint(&app, &window, &nodes);
        Ok(NativeSnapshot {
            app,
            window,
            nodes,
            fingerprint,
        })
    }

    fn native_node(
        element: IUIAutomationElement,
        index: usize,
    ) -> Result<NativeNode, AutomationError> {
        let control_type = unsafe { element.CurrentControlType() }
            .map_err(|error| uia_observation_error("read UIA control type", error))?;
        let name = element_text(unsafe { element.CurrentName() }.ok(), 256);
        let automation_id = element_text(unsafe { element.CurrentAutomationId() }.ok(), 256);
        let enabled = bool_or(unsafe { element.CurrentIsEnabled() }, false);
        let visible = !bool_or(unsafe { element.CurrentIsOffscreen() }, true);
        let focused = bool_or(unsafe { element.CurrentHasKeyboardFocus() }, false);
        let secure = bool_or(unsafe { element.CurrentIsPassword() }, false);
        let mut supported_actions = Vec::new();
        if supports_pattern::<IUIAutomationInvokePattern>(&element, UIA_InvokePatternId) {
            supported_actions.push(NativeAction::Invoke);
        }
        let toggle_pattern = unsafe {
            element.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
        }
        .ok();
        if toggle_pattern.is_some() {
            supported_actions.push(NativeAction::Toggle);
        }
        let selection_pattern = unsafe {
            element.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                UIA_SelectionItemPatternId,
            )
        }
        .ok();
        let selected = selection_pattern
            .as_ref()
            .and_then(|pattern| unsafe { pattern.CurrentIsSelected() }.ok())
            .map(|value| value.as_bool());
        if selection_pattern.is_some() && selected != Some(true) {
            supported_actions.push(NativeAction::Select);
        }
        let expand_pattern = unsafe {
            element.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                UIA_ExpandCollapsePatternId,
            )
        }
        .ok();
        let expanded = expand_pattern
            .as_ref()
            .and_then(|pattern| unsafe { pattern.CurrentExpandCollapseState() }.ok())
            .and_then(|state| {
                if state == windows::Win32::UI::Accessibility::ExpandCollapseState_Expanded {
                    Some(true)
                } else if state == windows::Win32::UI::Accessibility::ExpandCollapseState_Collapsed
                {
                    Some(false)
                } else {
                    None
                }
            });
        if expand_pattern.is_some() {
            match expanded {
                Some(true) => supported_actions.push(NativeAction::Collapse),
                Some(false) => supported_actions.push(NativeAction::Expand),
                None => {
                    supported_actions.push(NativeAction::Expand);
                    supported_actions.push(NativeAction::Collapse);
                }
            }
        }
        let value_pattern =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
                .ok();
        let writable_value = value_pattern
            .as_ref()
            .and_then(|pattern| unsafe { pattern.CurrentIsReadOnly() }.ok())
            .is_some_and(|value| !value.as_bool());
        if writable_value && !secure {
            supported_actions.push(NativeAction::SetValue);
        }
        let value_fingerprint = if secure {
            None
        } else {
            value_pattern
                .as_ref()
                .and_then(|pattern| unsafe { pattern.CurrentValue() }.ok())
                .map(|value| stable_hash(&value.to_string()))
        };
        let toggled = toggle_pattern
            .as_ref()
            .and_then(|pattern| unsafe { pattern.CurrentToggleState() }.ok())
            .map(|state| state == windows::Win32::UI::Accessibility::ToggleState_On);
        if bool_or(unsafe { element.CurrentIsKeyboardFocusable() }, false) {
            supported_actions.push(NativeAction::Focus);
        }
        let role = role_for(control_type);
        let opaque_id = format!(
            "uie:{:016x}",
            stable_hash(&format!(
                "{index}|{role:?}|{}|{}",
                name.as_deref().unwrap_or_default(),
                automation_id.as_deref().unwrap_or_default()
            ))
        );
        Ok(NativeNode {
            metadata: NodeMetadata {
                opaque_id,
                role,
                name,
                automation_id,
                enabled,
                visible,
                focused,
                secure,
                value_fingerprint,
                toggled,
                selected,
                expanded,
                supported_actions,
            },
            element,
        })
    }

    fn supports_pattern<T: Interface>(
        element: &IUIAutomationElement,
        pattern: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> bool {
        unsafe { element.GetCurrentPatternAs::<T>(pattern) }.is_ok()
    }

    fn resolve_unique<'a>(
        nodes: &'a [NativeNode],
        locator: &InternalLocator,
    ) -> Result<&'a IUIAutomationElement, AutomationError> {
        let matches: Vec<_> = nodes
            .iter()
            .filter(|node| {
                node.metadata.role == locator.role
                    && node.metadata.automation_id == locator.automation_id
                    && node.metadata.name == locator.accessible_name
                    && node.metadata.supported_actions.contains(&locator.action)
            })
            .collect();
        matches
            .get(locator.ordinal)
            .map(|node| &node.element)
            .ok_or_else(|| {
                AutomationError::Execution(
                    "semantic UIA target is missing after fresh re-observation".into(),
                )
            })
    }

    fn dispatch(
        element: &IUIAutomationElement,
        action: &NativeAction,
        input: &ExecutionInput,
    ) -> Result<(), AutomationError> {
        let result = unsafe {
            match action {
                NativeAction::Invoke => element
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                    .and_then(|pattern| pattern.Invoke()),
                NativeAction::Toggle => element
                    .GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                    .and_then(|pattern| pattern.Toggle()),
                NativeAction::Select => element
                    .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                        UIA_SelectionItemPatternId,
                    )
                    .and_then(|pattern| pattern.Select()),
                NativeAction::Expand => element
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )
                    .and_then(|pattern| pattern.Expand()),
                NativeAction::Collapse => element
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )
                    .and_then(|pattern| pattern.Collapse()),
                NativeAction::Focus => element.SetFocus(),
                NativeAction::SetValue => {
                    let value = input.value.as_deref().ok_or_else(|| {
                        AutomationError::Execution(
                            "SetValue requires caller-provided text at dispatch time".into(),
                        )
                    })?;
                    let value = BSTR::from(value);
                    element
                        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                        .and_then(|pattern| pattern.SetValue(&value))
                }
                unsupported => {
                    return Err(AutomationError::Execution(format!(
                        "native UIA action {unsupported:?} is not supported by this slice"
                    )))
                }
            }
        };
        result.map_err(|error| AutomationError::Execution(format!("UIA dispatch failed: {error}")))
    }

    fn candidate_kind(action: &NativeAction) -> CandidateKind {
        match action {
            NativeAction::Invoke => CandidateKind::Invoke,
            NativeAction::Toggle => CandidateKind::Toggle,
            NativeAction::Select => CandidateKind::Select,
            NativeAction::Expand => CandidateKind::Expand,
            NativeAction::Collapse => CandidateKind::Collapse,
            NativeAction::Focus => CandidateKind::Focus,
            NativeAction::SetValue => CandidateKind::SetValue {
                slot_id: "value".into(),
            },
            _ => unreachable!("candidate builder filters to the supported UIA slice"),
        }
    }

    fn describe_action(action: &NativeAction, node: &UiNode) -> String {
        let name = node
            .name
            .as_deref()
            .map(redact_public_name)
            .unwrap_or_else(|| "unnamed control".into());
        format!("{action:?} {:?} '{name}'", node.role)
    }

    fn candidate_relevance(goal: &str, node: &UiNode, action: &NativeAction) -> i32 {
        let goal = goal.to_lowercase();
        let name = node.name.as_deref().unwrap_or_default().to_lowercase();
        let mut score = 0_i32;
        if !name.is_empty() {
            score += 20;
            if goal.contains(&name) {
                score += 120;
            }
            for token in goal.split(|ch: char| !ch.is_alphanumeric()) {
                if token.chars().count() >= 2 && name.contains(token) {
                    score += 35;
                }
            }
        }
        if node.focused {
            score += 30;
        }
        score += match action {
            NativeAction::Invoke | NativeAction::SetValue => 18,
            NativeAction::Toggle | NativeAction::Select => 14,
            NativeAction::Expand | NativeAction::Collapse => 10,
            NativeAction::Focus => 2,
            _ => 0,
        };
        score
    }

    fn redact_public_name(value: &str) -> String {
        let lower = value.to_lowercase();
        let has_long_digit_run = value
            .split(|ch: char| !ch.is_ascii_digit())
            .any(|part| part.len() >= 8);
        if value.contains('@')
            || has_long_digit_run
            || lower.contains("apikey_")
            || lower.contains("api_key")
            || lower.contains("bearer ")
            || lower.contains("password")
            || lower.contains("密码")
        {
            "redacted control".into()
        } else {
            truncate(value, 80)
        }
    }

    fn classify_risk(action: &NativeAction, name: Option<&str>) -> RiskClass {
        if matches!(action, NativeAction::SetValue) {
            return RiskClass::BoundedWrite;
        }
        if !matches!(action, NativeAction::Invoke) {
            return RiskClass::Reversible;
        }
        let name = name.unwrap_or_default().to_lowercase();
        if [
            "permanently delete",
            "永久删除",
            "purchase",
            "支付",
            "buy now",
            "grant permission",
            "security settings",
            "安全设置",
        ]
        .iter()
        .any(|keyword| name.contains(keyword))
        {
            RiskClass::DestructiveCritical
        } else if ["send", "发送", "publish", "发布", "submit", "提交"]
            .iter()
            .any(|keyword| name.contains(keyword))
        {
            RiskClass::ExternalCommit
        } else if ["save", "保存", "apply", "应用"]
            .iter()
            .any(|keyword| name.contains(keyword))
        {
            RiskClass::BoundedWrite
        } else {
            RiskClass::Reversible
        }
    }

    fn predicate<'a>(
        name: &str,
        arguments: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Predicate {
        Predicate {
            name: name.into(),
            arguments: arguments
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }

    fn semantic_match(a: &NodeMetadata, b: &NodeMetadata, action: &NativeAction) -> bool {
        a.role == b.role
            && a.name == b.name
            && a.automation_id == b.automation_id
            && a.supported_actions.contains(action)
    }

    fn validate_scope(
        scope: &ObservationScope,
        snapshot: &NativeSnapshot,
    ) -> Result<(), AutomationError> {
        if scope
            .app_id
            .as_ref()
            .is_some_and(|expected| expected != &snapshot.app.id)
        {
            return Err(AutomationError::Observation(
                "foreground application is outside the requested scope".into(),
            ));
        }
        if scope
            .window_id
            .as_ref()
            .is_some_and(|expected| expected != &snapshot.window.id)
        {
            return Err(AutomationError::Observation(
                "foreground window is outside the requested scope".into(),
            ));
        }
        if scope.subtree_id.is_some() {
            return Err(AutomationError::Observation(
                "subtree-scoped Windows UIA observation is not implemented yet".into(),
            ));
        }
        Ok(())
    }

    fn fingerprint(app: &AppIdentity, window: &WindowIdentity, nodes: &[NativeNode]) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        app.id.hash(&mut hasher);
        window.title.hash(&mut hasher);
        for node in nodes {
            std::mem::discriminant(&node.metadata.role).hash(&mut hasher);
            node.metadata.name.hash(&mut hasher);
            node.metadata.automation_id.hash(&mut hasher);
            node.metadata.enabled.hash(&mut hasher);
            node.metadata.visible.hash(&mut hasher);
            node.metadata.focused.hash(&mut hasher);
            node.metadata.secure.hash(&mut hasher);
            node.metadata.value_fingerprint.hash(&mut hasher);
            node.metadata.toggled.hash(&mut hasher);
            node.metadata.selected.hash(&mut hasher);
            node.metadata.expanded.hash(&mut hasher);
            for action in &node.metadata.supported_actions {
                std::mem::discriminant(action).hash(&mut hasher);
            }
        }
        format!("uia:{:016x}", hasher.finish())
    }

    fn role_for(control_type: UIA_CONTROLTYPE_ID) -> UiRole {
        match control_type {
            value if value == UIA_WindowControlTypeId => UiRole::Window,
            value if value == UIA_ButtonControlTypeId => UiRole::Button,
            value if value == UIA_EditControlTypeId => UiRole::TextField,
            value if value == UIA_CheckBoxControlTypeId => UiRole::CheckBox,
            value if value == UIA_RadioButtonControlTypeId => UiRole::RadioButton,
            value if value == UIA_ListControlTypeId => UiRole::List,
            value if value == UIA_ListItemControlTypeId => UiRole::ListItem,
            value if value == UIA_MenuControlTypeId => UiRole::Menu,
            value if value == UIA_MenuItemControlTypeId => UiRole::MenuItem,
            value if value == UIA_TabControlTypeId || value == UIA_TabItemControlTypeId => {
                UiRole::Tab
            }
            value if value == UIA_DocumentControlTypeId => UiRole::Document,
            // These controls remain actionable through their advertised UIA
            // patterns even though the shared role vocabulary is compact.
            value
                if value == UIA_TextControlTypeId
                    || value == UIA_HyperlinkControlTypeId
                    || value == UIA_TreeControlTypeId
                    || value == UIA_TreeItemControlTypeId
                    || value == UIA_DataItemControlTypeId =>
            {
                UiRole::Other
            }
            _ => UiRole::Other,
        }
    }

    fn bool_or(result: windows::core::Result<BOOL>, fallback: bool) -> bool {
        result.map(|value| value.as_bool()).unwrap_or(fallback)
    }

    fn element_text(value: Option<windows::core::BSTR>, max_chars: usize) -> Option<String> {
        value
            .map(|value| truncate(&value.to_string(), max_chars))
            .and_then(nonempty)
    }

    fn nonempty(value: String) -> Option<String> {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    fn truncate(value: &str, max_chars: usize) -> String {
        value.chars().take(max_chars).collect()
    }

    fn window_title(hwnd: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
        let mut buffer = vec![0_u16; length.saturating_add(1).max(1)];
        let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
        truncate(&String::from_utf16_lossy(&buffer[..copied]), 256)
    }

    fn window_class(hwnd: HWND) -> String {
        let mut buffer = vec![0_u16; 256];
        let copied = unsafe { GetClassNameW(hwnd, &mut buffer) }.max(0) as usize;
        truncate(&String::from_utf16_lossy(&buffer[..copied]), 128)
    }

    fn stable_hash(value: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    fn uia_observation_error(context: &str, error: windows::core::Error) -> AutomationError {
        AutomationError::Observation(format!("{context}: {error}"))
    }

    struct ComApartment(bool);

    impl ComApartment {
        fn initialize() -> Result<Self, AutomationError> {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if result.is_ok() {
                Ok(Self(true))
            } else {
                // RPC_E_CHANGED_MODE means this thread already has a usable
                // COM apartment with another concurrency model.
                const RPC_E_CHANGED_MODE: i32 = unchecked_hresult(0x80010106);
                if result.0 == RPC_E_CHANGED_MODE {
                    Ok(Self(false))
                } else {
                    Err(AutomationError::Observation(format!(
                        "initialize COM for UIA: {result:?}"
                    )))
                }
            }
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    const fn unchecked_hresult(value: u32) -> i32 {
        value as i32
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use async_trait::async_trait;

    /// Explicit non-Windows stub. macOS and Linux adapters will use their
    /// native accessibility APIs rather than pretending UIA is available.
    pub struct WindowsUiaAdapter {
        _max_elements: usize,
    }

    impl Default for WindowsUiaAdapter {
        fn default() -> Self {
            Self::new(DEFAULT_MAX_ELEMENTS)
        }
    }

    impl WindowsUiaAdapter {
        pub fn new(max_elements: usize) -> Self {
            Self {
                _max_elements: max_elements.clamp(1, DEFAULT_MAX_ELEMENTS),
            }
        }
    }

    #[async_trait]
    impl ComputerObserver for WindowsUiaAdapter {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                supported: vec![],
                accessibility_permission: false,
            }
        }

        async fn observe(&self, _scope: &ObservationScope) -> Result<Observation, AutomationError> {
            Err(AutomationError::Observation(
                "Windows UI Automation is unsupported on this platform".into(),
            ))
        }
    }

    impl CandidateBuilder for WindowsUiaAdapter {
        fn build(
            &self,
            _goal: &str,
            _observation: &Observation,
        ) -> Result<Vec<ActionCandidate>, AutomationError> {
            Err(AutomationError::Candidates(
                "Windows UI Automation is unsupported on this platform".into(),
            ))
        }
    }

    #[async_trait]
    impl ComputerExecutor for WindowsUiaAdapter {
        async fn execute(
            &self,
            _fresh: &Observation,
            _action: &ActionCandidate,
            _input: &ExecutionInput,
        ) -> Result<ActionReceipt, AutomationError> {
            Err(AutomationError::Execution(
                "Windows UI Automation is unsupported on this platform".into(),
            ))
        }
    }
}

pub use platform::WindowsUiaAdapter;
