//! TaskRun — ExecAgent 执行生命周期的**唯一真相**（服务端持有）
//!
//! # 为什么存在这个模块
//!
//! 三个组件的职责必须单向切开：
//! - **planner** 只做「理解传递」：产出 `.plan.md` 文档，给 Exec 注入上下文；
//! - **dispatch** 只做「执行」：把一个任务标题交给 ExecAgent 跑完，并在本模块记一笔；
//! - **task 面板** 只做「渲染**：读本模块的快照，不做任何状态推断。
//!
//! 于是：**状态由「执行该动作的代码」写入，模型零参与**。历史上状态靠子模型在摘要里
//! 自愿吐 JSON、靠一个从未注册过的 `planner_update` 工具、靠前端按模型给的 task_id
//! 配对——三条路全是软耦合，表现为「只显示执行中不完成」「任务 id 对不上」。
//!
//! # 契约
//!
//! - 身份由服务端下发（`run_id`），模型只提供标题与归属标签；
//! - 每次状态变迁推**全量快照**（`NuphusEvent::TaskRuns`），前端丢失一个事件最多旧一帧，
//!   下一次变迁自愈——不需要前端对账、配对、推断；
//! - `RunGuard` 覆盖**全部**退出路径（return / continue / `?` / panic）：正常路径显式
//!   `settle`，其余由 Drop 兜底结算为失败，与退出路径数量解耦。

use crate::agent::events::{EventEmitter, NuphusEvent};
use crate::agent::parallel;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// 生命周期状态。只有四个值，没有「pending」—— pending 属于计划文档的意图，
/// 不属于执行生命周期；未派发的任务在面板上根本不该出现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// 执行中
    Running,
    /// 已完成（ExecAgent 正常返回 success=true）
    Completed,
    /// 失败（ExecAgent 返回 success=false / 执行器错误 / 用户中断）
    Failed,
    /// 未结算（进程重启 / 异常路径兜底）——**绝不长期停留在 Running**
    Interrupted,
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, RunState::Running)
    }
}

/// 归属标签：这次执行来自哪份计划的哪个方向。
/// **仅用于显示**（面板上标「计划#N」），不参与任何状态判断——模型传错/不传都不影响生命周期。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOrigin {
    pub plan_path: String,
    pub task_no: Option<usize>,
}

/// 一次 ExecAgent 执行的完整记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRun {
    /// 服务端下发的唯一身份（进程内单调）
    pub run_id: String,
    /// 任务标题（短，面板显示用；由 Leader 的 `title` 参数提供）
    pub title: String,
    /// 派发正文**全文**（给 Exec 的那份动态消息：任务定义/上下文等；点开任务要看它）
    pub task: String,
    /// 派发时声明的 goal_type
    pub goal_type: String,
    /// 归属标签（有计划时才有）
    pub origin: Option<RunOrigin>,
    /// 同一标题/归属的第几次执行（重试计数，从 1 开始）
    pub attempt: u32,
    pub state: RunState,
    /// epoch 毫秒
    pub started_at: u64,
    pub settled_at: Option<u64>,
    pub duration_ms: Option<u64>,
    /// 终态才有：true=Completed，false=Failed
    pub ok: Option<bool>,
    /// 终态才有：ExecAgent 交付内容**全文**（不截断；展示侧是全屏弹窗）
    pub summary: Option<String>,
}

