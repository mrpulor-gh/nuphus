//! Persistent five-field cron scheduling with IANA timezone support.

use crate::workflow::store::WorkflowStore;
use crate::workflow::types::{Action, InputSpec, ScheduleConfig, Step};
use crate::Result;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Internal persisted binding. Flattening keeps old ScheduleConfig-only JSON readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleBinding {
    #[serde(flatten)]
    pub config: ScheduleConfig,
    /// Explicit values only. Declaration defaults are deliberately resolved at trigger time.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    inputs: HashMap<String, serde_json::Value>,
}

impl ScheduleBinding {
    fn new(
        config: ScheduleConfig,
        specs: &[InputSpec],
        explicit: &HashMap<String, serde_json::Value>,
    ) -> Result<Self> {
        crate::workflow::inputs::resolve_declared_only(specs, explicit)?;
        let mut inputs = explicit.clone();
        for spec in specs.iter().filter(|spec| spec.sensitive) {
            let Some(value) = inputs.get_mut(&spec.name) else {
                continue;
            };
            let json = serde_json::to_string(value).map_err(|error| {
                crate::NuphusError::agent(format!(
                    "无法序列化敏感调度输入 '{}': {}",
                    spec.name, error
                ))
            })?;
            *value = serde_json::Value::String(crate::cookies::encrypt_secret(&json));
        }
        Ok(Self { config, inputs })
    }

    pub fn decode_inputs(&self, specs: &[InputSpec]) -> Result<HashMap<String, serde_json::Value>> {
        let mut inputs = self.inputs.clone();
        for spec in specs.iter().filter(|spec| spec.sensitive) {
            let Some(value) = inputs.get_mut(&spec.name) else {
                continue;
            };
            let stored = value.as_str().ok_or_else(|| {
                crate::NuphusError::agent(format!("敏感调度输入 '{}' 的持久化格式无效", spec.name))
            })?;
            let json = crate::cookies::decrypt_secret(stored).ok_or_else(|| {
                crate::NuphusError::agent(format!("敏感调度输入 '{}' 解密失败", spec.name))
            })?;
            *value = serde_json::from_str(&json).map_err(|_| {
                crate::NuphusError::agent(format!(
                    "敏感调度输入 '{}' 解密后的 JSON 无效",
                    spec.name
                ))
            })?;
        }
        // Revalidate against the workflow's current declaration. This intentionally resolves
        // current defaults only for validation and returns the original explicit snapshot.
        crate::workflow::inputs::resolve_declared_only(specs, &inputs)?;
        Ok(inputs)
    }

    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }
}

struct ScheduledTask {
    binding: ScheduleBinding,
    handle: JoinHandle<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedSchedules {
    pub schedules: HashMap<String, ScheduleBinding>,
}

pub struct SchedulerEngine {
    tasks: RwLock<HashMap<String, ScheduledTask>>,
    persist_path: PathBuf,
}

fn has_frontend_step(steps: &[Step]) -> bool {
    const FRONTEND_PREFIXES: [&str; 2] = ["desktop_", "browser_"];
    steps.iter().any(|step| match &step.action {
        Action::Tool { tool, .. } => FRONTEND_PREFIXES
            .iter()
            .any(|prefix| tool.starts_with(prefix)),
        Action::Seq { seq } => has_frontend_step(seq),
        Action::Loop { def } => has_frontend_step(&def.steps),
        Action::If { def } => has_frontend_step(&def.then) || has_frontend_step(&def.else_branch),
        Action::Wait { auto, .. } => has_frontend_step(auto),
        _ => false,
    })
}

fn parse_five_field_cron(expression: &str) -> Result<Schedule> {
    if expression.split_whitespace().count() != 5 {
        return Err(crate::NuphusError::agent(format!(
            "Invalid cron expression '{}': expected exactly 5 fields",
            expression
        )));
    }
    Schedule::from_str(&format!("0 {expression}")).map_err(|error| {
        crate::NuphusError::agent(format!(
            "Invalid cron expression '{}': {}",
            expression, error
        ))
    })
}

fn parse_timezone(name: &str) -> Result<Tz> {
    name.parse::<Tz>()
        .map_err(|_| crate::NuphusError::agent(format!("Invalid IANA timezone: '{}'", name)))
}

fn next_occurrence(schedule: &Schedule, timezone: Tz, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    schedule
        .after(&now.with_timezone(&timezone))
        .next()
        .map(|next| next.with_timezone(&Utc))
}

impl SchedulerEngine {
    pub fn new() -> Self {
        Self::with_persist_path(resolve_persist_path())
    }

