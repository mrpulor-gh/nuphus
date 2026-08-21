//! scheduler.rs — Cron schedule engine
//!
//! Responsibilities:
//! - Parse cron expressions → compute next execution time
//! - Execute workflows on schedule (background workflows only)
//! - Foreground workflows (desktop/browser) are rejected for scheduling
//! - Persistent: schedules survive restart via .nuphus/schedules.json
//!
//! Cron format: 5-field "minute hour day month weekday"

use crate::workflow::store::WorkflowStore;
use crate::workflow::types::{Action, ScheduleConfig, Step};
use crate::Result;
use chrono::{DateTime, Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Scheduled task handle
struct ScheduledTask {
    config: ScheduleConfig,
    handle: JoinHandle<()>,
}

/// Persisted schedule store format
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedSchedules {
    pub schedules: HashMap<String, ScheduleConfig>,
}

/// SchedulerEngine — cron-based scheduling engine with disk persistence
pub struct SchedulerEngine {
    tasks: RwLock<HashMap<String, ScheduledTask>>,
    persist_path: PathBuf,
}

/// 递归检查步骤树中是否包含前台工具（desktop_*/browser_*）
fn has_frontend_step(steps: &[Step]) -> bool {
    const FRONTEND_PREFIXES: [&str; 2] = ["desktop_", "browser_"];
    for step in steps {
        match &step.action {
            Action::Tool { tool, .. } if FRONTEND_PREFIXES.iter().any(|p| tool.starts_with(p)) => {
                return true;
            }
            Action::Seq { seq } if has_frontend_step(seq) => {
                return true;
            }
            Action::Loop { def } if has_frontend_step(&def.steps) => {
                return true;
            }
            Action::If { def }
                if (has_frontend_step(&def.then) || has_frontend_step(&def.else_branch)) =>
            {
                return true;
            }
            Action::Wait { auto, .. } if has_frontend_step(auto) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

impl SchedulerEngine {
    pub fn new() -> Self {
        let persist_path = resolve_persist_path();
        Self {
            tasks: RwLock::new(HashMap::new()),
            persist_path,
        }
    }

    /// Setup cron schedule (background workflows only)
    ///
    /// `on_run` is the async callback executed when the schedule fires.
    /// Foreground workflows (with desktop_/browser_ tools) are rejected.
    /// Auto-persists to disk on success.
    pub async fn set_schedule<F, Fut>(
        &self,
        workflow_id: &str,
        config: ScheduleConfig,
        store: &WorkflowStore,
        on_run: F,
    ) -> Result<()>
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        // Check if workflow contains foreground operations
        if let Some(wf) = store.get(workflow_id).await {
            let has_frontend = has_frontend_step(&wf.steps);
            if has_frontend {
                return Err(crate::NuphusError::agent(
                    "Foreground workflows (desktop/browser) cannot use cron scheduling; they require user presence. Run manually instead.".to_string()
                ));
            }
        }

        // Remove existing schedule if present
        self.remove_schedule(workflow_id).await;

        if !config.enabled {
            return Ok(());
        }

        // 验证 cron 表达式：无效表达式应返回错误，不静默默认 5 分钟
        let cfg = config.clone();
        if parse_cron_to_interval(&cfg.cron).is_none() {
            return Err(crate::NuphusError::agent(format!(
                "Invalid cron expression: '{}'",
                cfg.cron
            )));
        }

        let handle = tokio::spawn(async move {
            loop {
                // 精确对齐到下一次 cron 匹配时刻（修正：原实现按固定间隔 sleep，
                // "1 6 * * *" 会退化为每 24h 从设置时刻偏移，永不命中 6:01）。
                // 无法解析的复杂表达式退化为固定间隔兜底（与旧行为一致）。
                let delay = next_cron_delay(&cfg.cron, Local::now()).unwrap_or_else(|| {
                    Duration::from_secs(parse_cron_to_interval(&cfg.cron).unwrap_or(300))
                });

                // Wait until next execution
                tokio::time::sleep(delay).await;

                // Execute callback — spawn 到独立 task 中捕获 panic，避免静默死亡
                let join_handle = tokio::spawn(on_run());
                if let Err(e) = join_handle.await {
                    tracing::error!("[scheduler] Scheduled task panicked: {:?}", e);
                }
            }
        });

        self.tasks
            .write()
            .await
            .insert(workflow_id.to_string(), ScheduledTask { config, handle });

        // 持久化到磁盘
        self.persist().await;

        Ok(())
    }

    /// 移除定时调度（自动持久化 + 中止运行中的任务）
    pub async fn remove_schedule(&self, workflow_id: &str) {
        // guard 必须先 drop：persist() 内部会 read()，write 持有期间 read 是死锁
        let task = {
            let mut tasks = self.tasks.write().await;
            tasks.remove(workflow_id)
        };
        if let Some(task) = task {
            task.handle.abort();
            self.persist().await;
        }
    }

    /// 获取工作流的调度配置
    pub async fn get_schedule(&self, workflow_id: &str) -> Option<ScheduleConfig> {
        self.tasks
            .read()
            .await
            .get(workflow_id)
            .map(|t| t.config.clone())
    }

    /// 列出所有已配置的调度
    pub async fn list_schedules(&self) -> Vec<(String, ScheduleConfig)> {
        self.tasks
            .read()
            .await
            .iter()
            .map(|(id, task)| (id.clone(), task.config.clone()))
            .collect()
    }

    // ── 持久化 ──

    /// 将所有活跃调度写入磁盘
    async fn persist(&self) {
        let tasks = self.tasks.read().await;
        let schedules: HashMap<String, ScheduleConfig> = tasks
            .iter()
            .map(|(id, t)| (id.clone(), t.config.clone()))
            .collect();
        let data = PersistedSchedules { schedules };

        if let Some(parent) = self.persist_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&data) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.persist_path, &json) {
                    tracing::error!("[scheduler] Failed to write schedules: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("[scheduler] Failed to serialize schedules: {}", e);
            }
        }
    }

    /// 从磁盘加载持久化的调度配置（返回数据，不启动任务）
    pub fn load_persisted() -> PersistedSchedules {
        let path = resolve_persist_path();
        match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!("[scheduler] Failed to parse schedules file: {}", e);
                    PersistedSchedules::default()
                }
            },
            Err(_) => PersistedSchedules::default(),
        }
    }

    /// 获取持久化路径（供 schedule_cron 工具使用）
    pub fn persist_path() -> PathBuf {
        resolve_persist_path()
    }
}