impl TaskRun {
    /// 面板一行的高度浓缩：标题 + 归属 + 第几次。
    pub fn display_label(&self) -> String {
        let mut s = self.title.clone();
        if self.attempt > 1 {
            s.push_str(&format!("（第 {} 次）", self.attempt));
        }
        s
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 进程内台账：Vec 保序（创建序 = 展示序），`run_id` 唯一。
/// 可选落盘（`{dir}/task-runs.json`）：进程重启后加载 + 清扫，让「未结算」可见且确定。
pub struct TaskRunRegistry {
    runs: Mutex<Vec<TaskRun>>,
    seq: Mutex<u64>,
    persist_dir: Mutex<Option<PathBuf>>,
    loaded: Mutex<bool>,
}

impl Default for TaskRunRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRunRegistry {
    pub fn new() -> Self {
        Self {
            runs: Mutex::new(Vec::new()),
            seq: Mutex::new(0),
            persist_dir: Mutex::new(None),
            loaded: Mutex::new(false),
        }
    }

    /// 绑定落盘目录（首次调用生效，重复调用忽略）。
    pub fn bind_persistence(&self, dir: impl Into<PathBuf>) {
        if let Ok(mut slot) = self.persist_dir.lock() {
            if slot.is_none() {
                *slot = Some(dir.into());
            }
        }
    }

    fn persist_path(&self) -> Option<PathBuf> {
        self.persist_dir
            .lock()
            .ok()
            .and_then(|d| d.clone())
            .map(|d| d.join("task-runs.json"))
    }

    /// 落盘（尽力而为：失败只 warn，不影响执行链路）。
    fn flush(&self) {
        let Some(path) = self.persist_path() else {
            return;
        };
        let runs = match self.runs.lock() {
            Ok(r) => r.clone(),
            Err(_) => return,
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let tmp = path.with_extension("json.tmp");
        match serde_json::to_string(&runs) {
            Ok(json) => {
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            Err(e) => tracing::warn!("[TaskRun] 序列化快照失败: {e}"),
        }
    }

    /// 从磁盘加载上一次的终态快照（Running 一律丢弃——那是上个进程的未结算），
    /// 返回本次清扫条数。进程内只做一次（幂等）。
    pub fn load_and_sweep(&self) -> usize {
        {
            let loaded = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
            if *loaded {
                return 0;
            }
        }
        if let Some(path) = self.persist_path() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                match serde_json::from_str::<Vec<TaskRun>>(&content) {
                    Ok(runs) => {
                        // seq 越过历史最大值，避免重启后 run_id 撞号
                        let max_seq = runs
                            .iter()
                            .filter_map(|r| r.run_id.strip_prefix("run-"))
                            .filter_map(|n| n.parse::<u64>().ok())
                            .max()
                            .unwrap_or(0);
                        if let Ok(mut seq) = self.seq.lock() {
                            *seq = (*seq).max(max_seq);
                        }
                        if let Ok(mut cur) = self.runs.lock() {
                            cur.extend(runs);
                        }
                    }
                    Err(e) => tracing::warn!("[TaskRun] 旧快照解析失败（忽略）: {e}"),
                }
            }
        }
        if let Ok(mut loaded) = self.loaded.lock() {
            *loaded = true;
        }
        let swept = self.sweep_unsettled();
        self.flush();
        swept
    }

    /// **轮次边界**：开始新一代（一次用户消息 = 一轮）。
    ///
    /// task 面板的生命周期就是「一轮一面墙」——本轮所有派发累积在一块墙上，下一轮开始
    /// 整体归零。这样新计划/新派发绝不会和上一轮的记录混在一起；历史不丢，只是换了
    /// 归属：执行面板有时间线、计划文档有台账、落盘快照有上一代的终态。
    ///
    /// 未结算的 running 不保留（那是上一轮的事，由上一轮自己的结束事件或进程重启清扫
    /// 负责），直接清空后落盘。
    pub fn start_generation(&self) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.clear();
        }
        self.flush();
    }

    /// 开一次执行：下发 `run_id`，状态置 Running，返回身份。
    /// `attempt` 按「同归属（无归属时同标题）」的历史次数递增。
    /// `task` = 派发正文全文（给 Exec 的那份，含任务定义/上下文），面板点开要看它。
    pub fn open(
        &self,
        title: &str,
        task: &str,
        goal_type: &str,
        origin: Option<RunOrigin>,
    ) -> String {
        let mut seq = self.seq.lock().expect("task_run seq 锁中毒");
        *seq += 1;
        let run_id = format!("run-{}", *seq);
        drop(seq);

        let mut runs = self.runs.lock().expect("task_run 锁中毒");
        let attempt = runs
            .iter()
            .filter(|r| match (&r.origin, &origin) {
                (Some(a), Some(b)) => a == b,
                (None, None) => r.title == title,
                _ => false,
            })
            .count() as u32
            + 1;

        runs.push(TaskRun {
            run_id: run_id.clone(),
            title: title.to_string(),
            task: task.to_string(),
            goal_type: goal_type.to_string(),
            origin,
            attempt,
            state: RunState::Running,
            started_at: now_ms(),
            settled_at: None,
            duration_ms: None,
            ok: None,
            summary: None,
        });
        drop(runs);
        self.flush();
        run_id
    }

    /// 结算一次执行。返回 false = 该 run_id 不存在或已终态（重复结算被忽略，幂等）。
    pub fn settle(&self, run_id: &str, ok: bool, summary: &str) -> bool {
        let mut runs = self.runs.lock().expect("task_run 锁中毒");
        let now = now_ms();
        // 从后向前找：同 run_id 唯一，逆序匹配最近一次（防御重复 id）
        for r in runs.iter_mut().rev() {
            if r.run_id == run_id && r.state == RunState::Running {
                r.state = if ok {
                    RunState::Completed
                } else {
                    RunState::Failed
                };
                r.settled_at = Some(now);
                r.duration_ms = Some(now.saturating_sub(r.started_at));
                r.ok = Some(ok);
                // 交付内容**全文入库，不截断**：这是 ExecAgent 交给 Leader/用户的完整结果，
                // 不是给单行预览用的摘要。展示侧是全屏弹窗（可滚动），不需要 preview 语义。
                r.summary = Some(summary.to_string());
                drop(runs);
                self.flush();
                return true;
            }
        }
        false
    }

