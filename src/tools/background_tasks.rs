//! 后台任务注册表 —— **进程级单例**（谁还活着的那本账）
//!
//! # 为什么存在这个模块
//!
//! 「强制终止」此前只能置 `cancel_flag`，而**工具执行内部一个检查点都没有**：
//! agent 循环阻塞在 `spawn_blocking` 的同步工具里，`cancel_flag` 要等工具返回
//! 才被检查 ⇒ 用户侧表现就是「点了中断毫无反应」（`lifecycle.rs` 自陈）。
//!
//! 但解除等待只是第一步。`spawn_blocking` 不可中断，真正让阻塞线程退出的唯一
//! 办法是**杀掉它手里的子进程**——否则等待缩短了，进程照样变成孤儿继续跑。
//! 而现状下 `child` / `pgid` 在 `system_shell` 执行体里从不外泄，**无人能杀**。
//!
//! 于是本模块承担三件事：
//! 1. 登记任何「由工具启动、需要跨工具调用边界存活」的子进程（id / 工具 / 命令 /
//!    pid / 启动时刻 / retain / 输出文件位置）；
//! 2. 提供按 pid 杀进程树、按 id 结束、过滤死条目的能力（供 `interrupt` 与
//!    前端管理接口消费）；
//! 3. 规定取消事实的**文本编码**，让 Agent 拿到「命令是否还在跑」而不是笼统的
//!    “失败”（否则它会重跑长任务，跑两遍的代价远大于等 30 分钟）。
//!
//! # 契约（调用点勿私自改写）
//!
//! - **`retain` 是显式意图**：调用方声明后台保留才置 `true`。默认（前台）`false`，
//!   取消时按 pid 杀进程树。保留项**任何自动路径都不许杀**
//!   （[`BackgroundTaskRegistry::kill_foreground`] 只碰 `retain == false`），
//!   只能由用户/Agent 显式按 id 结束——不替用户杀保留项。
//! - **超时不杀**：长任务超时后进程保留 ⇒ 此时必须登记为 `retain = true`，
//!   并把 pid 与输出文件位置一并告诉 Agent。
//! - **取消事实必须结构化**：见 [`CancellationNotice`] 与 [`CANCELLED_MARKER`]。
//!
//! # 存活探测的开销
//!
//! [`SystemProcessControl::is_alive`] 在 Windows 走 `tasklist /FI "PID eq N"`
//! —— **一次进程 spawn（约 30–100ms）**。因此它只出现在用户触发的路径上
//! （清单列举 / `interrupt` 汇总 / `kill_one`），**绝不出现在工具的 150ms 轮询
//! 循环里**（那里用 `Child::try_wait()`，无 spawn）。
//!
//! 已知局限：探测只看 pid。pid 回收后条目可能被误判为存活（见交付说明盲区）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// 清单里命令文本的最大字符数。**仅展示用**，不是执行内容。
const COMMAND_PREVIEW_CHARS: usize = 160;

/// 取消事实标记。与「本批不改 `ToolResult` 结构」一致，采用文本约定编码 ——
/// 先例见 `lib.rs` 的 `__EXIT_CODE:{code}__`。
///
/// 载荷格式：`__CANCELLED__ {单行 JSON}\n{明文}`。
/// 第一段给程序解码（[`decode_cancellation`]），后一段给模型读。
pub const CANCELLED_MARKER: &str = "__CANCELLED__";

/// CREATE_NO_WINDOW —— 后台起 console 子进程不弹黑框（与 `definitions/system.rs`
/// 既有 shell 启动一致）。
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// ============================================================================
// 条目
// ============================================================================

/// 一个由工具启动、需要跨工具调用边界追踪的子进程。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTask {
    /// 进程内唯一 id（`bg-<seq>`），同时是输出日志文件名的主干
    pub id: String,
    /// 启动它的工具名（如 `system_shell`）
    pub tool: String,
    /// 命令文本（截断到 [`COMMAND_PREVIEW_CHARS`]，仅供展示/汇报）
    pub command: String,
    pub pid: u32,
    /// 启动时刻（epoch 毫秒）
    pub started_at_ms: u64,
    /// **显式保留意图**：true = 用户/Agent 要求保留在后台，取消时不杀
    pub retain: bool,
    /// 输出日志路径（tee 目标，进程存活期间持续追加）
    pub output_path: Option<String>,
}