/// 解析持久化文件路径
fn resolve_persist_path() -> PathBuf {
    let mut path = std::env::current_dir().unwrap_or_default();
    path.push(".nuphus");
    path.push("schedules.json");
    path
}

impl Default for SchedulerEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Cron parser (lightweight) ──

/// 解析 cron 表达式 → 执行间隔（秒）
fn parse_cron_to_interval(cron: &str) -> Option<u64> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }

    let minute = parts[0];
    let hour = parts[1];
    let _day_of_month = parts[2];
    let _month = parts[3];
    let _day_of_week = parts[4];

    // ── 情况 1: "*/N * * * *" → 每 N 分钟
    if let Some(n) = minute.strip_prefix("*/") {
        if let Ok(mins) = n.parse::<u64>() {
            if hour == "*" && _day_of_month == "*" && _month == "*" && _day_of_week == "*" {
                return Some(mins * 60);
            }
        }
    }

    // ── 情况 2: "M * * * *" → 每小时 M 分（间隔 1 小时）
    if minute != "*" && hour == "*" {
        return Some(3600);
    }

    // ── 情况 3: "M H * * *" → 每天 H:M（间隔 24 小时）
    if minute != "*" && hour != "*" && _day_of_month == "*" && _month == "*" {
        return Some(86400);
    }

    // ── 情况 4: "* * * * *" → 每分钟
    if minute == "*" && hour == "*" {
        return Some(60);
    }

    // ── 情况 5: "*/N * * * *" 或混合 → 保守使用 5 分钟
    if minute.contains('/') || minute.contains(',') || minute.contains('-') {
        return Some(300);
    }

    // ── 默认
    if minute.parse::<u64>().is_ok() {
        return Some(3600);
    }

    None
}