    /// 结算为「未结算」（异常路径：panic / future 被 drop / 早退漏结）。
    /// 与 `Failed` 分开：那是**业务失败**（有明确结论），这是**异常中断**（没有结论）。
    pub fn settle_interrupted(&self, run_id: &str, reason: &str) -> bool {
        let mut runs = self.runs.lock().expect("task_run 锁中毒");
        let now = now_ms();
        for r in runs.iter_mut().rev() {
            if r.run_id == run_id && r.state == RunState::Running {
                r.state = RunState::Interrupted;
                r.settled_at = Some(now);
                r.duration_ms = Some(now.saturating_sub(r.started_at));
                r.ok = Some(false);
                // 与 settle 同口径：全文，不截断（中断理由是完整一句话）
                r.summary = Some(reason.to_string());
                drop(runs);
                self.flush();
                return true;
            }
        }
        false
    }

    /// 把所有滞留 Running 的条月判为「未结算」。进程重启加载旧快照后必须调用一次。
    pub fn sweep_unsettled(&self) -> usize {
        let mut runs = self.runs.lock().expect("task_run 锁中毒");
        let mut n = 0;
        for r in runs.iter_mut() {
            if r.state == RunState::Running {
                r.state = RunState::Interrupted;
                r.settled_at = Some(now_ms());
                r.duration_ms = r.settled_at.map(|t| t.saturating_sub(r.started_at));
                r.ok = Some(false);
                r.summary = Some("进程重启前未结算".to_string());
                n += 1;
            }
        }
        drop(runs);
        if n > 0 {
            self.flush();
        }
        n
    }

    pub fn snapshot(&self) -> Vec<TaskRun> {
        self.runs.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn running_count(&self) -> usize {
        self.runs
            .lock()
            .map(|r| r.iter().filter(|x| x.state == RunState::Running).count())
            .unwrap_or(0)
    }
}

/// 进程级单例（桌面壳注入一次，Leader 每轮复用）。
static GLOBAL: OnceLock<Arc<TaskRunRegistry>> = OnceLock::new();

pub fn global_registry() -> Arc<TaskRunRegistry> {
    GLOBAL
        .get_or_init(|| Arc::new(TaskRunRegistry::new()))
        .clone()
}

/// 把一次快照推给前端。无 emitter（测试 / 未注入）时静默跳过。
/// 泛型签名：既接受 `Arc<dyn EventEmitter>`（RunGuard 持有），也接受具体 emitter 引用
/// （桌面壳启动清扫后要立刻推一帧）。
pub fn emit_snapshot<E: EventEmitter + ?Sized>(reg: &TaskRunRegistry, emitter: Option<&E>) {
    if let Some(em) = emitter {
        em.emit(NuphusEvent::TaskRuns {
            runs: reg.snapshot(),
        });
    }
}

/// RAII 守卫：`open` 之后 armed，显式 `settle` 后 disarm；
/// 其余任何退出（return / continue / `?` / panic）由 Drop 兜底结算为失败并推快照。
/// 与 dispatch 的退出路径数量解耦——新增早退分支不会漏结算。
pub struct RunGuard<'a> {
    reg: &'a TaskRunRegistry,
    run_id: String,
    title: String,
    emitter: Option<Arc<dyn EventEmitter>>,
    settled: bool,
}

