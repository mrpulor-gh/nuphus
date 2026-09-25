//! Local execution evidence. Large values are stored per invocation, not sent
//! through progress events or embedded in the workflow definition.
use super::types::{Step, Workflow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

#[cfg(test)]
mod tests;

tokio::task_local! { pub static CURRENT: Arc<TraceRecorder>; }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationSummary {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub workflow_id: String,
    pub step_id: String,
    pub step_name: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvocationTrace {
    #[serde(flatten)]
    pub summary: InvocationSummary,
    pub definition: Value,
    pub variables_before: HashMap<String, Value>,
    pub variables_after: HashMap<String, Value>,
    pub inputs: Value,
    pub output: Option<String>,
    pub error: Option<String>,
    pub attempts: Vec<Value>,
    /// Only native tool evidence or an explicit assert result; None is unverified.
    pub verification: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTrace {
    pub version: u32,
    pub run_id: String,
    pub workflow_id: String,
    pub debug: bool,
    pub revision: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String,
    pub invocations: Vec<InvocationSummary>,
    pub storage_error: Option<String>,
    #[serde(default)]
    pub source: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

struct TraceState {
    header: RunTrace,
    active: HashMap<u64, InvocationTrace>,
    stack: Vec<u64>,
}

pub struct TraceRecorder {
    pub workflow_id: String,
    directory: PathBuf,
    state: Mutex<TraceState>,
    sensitive: std::sync::RwLock<Vec<String>>,
}

pub fn valid_segment(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', ':', '\0'])
}

pub fn run_directory(
    root: &Path,
    workflow_id: &str,
    run_id: &str,
    debug: bool,
) -> Result<PathBuf, String> {
    if !valid_segment(workflow_id) || uuid::Uuid::parse_str(run_id).is_err() {
        return Err("invalid_run_id".into());
    }
    Ok(root
        .join(".editor-runs")
        .join(workflow_id)
        .join(if debug { "debug" } else { "normal" })
        .join(run_id))
}

fn redact(value: &mut Value, sensitive: &[String]) {
    match value {
        Value::String(text) => {
            for secret in sensitive.iter().filter(|secret| !secret.is_empty()) {
                *text = text.replace(secret, "[redacted]");
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|value| redact(value, sensitive)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| redact(value, sensitive)),
        other => {
            let encoded = other.to_string();
            if sensitive
                .iter()
                .any(|secret| !secret.is_empty() && secret == &encoded)
            {
                *other = Value::String("[redacted]".into());
            }
        }
    }
}

async fn write_json(
    path: &Path,
    value: &impl Serialize,
    sensitive: &[String],
) -> Result<(), String> {
    let mut value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    // Secret literals may be short (e.g. "1"). Never corrupt routing identifiers,
    // timestamps or status by applying payload redaction to the record envelope.
    if value.get("invocations").is_some() && value.get("run_id").is_some() {
        for key in ["source", "error", "storage_error"] {
            if let Some(field) = value.get_mut(key) {
                redact(field, sensitive);
            }
        }
        if let Some(invocations) = value.get_mut("invocations").and_then(Value::as_array_mut) {
            for invocation in invocations {
                if let Some(name) = invocation.get_mut("step_name") {
                    redact(name, sensitive);
                }
            }
        }
    } else if value.get("variables_before").is_some() && value.get("started_at").is_some() {
        for key in [
            "step_name",
            "definition",
            "variables_before",
            "variables_after",
            "inputs",
            "output",
            "error",
            "attempts",
            "verification",
        ] {
            if let Some(field) = value.get_mut(key) {
                redact(field, sensitive);
            }
        }
    } else {
        redact(&mut value, sensitive);
    }
    let text = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.tmp");
    tokio::fs::write(&temporary, text)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(temporary, path)
        .await
        .map_err(|e| e.to_string())
}

impl TraceRecorder {
    pub async fn create(
        root: &Path,
        workflow: &Workflow,
        run_id: &str,
        debug: bool,
        mut sensitive: Vec<String>,
    ) -> Result<Arc<Self>, String> {
        use sha2::{Digest, Sha256};
        for spec in workflow.inputs.iter().filter(|spec| spec.sensitive) {
            if let Some(value) = &spec.default {
                sensitive.push(value.to_string());
                if let Some(text) = value.as_str() {
                    sensitive.push(text.into());
                }
            }
        }
        let directory = run_directory(root, &workflow.id, run_id, debug)?;
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|e| e.to_string())?;
        let snapshot = serde_json::json!({"id":workflow.id,"name":workflow.name,"steps":workflow.steps,"inputs":workflow.inputs});
        let revision = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?)
        );
        write_json(&directory.join("definition.json"), &snapshot, &sensitive).await?;
        let header = RunTrace {
            version: 1,
            run_id: run_id.into(),
            workflow_id: workflow.id.clone(),
            debug,
            revision,
            started_at: chrono::Utc::now(),
            finished_at: None,
            status: "running".into(),
            invocations: Vec::new(),
            storage_error: None,
            source: None,
            error: None,
        };
        write_json(&directory.join("index.json"), &header, &sensitive).await?;
        Ok(Arc::new(Self {
            workflow_id: workflow.id.clone(),
            directory,
            state: Mutex::new(TraceState {
                header,
                active: HashMap::new(),
                stack: Vec::new(),
            }),
            sensitive: std::sync::RwLock::new(sensitive),
        }))
    }

    fn secrets(&self) -> Vec<String> {
        self.sensitive.read().unwrap().clone()
    }

    pub fn redacted_text(&self, text: &str) -> String {
        self.secrets()
            .iter()
            .filter(|value| !value.is_empty())
            .fold(text.to_string(), |text, value| {
                text.replace(value, "[redacted]")
            })
    }

    pub fn add_sensitive(&self, values: impl IntoIterator<Item = Value>) {
        let mut secrets = self.sensitive.write().unwrap();
        for value in values {
            for form in [Some(value.to_string()), value.as_str().map(str::to_string)]
                .into_iter()
                .flatten()
            {
                if !form.is_empty() && !secrets.contains(&form) {
                    secrets.push(form);
                }
            }
        }
    }

    pub async fn error(&self, error: String) {
        let mut state = self.state.lock().await;
        state.header.error = Some(error);
        self.persist(&mut state).await;
    }

    pub async fn provenance(&self, source: Value) -> Result<(), String> {
        write_json(
            &self.directory.join("provenance.json"),
            &source,
            &self.secrets(),
        )
        .await?;
        let mut state = self.state.lock().await;
        let mut summary = source;
        if let Some(summary) = summary.as_object_mut() {
            summary.remove("variables");
            summary.remove("runtime_inputs");
        }
        state.header.source = Some(summary);
        self.persist(&mut state).await;
        Ok(())
    }

    pub async fn snapshots(&self, snapshots: &Value) -> Result<(), String> {
        write_json(
            &self.directory.join("workflows.json"),
            snapshots,
            &self.secrets(),
        )
        .await
    }

    pub async fn status(&self, status: &str) {
        let mut state = self.state.lock().await;
        state.header.status = status.into();
        self.persist(&mut state).await;
    }

    pub async fn begin(
        &self,
        workflow_id: &str,
        step: &Step,
        inputs: Value,
        variables: &HashMap<String, Value>,
    ) -> u64 {
        let mut state = self.state.lock().await;
        let id = state.header.invocations.len() as u64 + 1;
        let summary = InvocationSummary {
            id,
            parent_id: state.stack.last().copied(),
            workflow_id: workflow_id.into(),
            step_id: step.id.clone(),
            step_name: step.name.clone(),
            started_at: chrono::Utc::now(),
            finished_at: None,
            status: "running".into(),
        };
        let trace = InvocationTrace {
            summary: summary.clone(),
            definition: serde_json::to_value(step).unwrap_or(Value::Null),
            inputs,
            variables_before: variables.clone(),
            variables_after: HashMap::new(),
            output: None,
            error: None,
            attempts: Vec::new(),
            verification: None,
        };
        if let Err(error) = write_json(
            &self.directory.join(format!("{id}.json")),
            &trace,
            &self.secrets(),
        )
        .await
        {
            state.header.storage_error = Some(error);
        }
        state.header.invocations.push(summary);
        state.stack.push(id);
        state.active.insert(id, trace);
        self.persist(&mut state).await;
        id
    }

    pub async fn attempt(&self, attempt: u32, inputs: &Value, output: Result<&str, &str>) {
        let mut state = self.state.lock().await;
        if let Some(id) = state.stack.last().copied() {
            if let Some(trace) = state.active.get_mut(&id) {
                let attempt = if attempt == 0 {
                    trace.attempts.len() as u32 + 1
                } else {
                    attempt
                };
                trace.attempts.push(serde_json::json!({"attempt":attempt,"inputs":inputs,"output":output.as_ref().ok(),"error":output.as_ref().err(), "finished_at":chrono::Utc::now(), "verification":output.as_ref().ok().and_then(|value| verification(value))}));
                if let Err(error) = write_json(
                    &self.directory.join(format!("{id}.json")),
                    trace,
                    &self.secrets(),
                )
                .await
                {
                    state.header.storage_error = Some(error);
                }
            }
        }
    }

    pub async fn finish(
        &self,
        id: u64,
        variables: &HashMap<String, Value>,
        output: Result<&str, &str>,
        verification: Option<Value>,
    ) {
        let mut state = self.state.lock().await;
        // A timed-out container may drop a still-active child future. Close that
        // subtree now so subsequent siblings do not inherit an orphan parent.
        if let Some(position) = state.stack.iter().position(|active| *active == id) {
            let descendants = state.stack.split_off(position + 1);
            for child_id in descendants {
                if let Some(mut child) = state.active.remove(&child_id) {
                    child.summary.status = "interrupted".into();
                    child.summary.finished_at = Some(chrono::Utc::now());
                    if let Some(summary) = state
                        .header
                        .invocations
                        .iter_mut()
                        .find(|summary| summary.id == child_id)
                    {
                        *summary = child.summary.clone();
                    }
                    if let Err(error) = write_json(
                        &self.directory.join(format!("{child_id}.json")),
                        &child,
                        &self.secrets(),
                    )
                    .await
                    {
                        state.header.storage_error = Some(error);
                    }
                }
            }
        }
        if let Some(mut trace) = state.active.remove(&id) {
            trace.summary.finished_at = Some(chrono::Utc::now());
            trace.summary.status = if output.is_ok() { "success" } else { "error" }.into();
            trace.variables_after = variables.clone();
            trace.verification = verification;
            match output {
                Ok(value) => trace.output = Some(value.into()),
                Err(error) => trace.error = Some(error.into()),
            }
            if let Some(summary) = state
                .header
                .invocations
                .iter_mut()
                .find(|summary| summary.id == id)
            {
                *summary = trace.summary.clone();
            }
            if let Err(error) = write_json(
                &self.directory.join(format!("{id}.json")),
                &trace,
                &self.secrets(),
            )
            .await
            {
                state.header.storage_error = Some(error);
            }
            state.stack.retain(|active| *active != id);
            self.persist(&mut state).await;
        }
    }

    async fn persist(&self, state: &mut TraceState) {
        if let Err(error) = write_json(
            &self.directory.join("index.json"),
            &state.header,
            &self.secrets(),
        )
        .await
        {
            tracing::warn!("workflow trace unavailable: {error}");
            state.header.storage_error = Some(error);
        }
    }

    pub async fn complete(&self, status: &str) {
        let mut state = self.state.lock().await;
        if state.header.finished_at.is_some() {
            return;
        }
        state.header.status = status.into();
        state.header.finished_at = Some(chrono::Utc::now());
        // Timeout/cancellation can drop in-flight futures. Their records must not claim success.
        let active = std::mem::take(&mut state.active);
        for (id, mut trace) in active {
            trace.summary.status = "interrupted".into();
            trace.summary.finished_at = state.header.finished_at;
            if let Some(summary) = state
                .header
                .invocations
                .iter_mut()
                .find(|summary| summary.id == id)
            {
                *summary = trace.summary.clone();
            }
            if let Err(error) = write_json(
                &self.directory.join(format!("{id}.json")),
                &trace,
                &self.secrets(),
            )
            .await
            {
                state.header.storage_error = Some(error);
            }
        }
        state.stack.clear();
        self.persist(&mut state).await;
    }
}

/// Preserve native structured evidence without inferring verification from transport success.
pub fn verification(output: &str) -> Option<Value> {
    let value: Value =
        serde_json::from_str(output.strip_prefix("tool_completed:").unwrap_or(output)).ok()?;
    let object = value.as_object()?;
    let evidence: serde_json::Map<String, Value> =
        ["verification", "evidence", "postcondition", "delivery"]
            .into_iter()
            .filter_map(|key| {
                object
                    .get(key)
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect();
    (!evidence.is_empty()).then_some(Value::Object(evidence))
}

pub fn current() -> Option<Arc<TraceRecorder>> {
    CURRENT.try_with(Arc::clone).ok()
}

pub async fn list(root: &Path, workflow_id: &str, debug: bool) -> Result<Vec<RunTrace>, String> {
    if !valid_segment(workflow_id) {
        return Err("invalid_workflow_id".into());
    }
    let path =
        root.join(".editor-runs")
            .join(workflow_id)
            .join(if debug { "debug" } else { "normal" });
    let mut dirs = match tokio::fs::read_dir(path).await {
        Ok(dirs) => dirs,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut runs: Vec<RunTrace> = Vec::new();
    while let Some(entry) = dirs.next_entry().await.map_err(|e| e.to_string())? {
        if let Ok(data) = tokio::fs::read(entry.path().join("index.json")).await {
            if let Ok(run) = serde_json::from_slice(&data) {
                runs.push(run);
            }
        }
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    Ok(runs)
}

pub async fn read(
    root: &Path,
    workflow_id: &str,
    run_id: &str,
    debug: bool,
    invocation_id: u64,
) -> Result<InvocationTrace, String> {
    let data = tokio::fs::read(
        run_directory(root, workflow_id, run_id, debug)?.join(format!("{invocation_id}.json")),
    )
    .await
    .map_err(|e| e.to_string())?;
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