/// 计算从 now 到下一个 cron 匹配时刻的延迟（精确对齐；1 分钟粒度）。
/// 修正语义：旧实现把 "1 6 * * *" 解析成固定 86400s 间隔从设置时刻偏移，
/// 永不命中 6:01。此函数逐分钟扫描直到第一个匹配时刻，返回真实延迟。
/// 无法解析（复杂/不支持表达式）返回 None，调用方退化为固定间隔兜底。
fn next_cron_delay(cron: &str, now: DateTime<Local>) -> Option<Duration> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    let (minute, hour, dom, month, dow) = (parts[0], parts[1], parts[2], parts[3], parts[4]);

    // 从下一分钟整点开始逐分钟扫描（最长 48 小时）
    let start = now.with_second(0)?.with_nanosecond(0)? + chrono::Duration::minutes(1);
    let mut t = start;
    for _ in 0..(48 * 60) {
        if cron_matches(t, minute, hour, dom, month, dow) {
            let secs = (t - now).num_seconds();
            if secs <= 0 {
                return None;
            }
            return Some(Duration::from_secs(secs as u64));
        }
        t += chrono::Duration::minutes(1);
    }
    None
}

/// 判断给定时刻是否匹配 cron 字段（5 字段全匹配）
fn cron_matches(
    t: DateTime<Local>,
    minute: &str,
    hour: &str,
    dom: &str,
    month: &str,
    dow: &str,
) -> bool {
    field_match(minute, t.minute())
        && field_match(hour, t.hour())
        && field_match(dom, t.day())
        && field_match(month, t.month())
        && field_match(dow, t.weekday().num_days_from_sunday())
}