impl<'a> RunGuard<'a> {
    pub fn new(
        reg: &'a TaskRunRegistry,
        run_id: String,
        title: String,
        emitter: Option<Arc<dyn EventEmitter>>,
    ) -> Self {
        // 并行协作契约：Exec 在途是伙伴侧「主会话正在改文件」避让文案的唯一
        // 事实源，随 open 同步落（写入点只有这一处）。
        parallel::set_exec(&run_id, &title);
        Self {
            reg,
            run_id,
            title,
            emitter,
            settled: false,
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// 正常结算（成功或失败都由调用方给真实结论）。
    pub fn settle(&mut self, ok: bool, summary: &str) {
        if self.settled {
            return;
        }
        self.reg.settle(&self.run_id, ok, summary);
        parallel::clear_exec(&self.run_id);
        self.settled = true;
        emit_snapshot(self.reg, self.emitter.as_deref());
    }

    /// 异常结算：没有结论的中断（与业务失败分开）。
    pub fn settle_interrupted(&mut self, reason: &str) {
        if self.settled {
            return;
        }
        self.reg.settle_interrupted(&self.run_id, reason);
        parallel::clear_exec(&self.run_id);
        self.settled = true;
        emit_snapshot(self.reg, self.emitter.as_deref());
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        // 无论是否已结算，在途态都按 run_id 收一次（匹配才清，迟到兜底
        // 动不了更新一笔）；先于 settle 兜底，保证任何退出路径都不残留。
        parallel::clear_exec(&self.run_id);
        if !self.settled {
            self.reg.settle_interrupted(
                &self.run_id,
                "派发异常中断（未结算：早退/panic/未走完退出路径）",
            );
            emit_snapshot(self.reg, self.emitter.as_deref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_settles_running_to_terminal() {
        let reg = TaskRunRegistry::new();
        let id = reg.open("任务A", "派发正文", "file_operation", None);
        assert_eq!(reg.running_count(), 1);
        assert!(reg.settle(&id, true, "做完了"));
        assert_eq!(reg.running_count(), 0);
        let r = &reg.snapshot()[0];
        assert_eq!(r.state, RunState::Completed);
        assert_eq!(r.ok, Some(true));
        assert!(r.duration_ms.is_some());
    }

    #[test]
    fn settle_is_idempotent() {
        let reg = TaskRunRegistry::new();
        let id = reg.open("任务A", "派发正文", "general", None);
        assert!(reg.settle(&id, true, "第一次"));
        assert!(!reg.settle(&id, false, "重复结算必须被忽略"));
        assert_eq!(reg.snapshot()[0].state, RunState::Completed);
    }

    #[test]
    fn attempt_counts_repeats_of_same_origin() {
        let reg = TaskRunRegistry::new();
        let o = || {
            Some(RunOrigin {
                plan_path: "p.plan.md".into(),
                task_no: Some(1),
            })
        };
        let a = reg.open("任务1", "派发正文", "general", o());
        let b = reg.open("任务1", "派发正文", "general", o());
        assert_eq!(reg.snapshot()[0].attempt, 1);
        assert_eq!(reg.snapshot()[1].attempt, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn sweep_never_leaves_running() {
        let reg = TaskRunRegistry::new();
        let _ = reg.open("任务A", "派发正文", "general", None);
        let _ = reg.open("任务B", "派发正文", "general", None);
        assert_eq!(reg.sweep_unsettled(), 2);
        assert_eq!(reg.running_count(), 0);
        assert!(reg.snapshot().iter().all(|r| r.state.is_terminal()));
    }

    #[test]
    fn guard_settles_on_early_return() {
        let reg = TaskRunRegistry::new();
        let id = reg.open("任务A", "派发正文", "general", None);
        {
            // 未显式 settle，作用域结束即触发 Drop —— 模拟 `?` / return 早退
            let _g = RunGuard::new(&reg, id.clone(), "任务A".to_string(), None);
        }
        // 守卫已兜底结算为「未结算」（不是业务失败）
        let r = reg.snapshot().into_iter().find(|x| x.run_id == id).unwrap();
        assert_eq!(r.state, RunState::Interrupted);
        assert_eq!(reg.running_count(), 0);
    }

    #[test]
    fn interrupted_is_distinct_from_failed() {
        let reg = TaskRunRegistry::new();
        let biz = reg.open("任务A", "派发正文", "general", None);
        let abn = reg.open("任务B", "派发正文", "general", None);
        assert!(reg.settle(&biz, false, "安全检查未通过"));
        assert!(reg.settle_interrupted(&abn, "panic"));
        let snap = reg.snapshot();
        assert_eq!(snap[0].state, RunState::Failed);
        assert_eq!(snap[1].state, RunState::Interrupted);
        assert_ne!(snap[0].state, snap[1].state);
    }

    #[test]
    fn generation_reset_never_mixes_rounds() {
        let reg = TaskRunRegistry::new();
        let old = reg.open("上一轮的任务", "派发正文", "general", None);
        assert!(reg.settle(&old, true, "完成"));
        reg.start_generation();
        assert!(reg.snapshot().is_empty(), "新一轮必须是空墙");
        let fresh = reg.open("本轮的任务", "派发正文", "general", None);
        assert_eq!(reg.snapshot().len(), 1);
        assert_eq!(reg.snapshot()[0].run_id, fresh);
    }
}
