//! background — 后台任务（跨工具调用存活的子进程）管理命令
//!
//! 数据源只有一个：`nuphus::tools::background_tasks::global()`（进程级单例账本）。
//! 本文件**只做传输**，不做任何状态推断——面板/上层看到什么完全取决于账本。
//!
//! 语义固定：
//! - 列举：只返回**存活**条目（死条目在读取时被顺带清理），每项含命令 / pid /
//!   已跑时长 / retain / 输出文件位置。
//! - 结束：按 id 终止该任务的**整个进程树**。这是显式意图，因此**包含保留项**
//!   ——与「取消只杀前台」的自动路径互为对照。

use nuphus::tools::background_tasks::{self, BackgroundTaskView};
use tauri::State;

use crate::state::AppState;

/// List still-running background tasks (dead entries are pruned on read).
#[tauri::command]
pub fn list_background_tasks() -> Result<Vec<BackgroundTaskView>, String> {
    let now = background_tasks::now_ms();
    Ok(background_tasks::global()
        .list()
        .into_iter()
        .map(|t| t.view(now))
        .collect())
}

/// Terminate one background task (including retained ones) by its registry id.
#[tauri::command]
pub fn kill_background_task(state: State<'_, AppState>, id: String) -> Result<String, String> {
    // 置位取消旗标：同一次「用户想停下来」的语义里，正在跑的工具也应当尽快脱身，
    // 否则用户点完「结束这个任务」还要再点一次「中断」才肯停。
    state
        .cancel_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let outcome = background_tasks::global().kill_one(&id)?;
    let verb = if outcome.already_gone {
        "已自行结束（清理账本条目）"
    } else {
        "已终止（含子孙进程）"
    };
    tracing::info!(
        "[BG] kill_background_task {}: pid={} {}",
        id,
        outcome.task.pid,
        verb
    );
    Ok(format!(
        "后台任务 {}（{}，PID {}）{}",
        id, outcome.task.command, outcome.task.pid, verb
    ))
}

/// Count of still-running **retained** background tasks.
///
/// 单列一个命令的理由：中断汇报需要「保留 M 个」这个数字，而列举是重操作
/// （每条目一次存活探测）。这里仍走同一本账，只是过滤后计数。
#[tauri::command]
pub fn count_retained_background_tasks(_state: State<'_, AppState>) -> Result<usize, String> {
    Ok(background_tasks::global().retained_count())
}