/// 字段匹配：支持 `*`、`*/N`、`N`、`A-B` 范围、逗号列表（周日=0，cron 惯例）
fn field_match(field: &str, value: u32) -> bool {
    if field == "*" {
        return true;
    }
    if let Some(step) = field.strip_prefix("*/") {
        return step
            .parse::<u32>()
            .map(|n| n > 0 && value % n == 0)
            .unwrap_or(false);
    }
    if field.contains(',') {
        return field.split(',').any(|f| field_match(f, value));
    }
    if let Some((a, b)) = field.split_once('-') {
        if let (Ok(lo), Ok(hi)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
            return value >= lo && value <= hi;
        }
        return false;
    }
    field
        .trim()
        .parse::<u32>()
        .map(|v| v == value)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::store::WorkflowStore;

    #[test]
    fn test_parse_cron_every_minute() {
        assert_eq!(parse_cron_to_interval("* * * * *"), Some(60));
    }

    #[test]
    fn test_parse_cron_every_5min() {
        assert_eq!(parse_cron_to_interval("*/5 * * * *"), Some(300));
    }

    #[test]
    fn test_parse_cron_hourly() {
        assert_eq!(parse_cron_to_interval("0 * * * *"), Some(3600));
    }

    #[test]
    fn test_parse_cron_daily() {
        assert_eq!(parse_cron_to_interval("0 9 * * *"), Some(86400));
    }

    #[test]
    fn test_parse_cron_invalid() {
        assert_eq!(parse_cron_to_interval("invalid"), None);
        assert_eq!(parse_cron_to_interval(""), None);
    }

    #[test]
    fn test_next_cron_delay_daily_aligns_to_601() {
        // 核心回归：2026-08-17 18:02 设置 "1 6 * * *" → 明天 6:01（约 11h59m），
        // 旧实现按 86400s 固定偏移会错到明天 18:02。
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 8, 17, 18, 2, 0)
            .single()
            .unwrap();
        let d = next_cron_delay("1 6 * * *", now).unwrap();
        assert_eq!(d.as_secs(), 11 * 3600 + 59 * 60, "delay={}s", d.as_secs());
    }

    #[test]
    fn test_next_cron_delay_daily_past_hour_rolls_to_tomorrow() {
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 8, 17, 18, 2, 0)
            .single()
            .unwrap();
        // 9:00 已过 → 明天 9:00 = 14h58m
        let d = next_cron_delay("0 9 * * *", now).unwrap();
        assert_eq!(d.as_secs(), 14 * 3600 + 58 * 60, "delay={}s", d.as_secs());
    }

    #[test]
    fn test_next_cron_delay_hourly_and_step() {
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 8, 17, 18, 2, 30)
            .single()
            .unwrap();
        // 每分钟 → 18:03:00 = 30s（对齐到下一分钟整点）
        let d = next_cron_delay("* * * * *", now).unwrap();
        assert_eq!(d.as_secs(), 30);
        // 每 5 分钟 → 18:05:00 = 150s
        let d = next_cron_delay("*/5 * * * *", now).unwrap();
        assert_eq!(d.as_secs(), 150);
        // 每小时 0 分 → 19:00:00 = 57m30s = 3450s
        let d = next_cron_delay("0 * * * *", now).unwrap();
        assert_eq!(d.as_secs(), 3450);
    }

    #[test]
    fn test_next_cron_delay_invalid_and_range() {
        use chrono::TimeZone;
        let now = Local
            .with_ymd_and_hms(2026, 8, 17, 18, 2, 0)
            .single()
            .unwrap();
        assert_eq!(next_cron_delay("invalid", now), None);
        assert_eq!(next_cron_delay("", now), None);
        // 范围：周一至周五 6:01；2026-08-17 是周一，已过 → 明天（周二）6:01
        let d = next_cron_delay("1 6 * * 1-5", now).unwrap();
        assert_eq!(d.as_secs(), 11 * 3600 + 59 * 60, "delay={}s", d.as_secs());
    }

    #[tokio::test]
    async fn test_schedule_set_and_remove() {
        let scheduler = SchedulerEngine::new();
        let store = WorkflowStore::new();
        let config = ScheduleConfig {
            enabled: true,
            cron: "*/1 * * * *".to_string(),
            timezone: "Asia/Shanghai".to_string(),
            label: None,
        };

        // 模拟回调
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let result = scheduler
            .set_schedule("test-wf", config, &store, move || {
                let tx = tx.clone();
                async move {
                    let _ = tx.send("tick");
                }
            })
            .await;

        // 没有这个工作流，但 set_schedule 中 get 返回 None → 不阻塞
        assert!(result.is_ok());

        // 等待任务注册
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let listed = scheduler.list_schedules().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "test-wf");

        scheduler.remove_schedule("test-wf").await;

        let listed = scheduler.list_schedules().await;
        assert_eq!(listed.len(), 0);
    }

    /// 真实时钟冒烟测试：注册 */1 调度，等待下一次整分钟触发，验证「cron 对齐 +
    /// 定时触发 → on_run 回调」全链路。标 #[ignore] 防拖慢常驻测试集；
    /// 手动执行：cargo test -p nuphus scheduler -- --ignored
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn real_clock_trigger_smoke() {
        let scheduler = SchedulerEngine::new();
        let store = WorkflowStore::new();
        let config = ScheduleConfig {
            enabled: true,
            cron: "*/1 * * * *".to_string(),
            timezone: "Asia/Shanghai".to_string(),
            label: Some("smoke".to_string()),
        };

        let flag_path = std::env::temp_dir().join(format!(
            "nuphus-scheduler-smoke-{}.txt",
            uuid::Uuid::new_v4().simple()
        ));
        let flag = flag_path.clone();
        let result = scheduler
            .set_schedule("smoke-wf", config, &store, move || {
                let flag = flag.clone();
                async move {
                    // 触发成功标志：写入时间戳文件（无副作用）
                    let _ = std::fs::write(&flag, chrono::Local::now().to_rfc3339());
                }
            })
            .await;
        assert!(result.is_ok(), "set_schedule 应成功");

        // 等待触发：下一次整分钟最多 60s + 余量
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(75);
        let mut triggered = false;
        while tokio::time::Instant::now() < deadline {
            if flag_path.exists() {
                triggered = true;
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        scheduler.remove_schedule("smoke-wf").await;
        let _ = std::fs::remove_file(&flag_path);
        assert!(
            triggered,
            "*/1 调度应在 75s 内真实触发（cron 对齐 + 触发全链路）"
        );
    }
}
