//! Editor debug sessions use the production executor with frozen, memory-only definitions.
use super::{
    store::WorkflowStore,
    trace::TraceRecorder,
    types::{Action, InputSpec, Step, Workflow},
    WorkflowEngine,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

tokio::task_local! { pub static CURRENT: Arc<DebugSession>; }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DebugMode {
    Node,
    Through,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DebugRequest {
    pub workflow_id: String,
    pub steps: Vec<Step>,
    pub inputs: Option<Vec<InputSpec>>,
    pub selected_step_id: String,
    pub mode: DebugMode,
    #[serde(default)]
    pub variables: HashMap<String, Value>,
    #[serde(default)]
    pub runtime_inputs: HashMap<String, Value>,
    #[serde(default)]
    pub use_retry_policy: bool,
    /// Provenance only; history values are explicitly selected and supplied by the editor.
    pub source: Option<Value>,
}

pub struct DebugSession {
    pub run_id: String,
    pub workflow_id: String,
    pub store: WorkflowStore,
    pub recorder: Arc<TraceRecorder>,
    pub variables: HashMap<String, Value>,
    pub runtime_inputs: HashMap<String, Value>,
    pub selected_step_id: String,
    pub mode: DebugMode,
    pub use_retry_policy: bool,
    pub cancelled: Arc<AtomicBool>,
    pub cancel_notify: tokio::sync::Notify,
    breakpoint_consumed: AtomicBool,
    ticks: AtomicU64,
}

pub fn current() -> Option<Arc<DebugSession>> {
    CURRENT.try_with(Arc::clone).ok()
}

impl DebugSession {
    pub fn target_reached(&self) -> bool {
        self.breakpoint_consumed.load(Ordering::Relaxed)
    }
    pub async fn cancelled(&self) {
        loop {
            let notified = self.cancel_notify.notified();
            if self.cancelled.load(Ordering::Relaxed) {
                return;
            }
            notified.await;
        }
    }
    pub fn check_budget(&self) -> crate::Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(crate::NuphusError::agent("debug_cancelled"));
        }
        if self.ticks.fetch_add(1, Ordering::Relaxed) >= 10_000 {
            return Err(crate::NuphusError::agent("debug_step_budget_exceeded"));
        }
        Ok(())
    }

    pub fn should_break(&self, workflow_id: &str, step_id: &str) -> bool {
        self.mode == DebugMode::Through
            && workflow_id == self.workflow_id
            && step_id == self.selected_step_id
            && !self.breakpoint_consumed.swap(true, Ordering::Relaxed)
    }
}