/// 上层看到的清单条目（只含存活项，见 [`BackgroundTaskRegistry::list`]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTaskView {
    pub id: String,
    pub tool: String,
    pub command: String,
    pub pid: u32,
    pub started_at_ms: u64,
    /// 已运行时长（毫秒）
    pub elapsed_ms: u64,
    pub retain: bool,
    pub output_path: Option<String>,
}

impl BackgroundTask {
    /// 清单视图（`now_ms` 由调用方传入，便于测试固定时间基准）
    pub fn view(&self, now_ms: u64) -> BackgroundTaskView {
        BackgroundTaskView {
            id: self.id.clone(),
            tool: self.tool.clone(),
            command: self.command.clone(),
            pid: self.pid,
            started_at_ms: self.started_at_ms,
            elapsed_ms: now_ms.saturating_sub(self.started_at_ms),
            retain: self.retain,
            output_path: self.output_path.clone(),
        }
    }
}

/// 命令文本截断（清单展示用）。
pub fn truncate_command(command: &str) -> String {
    let mut out = String::new();
    for (i, ch) in command.chars().enumerate() {
        if i >= COMMAND_PREVIEW_CHARS {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// 当前 epoch 毫秒。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ============================================================================
// 取消事实（结构化文本）
// ============================================================================

/// 工具被取消时返回给 Agent 的**结构化事实**。
///
/// 存在的理由：只回 “cancelled” 会让 Agent 认为「命令没跑过」，从而重跑长任务。
/// 模型必须能一眼分清「已终止（可重跑）」与「仍在后台跑（勿重复执行）」。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancellationNotice {
    /// 进程树是否已终止。`false` 即「仍在后台跑」。
    pub terminated: bool,
    /// 相关进程 pid（无子进程时为 None）
    pub pid: Option<u32>,
    /// 仍存活进程的输出文件位置（可用 `Read` 读取）
    pub output_path: Option<String>,
    /// 明文事实陈述（给模型读）
    pub message: String,
}

impl CancellationNotice {
    /// 命令是否仍在后台运行
    pub fn still_running(&self) -> bool {
        !self.terminated
    }

    /// 结局一：前台命令被取消 ⇒ 进程树已终止，可重跑。
    pub fn terminated(command: &str, pid: Option<u32>) -> Self {
        let pid_label = pid
            .map(|p| format!("PID {p}"))
            .unwrap_or_else(|| "无子进程".into());
        Self {
            terminated: true,
            pid,
            output_path: None,
            message: format!(
                "取消已生效：命令「{}」的前台进程（{}）已被终止，其子孙进程一并结束。\
                 该命令**没有留在后台运行**，确认需要重跑时可重新执行。",
                truncate_command(command),
                pid_label
            ),
        }
    }

    /// 结局二：显式后台保留 ⇒ 只解除等待，进程继续跑，**勿重复执行**。
    pub fn retained(command: &str, pid: Option<u32>, output_path: Option<String>) -> Self {
        let pid_label = pid
            .map(|p| format!("PID {p}"))
            .unwrap_or_else(|| "未知".into());
        let output_hint = match &output_path {
            Some(p) => format!("\n输出持续追加到：{p}（用 Read 读取，不要重跑命令）。"),
            None => "\n未开启输出文件；进度需另行确认。".to_string(),
        };
        Self {
            terminated: false,
            pid,
            output_path,
            message: format!(
                "取消已生效（等待已解除），但命令「{}」被**显式声明为后台保留**，\
                 进程（{}）仍在后台运行中 —— **勿重复执行**（重复触发会让长任务从零重跑，\
                 代价远大于继续等）。{}",
                truncate_command(command),
                pid_label,
                output_hint
            ),
        }
    }

    /// 结局三：本次调用没有启动任何子进程（如 `system_sleep` 被中止）。
    pub fn no_process(command: &str) -> Self {
        Self {
            terminated: true,
            pid: None,
            output_path: None,
            message: format!(
                "取消已生效：命令「{}」的等待已中止，本次未启动任何子进程，无残留进程。",
                truncate_command(command)
            ),
        }
    }

    /// 结局四：工具内部没有可终止的进程（桌面/浏览器等同步执行体）。
    ///
    /// 口径必须讲实情：`spawn_blocking` 不可中断，等待已解除但同步体可能仍在收尾。
    pub fn wait_released_without_kill(tool: &str) -> Self {
        Self {
            terminated: true,
            pid: None,
            output_path: None,
            message: format!(
                "取消已生效：等待已立即解除，工具 '{tool}' 的执行体已交还控制权。\
                 注意：同步执行体本身不可强制中止，它可能仍在后台收尾 —— \
                 重试前请先核对实况（文件/窗口/进程），避免重复执行。"
            ),
        }
    }
}

/// 编码取消事实：`__CANCELLED__ {单行 JSON}\n{明文}`。
pub fn encode_cancellation(notice: &CancellationNotice) -> String {
    let payload = serde_json::to_string(notice).unwrap_or_else(|_| "{}".to_string());
    format!("{CANCELLED_MARKER} {payload}\n{}", notice.message)
}

/// 解码取消事实。非本约定格式或载荷损坏时返回 `None`（调用点据此判定
/// 「这条文本不是取消事实」，不得当成取消处理）。
pub fn decode_cancellation(text: &str) -> Option<CancellationNotice> {
    let rest = text.strip_prefix(CANCELLED_MARKER)?.trim_start();
    // JSON 载荷内的换行已被 serde_json 转义为 `\n`，故首行即完整载荷
    let payload = rest.lines().next()?.trim();
    if payload.is_empty() {
        return None;
    }
    serde_json::from_str(payload).ok()
}

/// 把「已产生的输出 + 取消事实」组装成 `ToolResult`。
///
/// 口径沿用既有超时/流式路径（`sub_task_shell.rs`）：`output` 给已产生的输出，
/// `error` 给事实陈述。`ToolResult::into_exec_result()` 对失败结果取 `error`
/// （且 exit_code 为 None 时不加 `__EXIT_CODE_` 前缀），故事实一定送达 Agent。
pub fn cancelled_result(output: String, notice: &CancellationNotice) -> crate::ToolResult {
    crate::ToolResult {
        success: false,
        output: if output.trim().is_empty() {
            None
        } else {
            Some(output)
        },
        error: Some(encode_cancellation(notice)),
        exit_code: None,
    }
}

// ============================================================================
// 进程控制（可注入，便于无副作用单测）
// ============================================================================

/// 进程存活探测 / 进程树终止。抽象成 trait 的唯一理由：注册表的生命周期逻辑
/// （登记 / 注销 / 过滤死条目 / 按 id 结束 / 只杀非保留项）必须能在**不真的杀进程**
/// 的前提下被确定性验证。
pub trait ProcessControl: Send + Sync {
    /// pid 是否仍然存活
    fn is_alive(&self, pid: u32) -> bool;
    /// 终止 pid 及其子孙进程
    fn kill_tree(&self, pid: u32) -> std::result::Result<(), String>;
}

/// 真实实现。
///
/// - Windows：`taskkill /PID <pid> /T /F`。**只按 pid 且带 `/T`**，禁止
///   `taskkill /IM`（会误杀用户自己开的同名进程）。
/// - Unix：先 `kill -9 <pid>`，再 `pkill -9 -P <pid>` 扫直接子进程。**没有进程组
///   （pgid）概念 ⇒ 孙进程可能存活**，这是已知盲区，未做 setsid 改造。
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcessControl;

impl ProcessControl for SystemProcessControl {
    fn is_alive(&self, pid: u32) -> bool {
        is_process_alive(pid)
    }
    fn kill_tree(&self, pid: u32) -> std::result::Result<(), String> {
        kill_process_tree(pid)
    }
}

/// 按 pid 杀进程树（含宿主自身保护）。
pub fn kill_process_tree(pid: u32) -> std::result::Result<(), String> {
    if pid == 0 {
        return Err("非法 pid: 0".to_string());
    }
    // 防御性自保：注册表条目按契约只可能是子进程，但 pid 回收后可能撞上宿主自身。
    if pid == std::process::id() {
        return Err("拒绝终止宿主自身进程".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("taskkill");
        cmd.arg("/PID")
            .arg(pid.to_string())
            .arg("/T")
            .arg("/F")
            .creation_flags(CREATE_NO_WINDOW);
        let output = cmd
            .output()
            .map_err(|e| format!("taskkill 启动失败: {e}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.success() {
            Ok(())
        } else {
            let detail = format!("{}{}", stdout.trim(), stderr.trim());
            Err(format!("taskkill 失败: {}", detail.trim().to_string()))
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .output()
            .map_err(|e| format!("kill 启动失败: {e}"))?;
        // 直接子进程尽力扫一遍（Unix 无 pgid ⇒ 孙进程不可达，见模块文档）
        let _ = std::process::Command::new("pkill")
            .args(["-9", "-P", &pid.to_string()])
            .output();
        Ok(())
    }
}

/// pid 是否仍然存活（对外可查：单测与上层诊断都用它核实「进程真的没了」）。
///
/// **开销**：Windows 走 `tasklist /FI "PID eq N"`，一次进程 spawn（约 30–100ms）。
/// 故只用于用户触发路径（列清单 / interrupt 汇总 / kill_one）与测试，
/// 不得放进工具的 150ms 轮询循环（那里用 `Child::try_wait()`，无 spawn）。
///
/// 探测失败（tasklist 起不来 / 输出无法解析）时返回 `false` —— 宁可漏杀，
/// 不可把「探测不了」当成「进程还在」而对着用户进程下狠手。
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let filter = format!("PID eq {pid}");
        let mut cmd = std::process::Command::new("tasklist");
        cmd.args(["/FI", &filter, "/NH", "/FO", "CSV"])
            .creation_flags(CREATE_NO_WINDOW);
        let Ok(output) = cmd.output() else {
            return false;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().any(|line| {
            let mut parts = line.split(',');
            let _image = parts.next();
            parts
                .next()
                .map(|p| p.trim().trim_matches('"') == pid.to_string())
                .unwrap_or(false)
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "pid="])
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false)
    }
}

// ============================================================================
// 注册表
// ============================================================================

/// `kill_foreground` 的结算报告。**只报实情**：被杀掉的、被发现已经自己死掉的、
/// 杀失败的、以及**仍然存活的保留项**，各自计数。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KillReport {
    /// 已确认终止的前台进程
    pub killed: Vec<BackgroundTask>,
    /// 取消/结束时已经自己退出、被顺带清理的条目数
    pub already_gone: usize,
    /// 终止失败的条目（id / pid / 原因）
    pub failed: Vec<(String, u32, String)>,
    /// **仍然存活**的保留项数量——这些**不会**被自动杀掉，必须如实上报
    pub retained: usize,
}

/// `kill_one` 的结局。
#[derive(Debug, Clone, PartialEq)]
pub struct KillOutcome {
    pub task: BackgroundTask,
    /// true = 下刀时进程已经自己退出（无残留）
    pub already_gone: bool,
}

/// 进程级后台任务注册表。
pub struct BackgroundTaskRegistry {
    tasks: Mutex<HashMap<String, BackgroundTask>>,
    seq: AtomicU64,
    control: Box<dyn ProcessControl>,
}

impl Default for BackgroundTaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundTaskRegistry {
    /// 真实进程控制的实例（生产唯一构造路径）
    pub fn new() -> Self {
        Self::with_control(Box::new(SystemProcessControl))
    }

    /// 注入进程控制（测试专用：让生命周期逻辑可在不真的杀进程的前提下验证）
    pub fn with_control(control: Box<dyn ProcessControl>) -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(0),
            control,
        }
    }

    /// 锁中毒时退回内部值：注册表是纯账本，panic 不该让它永久不可用
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, BackgroundTask>> {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 生成下一个任务 id（`bg-<seq>`，进程内单调递增）
    pub fn next_task_id(&self) -> String {
        format!("bg-{}", self.seq.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// 登记一个后台进程。**子进程一启动就必须登记**——否则没人知道该杀谁。
    pub fn register(&self, task: BackgroundTask) {
        self.lock().insert(task.id.clone(), task);
    }

    /// 注销。返回被移除的条目（不存在则 `None`）。
    pub fn unregister(&self, id: &str) -> Option<BackgroundTask> {
        self.lock().remove(id)
    }

    /// 更新 retain 意图（超时转后台 / 取消时保留都会走这里）
    pub fn set_retain(&self, id: &str, retain: bool) -> bool {
        match self.lock().get_mut(id) {
            Some(task) => {
                task.retain = retain;
                true
            }
            None => false,
        }
    }

    /// 取条目快照
    pub fn get(&self, id: &str) -> Option<BackgroundTask> {
        self.lock().get(id).cloned()
    }

    /// **过滤死条目后的存活清单**（顺带把死条目清出账本）。
    ///
    /// 开销：每条目一次存活探测（Windows 一次 tasklist spawn）。
    pub fn list(&self) -> Vec<BackgroundTask> {
        self.list_and_prune().0
    }

    /// [`BackgroundTaskRegistry::list`] + 死条目清理条数
    pub fn list_and_prune(&self) -> (Vec<BackgroundTask>, usize) {
        let mut tasks = self.lock();
        let dead: Vec<String> = tasks
            .iter()
            .filter(|(_, t)| !self.control.is_alive(t.pid))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &dead {
            tasks.remove(id);
        }
        let mut alive: Vec<BackgroundTask> = tasks.values().cloned().collect();
        alive.sort_by_key(|t| t.started_at_ms);
        (alive, dead.len())
    }

    /// 清理死条目，返回清理条数
    pub fn prune_dead(&self) -> usize {
        self.list_and_prune().1
    }

    /// 仍然存活的**保留项**数量（interrupt 如实汇报用）
    pub fn retained_count(&self) -> usize {
        self.list().iter().filter(|t| t.retain).count()
    }

    /// 按 id 结束某个后台任务（**含保留项**——这是显式意图，允许）。
    pub fn kill_one(&self, id: &str) -> std::result::Result<KillOutcome, String> {
        let Some(task) = self.get(id) else {
            return Err(format!("后台任务 {id} 不存在或已结束"));
        };
        match self.control.kill_tree(task.pid) {
            Ok(()) => {
                self.unregister(id);
                Ok(KillOutcome {
                    task,
                    already_gone: false,
                })
            }
            Err(_e) if !self.control.is_alive(task.pid) => {
                // 进程已自行退出：下刀失败不是失败，只是无事可做
                self.unregister(id);
                Ok(KillOutcome {
                    task,
                    already_gone: true,
                })
            }
            Err(e) => Err(format!("结束后台任务 {id}（PID {}）失败: {e}", task.pid)),
        }
    }

    /// **取消路径专用**：终止全部 `retain == false` 的前台进程。
    ///
    /// 保留项一律不碰 —— 不替用户杀「保留项」是硬约束；它们的数量在
    /// [`KillReport::retained`] 里如实上报，由上层决定如何汇报。
    pub fn kill_foreground(&self) -> KillReport {
        let mut report = KillReport::default();
        let targets: Vec<BackgroundTask> = {
            let tasks = self.lock();
            tasks.values().filter(|t| !t.retain).cloned().collect()
        };
        for task in targets {
            match self.control.kill_tree(task.pid) {
                Ok(()) => {
                    self.unregister(&task.id);
                    report.killed.push(task);
                }
                Err(e) => {
                    if self.control.is_alive(task.pid) {
                        report.failed.push((task.id.clone(), task.pid, e));
                    } else {
                        self.unregister(&task.id);
                        report.already_gone += 1;
                    }
                }
            }
        }
        report.retained = self.retained_count();
        report
    }
}

// ============================================================================
// 进程级单例
// ============================================================================

static GLOBAL: OnceLock<BackgroundTaskRegistry> = OnceLock::new();

/// 全进程唯一的注册表实例。
///
/// 单例而非注入的理由：任务的**生命周期边界就是工具调用边界**，而 registry
/// （Leader / Exec / workflow / 插件）是多份的、要各自 clone 的——注入会让
/// 「谁拥有账本」变成构造顺序问题。账本本身是纯进程级事实，一份即可。
pub fn global() -> &'static BackgroundTaskRegistry {
    GLOBAL.get_or_init(BackgroundTaskRegistry::new)
}

/// 后台任务输出日志目录（`<data_dir>/background_tasks/`）
pub fn background_log_dir() -> PathBuf {
    crate::utils::nuphus_data_dir().join("background_tasks")
}

/// 建一个空的输出日志文件，返回路径。
///
/// 拿不到文件**不拒绝执行命令**（返回 `None` 即可）——命令能跑比有输出位置重要。
/// 返回路径而非句柄：stdout / stderr 需要**两个独立句柄**追加进同一文件
/// （`std::fs::File` 不实现 `Clone`，共享同一句柄会让两个读者线程互抢偏移）。
pub fn open_log_file(id: &str) -> Option<PathBuf> {
    let dir = background_log_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{id}.log"));
    std::fs::File::options()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .ok()?;
    Some(path)
}

/// 尽力删除条目的输出日志（保留项不删——那正是它还能被读取的依据）。
/// Windows 上文件仍被读取线程持有时删除会失败，属预期，忽略即可。
pub fn discard_log(task: &BackgroundTask) {
    if task.retain {
        return;
    }
    let Some(ref path) = task.output_path else {
        return;
    };
    if let Err(e) = std::fs::remove_file(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::debug!("[BG] 删除输出日志失败 {}: {e}", path);
        }
    }
}

// ============================================================================
// 单测
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex as StdMutex};

    /// 假进程控制：可克隆的观察句柄 + 共享状态。全程不碰真实进程。
    #[derive(Default)]
    struct FakeState {
        alive: StdMutex<HashSet<u32>>,
        killed: StdMutex<Vec<u32>>,
        /// 这些 pid 的 kill 永远失败（模拟「杀不掉」：权限不足 / 已脱离进程树）
        unkillable: StdMutex<HashSet<u32>>,
    }

    #[derive(Clone, Default)]
    struct FakeControl(Arc<FakeState>);

    impl FakeControl {
        fn new(alive: &[u32], unkillable: &[u32]) -> Self {
            Self(Arc::new(FakeState {
                alive: StdMutex::new(alive.iter().copied().collect()),
                killed: StdMutex::new(Vec::new()),
                unkillable: StdMutex::new(unkillable.iter().copied().collect()),
            }))
        }
        /// 注册表已持有 clone，这里给断言用的观察句柄
        fn observer(&self) -> FakeControl {
            self.clone()
        }
        fn kill_calls(&self) -> Vec<u32> {
            self.0.killed.lock().unwrap().clone()
        }
        fn mark_dead(&self, pid: u32) {
            self.0.alive.lock().unwrap().remove(&pid);
        }
    }

    impl ProcessControl for FakeControl {
        fn is_alive(&self, pid: u32) -> bool {
            self.0.alive.lock().unwrap().contains(&pid)
        }
        fn kill_tree(&self, pid: u32) -> std::result::Result<(), String> {
            self.0.killed.lock().unwrap().push(pid);
            if self.0.unkillable.lock().unwrap().contains(&pid) {
                return Err("模拟失败: 杀不掉".to_string());
            }
            let mut alive = self.0.alive.lock().unwrap();
            if !alive.contains(&pid) {
                // 与真实实现同形：进程不存在时 taskkill / kill 都会失败。
                // 少了这条，FakeControl 会把「进程早退了」也报成终止成功，
                // 于是 already_gone 分支永远测不到。
                return Err("模拟失败: 进程不存在".to_string());
            }
            alive.remove(&pid);
            Ok(())
        }
    }

    fn task(id: &str, pid: u32, retain: bool) -> BackgroundTask {
        BackgroundTask {
            id: id.to_string(),
            tool: "system_shell".to_string(),
            command: "cargo build --release".to_string(),
            pid,
            started_at_ms: 1_000,
            retain,
            output_path: Some(format!("C:/tmp/{id}.log")),
        }
    }

    // ── 注册表生命周期 ──────────────────────────────────────────────────

    #[test]
    fn register_then_get_and_unregister() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::new(&[4242], &[])));
        reg.register(task("bg-1", 4242, false));
        assert_eq!(reg.get("bg-1").map(|t| t.pid), Some(4242));
        assert_eq!(reg.unregister("bg-1").map(|t| t.pid), Some(4242));
        assert!(reg.get("bg-1").is_none());
        assert!(reg.unregister("bg-1").is_none(), "重复注销必须返回 None");
    }

    #[test]
    fn list_filters_and_prunes_dead_entries() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::new(&[111], &[])));
        reg.register(task("bg-1", 111, false));
        reg.register(task("bg-2", 222, false)); // 222 从未「活」→ 视为已死

        let (alive, pruned) = reg.list_and_prune();
        assert_eq!(pruned, 1);
        assert_eq!(alive.len(), 1);
        assert_eq!(alive[0].id, "bg-1");
        assert!(reg.get("bg-2").is_none(), "死条目必须被清出账本");
    }

    #[test]
    fn next_task_id_is_unique_and_monotonic() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::default()));
        assert_eq!(reg.next_task_id(), "bg-1");
        assert_eq!(reg.next_task_id(), "bg-2");
    }

    #[test]
    fn kill_one_terminates_even_retained_entry() {
        let ctl = FakeControl::new(&[777], &[]);
        let observer = ctl.observer();
        let reg = BackgroundTaskRegistry::with_control(Box::new(ctl));
        reg.register(task("bg-9", 777, true)); // 保留项：自动路径不杀，显式按 id 结束允许

        let outcome = reg.kill_one("bg-9").expect("显式结束应成功");
        assert!(!outcome.already_gone);
        assert_eq!(outcome.task.pid, 777);
        assert_eq!(observer.kill_calls(), vec![777]);
        assert!(reg.get("bg-9").is_none());
    }

    #[test]
    fn kill_one_reports_already_gone_when_process_exited_on_its_own() {
        let ctl = FakeControl::new(&[888], &[]);
        let observer = ctl.observer();
        let reg = BackgroundTaskRegistry::with_control(Box::new(ctl));
        reg.register(task("bg-2", 888, false));
        observer.mark_dead(888); // 进程自己退了 → kill_tree 失败但不该算失败

        let outcome = reg.kill_one("bg-2").expect("进程已死不算失败");
        assert!(outcome.already_gone);
        assert!(reg.get("bg-2").is_none());
    }

    #[test]
    fn kill_one_reports_real_failure_when_process_still_alive() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::new(&[999], &[999])));
        reg.register(task("bg-3", 999, false));
        let err = reg.kill_one("bg-3").expect_err("杀不掉必须如实报错");
        assert!(err.contains("999"), "报错必须带 pid 便于排查: {err}");
    }

    #[test]
    fn kill_one_unknown_id_is_error() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::default()));
        assert!(reg.kill_one("bg-nope").is_err());
    }

    // ── 取消路径：只杀非保留项 ──────────────────────────────────────────

    #[test]
    fn kill_foreground_only_kills_non_retained() {
        let ctl = FakeControl::new(&[10, 20], &[]);
        let observer = ctl.observer();
        let reg = BackgroundTaskRegistry::with_control(Box::new(ctl));
        reg.register(task("fg", 10, false));
        reg.register(task("keep", 20, true));

        let report = reg.kill_foreground();

        assert_eq!(report.killed.len(), 1);
        assert_eq!(report.killed[0].id, "fg");
        assert_eq!(report.retained, 1, "保留项必须仍然存活并被计数");
        assert!(report.failed.is_empty());
        // 保留项绝不能出现在终止记录里
        assert_eq!(observer.kill_calls(), vec![10]);
        assert!(reg.get("keep").is_some(), "保留项不得被注销");
        assert!(reg.get("fg").is_none());
    }

    #[test]
    fn kill_foreground_separates_already_gone_from_real_failures() {
        let ctl = FakeControl::new(&[30, 40], &[40]);
        let observer = ctl.observer();
        let reg = BackgroundTaskRegistry::with_control(Box::new(ctl));
        reg.register(task("gone", 30, false));
        reg.register(task("stuck", 40, false));
        observer.mark_dead(30); // 自己退了

        let report = reg.kill_foreground();
        assert_eq!(report.already_gone, 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].1, 40);
        assert_eq!(report.killed.len(), 0);
        assert!(
            reg.get("stuck").is_some(),
            "杀不掉的条目必须留账，如实上报失败"
        );
    }

    #[test]
    fn kill_foreground_on_empty_registry_is_noop() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::default()));
        let report = reg.kill_foreground();
        assert_eq!(report, KillReport::default());
    }

    #[test]
    fn set_retain_switches_entry_into_protected_mode() {
        let reg = BackgroundTaskRegistry::with_control(Box::new(FakeControl::new(&[50], &[])));
        reg.register(task("t", 50, false));
        assert_eq!(reg.retained_count(), 0);
        assert!(reg.set_retain("t", true));
        assert_eq!(reg.retained_count(), 1);

        let report = reg.kill_foreground();
        assert!(report.killed.is_empty(), "转为保留后取消不得再杀");
        assert_eq!(report.retained, 1);
        assert!(!reg.set_retain("missing", true));
    }

    // ── 取消事实的编码 / 解码 ──────────────────────────────────────────

    #[test]
    fn cancellation_roundtrip_terminated() {
        let notice = CancellationNotice::terminated("cargo build --release", Some(4321));
        let text = encode_cancellation(&notice);
        assert!(text.starts_with(CANCELLED_MARKER));
        assert!(text.contains("PID 4321"));
        assert!(!notice.still_running());
        assert_eq!(decode_cancellation(&text).as_ref(), Some(&notice));
    }

    #[test]
    fn cancellation_roundtrip_retained_carries_pid_and_output() {
        let notice = CancellationNotice::retained(
            "yarn build",
            Some(555),
            Some("C:/data/background_tasks/bg-3.log".to_string()),
        );
        let text = encode_cancellation(&notice);
        assert!(notice.still_running(), "保留项必须如实报告「仍在跑」");
        assert!(text.contains("勿重复执行"));
        assert!(text.contains("bg-3.log"));
        let decoded = decode_cancellation(&text).expect("必须可解码");
        assert_eq!(decoded.pid, Some(555));
        assert_eq!(
            decoded.output_path.as_deref(),
            Some("C:/data/background_tasks/bg-3.log")
        );
        assert!(!decoded.terminated);
        assert_eq!(decoded, notice);
    }

    #[test]
    fn decode_rejects_foreign_text() {
        assert!(decode_cancellation("普通失败").is_none());
        assert!(decode_cancellation("__EXIT_CODE:1__ boom").is_none());
        assert!(decode_cancellation("__CANCELLED__ not-json").is_none());
        assert!(decode_cancellation("__CANCELLED__ \n明文").is_none());
    }

    #[test]
    fn no_process_notice_carries_no_pid() {
        let notice = CancellationNotice::no_process("system_sleep 30");
        assert!(!notice.still_running());
        assert_eq!(notice.pid, None);
        let decoded = decode_cancellation(&encode_cancellation(&notice)).unwrap();
        assert_eq!(decoded, notice);
    }

    #[test]
    fn cancelled_result_places_fact_in_error_field() {
        let notice = CancellationNotice::terminated("ping", Some(7));
        let r = cancelled_result("半截输出".to_string(), &notice);
        assert!(!r.success);
        assert_eq!(r.output.as_deref(), Some("半截输出"));
        assert!(r.error.as_deref().unwrap().starts_with(CANCELLED_MARKER));
        assert_eq!(r.exit_code, None);
        // into_exec_result 是失败结果的事实送达通道
        let err = r.into_exec_result().expect_err("取消必须以失败形态上抛");
        assert!(decode_cancellation(&err).is_some());
    }

    #[test]
    fn cancelled_result_with_blank_output_has_no_output_field() {
        let r = cancelled_result("   ".to_string(), &CancellationNotice::no_process("s"));
        assert!(r.output.is_none());
    }

    // ── 工具函数 ────────────────────────────────────────────────────────

    #[test]
    fn truncate_command_keeps_short_text_verbatim() {
        assert_eq!(truncate_command("echo hi"), "echo hi");
    }

    #[test]
    fn truncate_command_marks_overflow() {
        let long = "x".repeat(COMMAND_PREVIEW_CHARS + 50);
        let t = truncate_command(&long);
        assert_eq!(t.chars().count(), COMMAND_PREVIEW_CHARS + 1);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn truncate_command_is_char_safe_on_multibyte() {
        let t = truncate_command(&"中".repeat(COMMAND_PREVIEW_CHARS + 10));
        assert!(t.ends_with('…'), "不得按字节切出半个字符");
        assert_eq!(t.chars().count(), COMMAND_PREVIEW_CHARS + 1);
    }

    #[test]
    fn view_reports_elapsed_from_fixed_now() {
        let v = task("bg-1", 1, true).view(4_000);
        assert_eq!(v.elapsed_ms, 3_000);
        assert!(v.retain);
    }

    #[test]
    fn kill_process_tree_refuses_host_self_and_zero() {
        assert!(kill_process_tree(0).is_err());
        assert!(kill_process_tree(std::process::id()).is_err());
    }

    #[test]
    fn discard_log_keeps_file_for_retained_entries() {
        let dir = background_log_dir();
        std::fs::create_dir_all(&dir).expect("建日志目录");
        let path = dir.join("__test_discard_keep.log");
        std::fs::write(&path, b"x").expect("写测试日志");

        let mut retained = task("bg-keep", 1, true);
        retained.output_path = Some(path.to_string_lossy().to_string());
        discard_log(&retained);
        assert!(
            path.exists(),
            "保留项的输出文件是它还能被读取的依据，不得删"
        );

        let mut foreground = retained.clone();
        foreground.retain = false;
        discard_log(&foreground);
        assert!(!path.exists(), "前台项收尾后应清理输出文件");
    }

    /// 真进程：验证存活探测在当前平台确实可用（本模块唯一的真实副作用用例）
    #[test]
    fn is_process_alive_detects_a_real_child_process() {
        #[cfg(target_os = "windows")]
        let child = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 20",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
        };
        #[cfg(not(target_os = "windows"))]
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 20"])
            .spawn();

        let Ok(mut child) = child else {
            // 环境没有可用 shell（精简 CI 容器）→ 无法验证，如实跳过而非伪造通过
            eprintln!("[test] 当前环境无法启动 shell 子进程，跳过存活探测实测");
            return;
        };
        assert!(
            is_process_alive(child.id()),
            "刚启动的子进程必须被判定为存活"
        );
        let _ = child.kill();
        let _ = child.wait();
    }
}