    pub(crate) fn with_persist_path(persist_path: PathBuf) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            persist_path,
        }
    }

    pub async fn set_schedule<F, Fut>(
        &self,
        workflow_id: &str,
        config: ScheduleConfig,
        explicit_inputs: HashMap<String, serde_json::Value>,
        store: &WorkflowStore,
        on_run: F,
    ) -> Result<()>
    where
        F: Fn(HashMap<String, serde_json::Value>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let workflow = store.get(workflow_id).await.ok_or_else(|| {
            crate::NuphusError::agent(format!("Workflow not found: {workflow_id}"))
        })?;
        if has_frontend_step(&workflow.steps) {
            return Err(crate::NuphusError::agent(
                "Foreground workflows (desktop/browser) cannot use cron scheduling; they require user presence. Run manually instead.".to_string(),
            ));
        }

        let schedule = parse_five_field_cron(&config.cron)?;
        let timezone = parse_timezone(&config.timezone)?;
        let binding = ScheduleBinding::new(config.clone(), &workflow.inputs, &explicit_inputs)?;

        self.remove_schedule(workflow_id).await;
        if !config.enabled {
            return Ok(());
        }

        let callback_inputs = explicit_inputs;
        let handle = tokio::spawn(async move {
            loop {
                let Some(next) = next_occurrence(&schedule, timezone, Utc::now()) else {
                    tracing::error!("[scheduler] Cron expression has no future occurrence");
                    return;
                };
                let delay = (next - Utc::now())
                    .to_std()
                    .unwrap_or_else(|_| Duration::from_millis(1));
                tokio::time::sleep(delay).await;
                let join = tokio::spawn(on_run(callback_inputs.clone()));
                if let Err(error) = join.await {
                    tracing::error!("[scheduler] Scheduled task panicked: {:?}", error);
                }
            }
        });

        self.tasks
            .write()
            .await
            .insert(workflow_id.to_string(), ScheduledTask { binding, handle });
        if let Err(error) = self.persist_current().await {
            if let Some(task) = self.tasks.write().await.remove(workflow_id) {
                task.handle.abort();
            }
            return Err(error);
        }
        Ok(())
    }

    pub async fn remove_schedule(&self, workflow_id: &str) {
        if let Some(task) = self.tasks.write().await.remove(workflow_id) {
            task.handle.abort();
        }
        if let Err(error) = self.persist_current().await {
            tracing::error!("[scheduler] Failed to persist schedule removal: {}", error);
        }
    }

    pub async fn get_schedule(&self, workflow_id: &str) -> Option<ScheduleConfig> {
        self.tasks
            .read()
            .await
            .get(workflow_id)
            .map(|task| task.binding.config.clone())
    }

    pub async fn list_schedules(&self) -> Vec<(String, ScheduleConfig)> {
        self.tasks
            .read()
            .await
            .iter()
            .map(|(id, task)| (id.clone(), task.binding.config.clone()))
            .collect()
    }

    pub async fn persist_current(&self) -> Result<()> {
        let schedules = self
            .tasks
            .read()
            .await
            .iter()
            .map(|(id, task)| (id.clone(), task.binding.clone()))
            .collect();
        write_persisted(&self.persist_path, &PersistedSchedules { schedules })
    }

    pub fn load_persisted() -> PersistedSchedules {
        load_persisted_from(&resolve_persist_path())
    }

    pub fn load_current(&self) -> PersistedSchedules {
        load_persisted_from(&self.persist_path)
    }

    pub fn persist_path() -> PathBuf {
        resolve_persist_path()
    }
}

fn write_persisted(path: &Path, data: &PersistedSchedules) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn load_persisted_from(path: &Path) -> PersistedSchedules {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_else(|error| {
            tracing::warn!("[scheduler] Failed to parse schedules file: {}", error);
            PersistedSchedules::default()
        }),
        Err(_) => PersistedSchedules::default(),
    }
}

fn resolve_persist_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join(".nuphus")
        .join("schedules.json")
}