fn find_step<'a>(steps: &'a [Step], id: &str) -> Option<&'a Step> {
    for step in steps {
        if step.id == id {
            return Some(step);
        }
        let found = match &step.action {
            Action::Seq { seq } => find_step(seq, id),
            Action::Loop { def } => find_step(&def.steps, id),
            Action::If { def } => {
                find_step(&def.then, id).or_else(|| find_step(&def.else_branch, id))
            }
            Action::Wait { auto, .. } => find_step(auto, id),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

impl WorkflowEngine {
    /// Validate and freeze before spawn, making malformed drafts a synchronous API error.
    pub async fn prepare_debug(&self, request: DebugRequest) -> Result<Arc<DebugSession>, String> {
        let mut draft = self
            .store
            .get(&request.workflow_id)
            .await
            .ok_or("workflow_not_found")?;
        draft.steps = request.steps;
        if let Some(inputs) = request.inputs {
            draft.inputs = inputs;
        }
        draft.run_history.clear();
        draft.dry_run = false;
        draft.timeout_secs = Some(draft.timeout_secs.unwrap_or(300).min(300));
        let selected = find_step(&draft.steps, &request.selected_step_id)
            .ok_or("debug_step_not_found")?
            .clone();
        let frozen_definition = draft.clone();
        if request.mode == DebugMode::Node {
            for spec in &mut draft.inputs {
                if !super::references::uses_input(&selected, &spec.name) {
                    spec.required = false;
                }
            }
            draft.steps = vec![selected];
        }
        let mut runtime_inputs = request.runtime_inputs;
        // Historical/manual input variables can satisfy input declarations; explicit form values win.
        for spec in &draft.inputs {
            if !runtime_inputs.contains_key(&spec.name) {
                if let Some(value) = request
                    .variables
                    .get("inputs")
                    .and_then(|v| v.get(&spec.name))
                    .or_else(|| request.variables.get(&spec.name))
                {
                    runtime_inputs.insert(spec.name.clone(), value.clone());
                }
            }
        }
        let declared = super::inputs::resolve_declared_inputs(&draft.inputs, &runtime_inputs)
            .map_err(|e| e.to_string())?;
        let mut sensitive: Vec<String> = draft
            .inputs
            .iter()
            .filter(|spec| spec.sensitive)
            .filter_map(|spec| declared.get(&spec.name))
            .flat_map(|value| {
                let mut forms = vec![value.to_string()];
                if let Some(text) = value.as_str() {
                    forms.push(text.to_string());
                }
                forms
            })
            .collect();
        fn calls(steps: &[Step], out: &mut Vec<String>) {
            for step in steps {
                match &step.action {
                    Action::Call { call, .. } => out.push(call.clone()),
                    Action::Seq { seq } => calls(seq, out),
                    Action::Loop { def } => calls(&def.steps, out),
                    Action::If { def } => {
                        calls(&def.then, out);
                        calls(&def.else_branch, out);
                    }
                    Action::Wait { auto, .. } => calls(auto, out),
                    _ => {}
                }
            }
        }
        let mut definitions: Vec<Workflow> = Vec::new();
        let mut visited = std::collections::HashSet::from([draft.id.clone()]);
        let mut pending = Vec::new();
        calls(&draft.steps, &mut pending);
        while let Some(id) = pending.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if let Some(mut wf) = self.store.get(&id).await {
                wf.run_history.clear();
                calls(&wf.steps, &mut pending);
                definitions.push(wf);
            }
        }
        definitions.push(draft.clone());
        for wf in &definitions {
            for spec in wf.inputs.iter().filter(|spec| spec.sensitive) {
                if let Some(value) = &spec.default {
                    sensitive.push(value.to_string());
                    if let Some(value) = value.as_str() {
                        sensitive.push(value.into());
                    }
                }
            }
        }
        let snapshots = serde_json::to_value(&definitions).map_err(|error| error.to_string())?;
        let store = WorkflowStore::frozen(self.store.root().to_path_buf(), definitions);
        let schemas = self.tools().map(|tools| tools.get_schemas());
        let report = match schemas.as_deref() {
            Some(schemas) => {
                super::compiler::Compiler::validate_workflow_with_tools(&draft, schemas)
            }
            None => super::compiler::Compiler::validate_workflow(&draft),
        };
        let mut errors = report.errors;
        errors.extend(super::compiler::Compiler::validate_calls(&draft, &store).await);
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        let run_id = uuid::Uuid::new_v4().to_string();
        let recorder = TraceRecorder::create(
            self.store.root(),
            &frozen_definition,
            &run_id,
            true,
            sensitive,
        )
        .await?;
        recorder.snapshots(&snapshots).await?;
        recorder
            .provenance(serde_json::json!({
                "mode":request.mode, "selected_step_id":request.selected_step_id,
                "source":request.source, "variables":request.variables,
                "runtime_inputs":runtime_inputs, "use_retry_policy":request.use_retry_policy,
                "step_budget":10000,"timeout_secs":draft.timeout_secs
            }))
            .await?;
        let mut variables = request.variables;
        if !variables.contains_key("params") {
            if let Ok(data) =
                tokio::fs::read(self.store.workflow_dir(&draft.id).join("params.json")).await
            {
                if let Ok(params) = serde_json::from_slice::<Value>(&data) {
                    variables.insert("params".into(), params);
                }
            }
        }
        let session = Arc::new(DebugSession {
            run_id: run_id.clone(),
            workflow_id: draft.id,
            store,
            recorder,
            variables,
            runtime_inputs,
            selected_step_id: request.selected_step_id,
            mode: request.mode,
            use_retry_policy: request.use_retry_policy,
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_notify: tokio::sync::Notify::new(),
            breakpoint_consumed: AtomicBool::new(false),
            ticks: AtomicU64::new(0),
        });
        self.debug_sessions
            .write()
            .await
            .insert(run_id, session.clone());
        Ok(session)
    }

    pub async fn debug_control(
        &self,
        workflow_id: &str,
        run_id: &str,
        action: &str,
    ) -> Result<(), String> {
        let sessions = self.debug_sessions.read().await;
        let session = sessions
            .get(run_id)
            .filter(|session| session.workflow_id == workflow_id)
            .ok_or("debug_session_not_active")?;
        match action {
            "pause" => self.executor.pause(workflow_id).await,
            "resume" => self.executor.resume(workflow_id).await,
            "cancel" => {
                session.cancelled.store(true, Ordering::Relaxed);
                session.cancel_notify.notify_one();
                self.executor.cancel(workflow_id).await;
            }
            _ => return Err("invalid_debug_control".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