impl Default for SchedulerEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::types::{InputKind, Workflow};
    use chrono::TimeZone;

    fn config(cron: &str, timezone: &str) -> ScheduleConfig {
        ScheduleConfig {
            cron: cron.into(),
            timezone: timezone.into(),
            enabled: true,
            label: None,
        }
    }

    fn input(name: &str, kind: InputKind, sensitive: bool) -> InputSpec {
        InputSpec {
            name: name.into(),
            kind,
            required: false,
            default: None,
            description: None,
            sensitive,
        }
    }

    #[test]
    fn old_schedule_file_migrates_to_empty_inputs() {
        let data: PersistedSchedules = serde_json::from_value(serde_json::json!({
            "schedules": {"wf": {"cron": "0 9 * * *", "timezone": "UTC", "enabled": true}}
        }))
        .unwrap();
        assert_eq!(data.schedules["wf"].input_count(), 0);
        assert_eq!(data.schedules["wf"].config.cron, "0 9 * * *");
    }

    #[test]
    fn sensitive_input_round_trips_and_bad_cipher_fails() {
        let specs = vec![input("token", InputKind::String, true)];
        let explicit = HashMap::from([("token".into(), serde_json::json!("secret"))]);
        let binding = ScheduleBinding::new(config("0 9 * * *", "UTC"), &specs, &explicit).unwrap();
        assert_eq!(binding.decode_inputs(&specs).unwrap(), explicit);

        let mut bad = binding;
        bad.inputs
            .insert("token".into(), serde_json::json!("enc:v1:not-base64"));
        assert!(bad.decode_inputs(&specs).is_err());
    }

    #[test]
    fn cron_timezone_and_dst_are_resolved_by_timezone_database() {
        let schedule = parse_five_field_cron("30 2 * * *").unwrap();
        let timezone = parse_timezone("America/New_York").unwrap();
        let before_gap = Utc.with_ymd_and_hms(2026, 3, 8, 6, 0, 0).unwrap();
        let next = next_occurrence(&schedule, timezone, before_gap).unwrap();
        // 02:30 does not exist on spring-forward day, so the next run is March 9.
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 3, 9, 6, 30, 0).unwrap());
        assert!(parse_five_field_cron("0 0 * *").is_err());
        assert!(parse_timezone("Mars/Olympus").is_err());
    }

    #[tokio::test]
    async fn schedule_requires_workflow_and_replaces_input_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "nuphus_scheduler_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = WorkflowStore::with_root(root.join("workflows"));
        let scheduler = SchedulerEngine::with_persist_path(root.join("schedules.json"));
        let missing = scheduler
            .set_schedule(
                "missing",
                config("* * * * *", "UTC"),
                HashMap::new(),
                &store,
                |_| async {},
            )
            .await;
        assert!(missing.is_err());

        let mut workflow = Workflow::new("scheduled");
        workflow.inputs = vec![input("count", InputKind::Number, false)];
        store.save(&workflow).await.unwrap();
        scheduler
            .set_schedule(
                &workflow.id,
                config("* * * * *", "UTC"),
                HashMap::from([("count".into(), serde_json::json!(1))]),
                &store,
                |_| async {},
            )
            .await
            .unwrap();
        scheduler
            .set_schedule(
                &workflow.id,
                config("*/5 * * * *", "UTC"),
                HashMap::from([("count".into(), serde_json::json!(2))]),
                &store,
                |_| async {},
            )
            .await
            .unwrap();
        let persisted = load_persisted_from(&root.join("schedules.json"));
        assert_eq!(persisted.schedules.len(), 1);
        assert_eq!(
            persisted.schedules[&workflow.id].inputs["count"],
            serde_json::json!(2)
        );
        scheduler.remove_schedule(&workflow.id).await;
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn current_default_is_not_persisted() {
        let root = std::env::temp_dir().join(format!(
            "nuphus_scheduler_default_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let store = WorkflowStore::with_root(root.join("workflows"));
        let scheduler = SchedulerEngine::with_persist_path(root.join("schedules.json"));
        let mut workflow = Workflow::new("defaulted");
        let mut spec = input("mode", InputKind::String, false);
        spec.required = true;
        spec.default = Some(serde_json::json!("old"));
        workflow.inputs = vec![spec];
        store.save(&workflow).await.unwrap();
        scheduler
            .set_schedule(
                &workflow.id,
                config("* * * * *", "UTC"),
                HashMap::new(),
                &store,
                |_| async {},
            )
            .await
            .unwrap();
        let persisted = load_persisted_from(&root.join("schedules.json"));
        assert_eq!(persisted.schedules[&workflow.id].input_count(), 0);
        let mut current_spec = input("mode", InputKind::String, false);
        current_spec.required = true;
        current_spec.default = Some(serde_json::json!("new"));
        let explicit = persisted.schedules[&workflow.id]
            .decode_inputs(std::slice::from_ref(&current_spec))
            .unwrap();
        let resolved =
            crate::workflow::inputs::resolve_declared_inputs(&[current_spec], &explicit).unwrap();
        assert_eq!(resolved["mode"], serde_json::json!("new"));
        scheduler.remove_schedule(&workflow.id).await;
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
