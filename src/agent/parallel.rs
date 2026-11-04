//! parallel — 临时伙伴（Side Chat）与主会话的**进程级并行协作契约**
//!
//! # 为什么存在这个模块
//!
//! 并行协作有两个方向的「互相感知」需求，而两边的真实状态都藏在别人的模块里：
//! 侧边要知道「主会话是否正在派发 Exec」（避免读写同一工作区时互相覆盖），
//! Leader 要知道「用户是否正在伙伴面板并行操作」（派发时提醒避让）。若各写
//! 各的标志位，就会出现「侧边以为主会话闲着、Leader 以为用户没开面板」的
//! 双重误判——与执行态收敛前 5 个布尔源互为真相是同一类事故。
//!
//! 于是本模块做**进程级单一事实源**：侧边在途 / Exec 在途 / 曾用信号 + 待交付箱。
//! 写入方只有四个既有路径（不新增入口、不新增命令）：
//! - Exec 在途：`agent::task_run::RunGuard`（RAII，覆盖全部退出路径）；
//! - 侧边在途：`commands::side_chat` 的 send 起点与 `ReturnGuard::drop`；
//! - 待交付箱：`commands::side_chat` 的自动交付被 Finalizing 拒收时。
//!
//! # 纪律
//!
//! - 两把全局锁都**只在同步段取放**：绝不持锁跨 await，绝不在锁内调用
//!   `submit_user_message` 之类的重活（交付箱只做 FIFO 存取，投递在锁外）；
//! - 清理一律**带身份匹配**（key / run_id）：并行面板与会话壳重建都会让
//!   「迟到的 drop」落在新轮次之后，不匹配就绝不清——防误清新状态；
//! - 锁中毒时降级（`unwrap_or_else(|e| e.into_inner())`）：快照照常返回、
//!   清理退化为空操作，绝不 panic 把并行协作的故障升级成进程故障。

use std::sync::{Mutex, OnceLock};

/// 伙伴在途：某个临时会话面板正在执行（连续轮次）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideOcc {
    /// 临时会话 key（前端下发的 session_key，随面板开合生灭）。
    pub key: String,
    /// 连续在途轮次数——「用户正在这个面板里连着问」的深度信号。
    pub turns: u32,
}

/// Exec 在途：主会话派发的一次执行正在跑。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOcc {
    /// 生命周期台账身份（`task_run` 下发），清理时按它匹配。
    pub run_id: String,
    /// 任务标题（给人看的那份，注入避让文案用）。
    pub title: String,
}

/// 并行状态快照。**clone 出的值，不是引用**——调用方持多久都不会死锁，
/// 也不会看到半更新的中间态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParallelSnapshot {
    pub side: Option<SideOcc>,
    pub exec: Option<ExecOcc>,
    /// 进程内曾出现过伙伴轮次（用户习惯信号，启动重置，不复位）。
    pub side_ever_used: bool,
}

/// 一条待交付的伙伴结论（Finalizing 窗口拒收后落箱，伙伴下一轮完成时补投）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryItem {
    /// 前端轮次 id（投递 send_id 与文案都从它派生，天然防重）。
    pub turn_id: String,
    /// 结论全文（已 normalize 的回复）。
    pub reply: String,
    /// 轮次完成时刻（epoch ms 字符串，落箱排查用）。
    pub ts: String,
}

/// 内部状态：两个在途槽 + 一个曾用信号。单槽（非 Vec）是有意的——
/// 主循环同步派发决定了 Exec 同时只可能有一个在途；并行面板各有 key，
/// 侧边槽记录「当前正在跑的那个」。
#[derive(Default)]
struct ParallelState {
    side: Option<SideOcc>,
    exec: Option<ExecOcc>,
    side_ever_used: bool,
}

fn parallel_state() -> &'static Mutex<ParallelState> {
    static PARALLEL: OnceLock<Mutex<ParallelState>> = OnceLock::new();
    PARALLEL.get_or_init(|| Mutex::new(ParallelState::default()))
}

fn delivery_box() -> &'static Mutex<Vec<DeliveryItem>> {
    static DELIVERY_BOX: OnceLock<Mutex<Vec<DeliveryItem>>> = OnceLock::new();
    DELIVERY_BOX.get_or_init(|| Mutex::new(Vec::new()))
}

/// 取当前并行状态快照（clone）。
pub fn snapshot() -> ParallelSnapshot {
    let st = parallel_state().lock().unwrap_or_else(|e| e.into_inner());
    ParallelSnapshot {
        side: st.side.clone(),
        exec: st.exec.clone(),
        side_ever_used: st.side_ever_used,
    }
}

/// 伙伴一轮开始：同 key 累加连续轮次，换 key 重建计数；曾用信号一次性置位。
pub fn set_side_start(key: &str) {
    let mut st = parallel_state().lock().unwrap_or_else(|e| e.into_inner());
    st.side_ever_used = true;
    if st.side.as_ref().map(|s| s.key.as_str()) == Some(key) {
        if let Some(s) = st.side.as_mut() {
            s.turns = s.turns.saturating_add(1);
        }
    } else {
        st.side = Some(SideOcc {
            key: key.to_string(),
            turns: 1,
        });
    }
}

/// 伙伴一轮结束（`ReturnGuard::drop`）。**key 匹配才清**：并行面板各有 key，
/// 且壳重建会让迟到的 drop 落在同 key 的新轮次之后——不匹配就绝不清。
pub fn clear_side(key: &str) {
    let mut st = parallel_state().lock().unwrap_or_else(|e| e.into_inner());
    if st.side.as_ref().map(|s| s.key.as_str()) == Some(key) {
        st.side = None;
    }
}

/// Exec 一笔开跑（`RunGuard::new`）。单槽覆盖：同轮只可能有一个在途。
pub fn set_exec(run_id: &str, title: &str) {
    let mut st = parallel_state().lock().unwrap_or_else(|e| e.into_inner());
    st.exec = Some(ExecOcc {
        run_id: run_id.to_string(),
        title: title.to_string(),
    });
}

/// Exec 一笔收官（settle / Drop 兜底）。**run_id 匹配才清**：迟到兜底绝不能
/// 把更新一笔的在途状态清掉。
pub fn clear_exec(run_id: &str) {
    let mut st = parallel_state().lock().unwrap_or_else(|e| e.into_inner());
    if st.exec.as_ref().map(|e| e.run_id.as_str()) == Some(run_id) {
        st.exec = None;
    }
}

/// 结论落箱（FIFO 尾部）。
pub fn enqueue_delivery(item: DeliveryItem) {
    delivery_box()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(item);
}

/// 全取空（FIFO 头部先出）。
pub fn take_deliveries() -> Vec<DeliveryItem> {
    std::mem::take(&mut *delivery_box().lock().unwrap_or_else(|e| e.into_inner()))
}

/// 箱内积压条数（前端「待交付 N」角标的数据源）。
pub fn delivery_pending() -> usize {
    delivery_box()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 并行状态是全进程共享的：本模块用例之间必须串行，否则互相覆盖槽位。
    /// 中毒也恢复（`into_inner`）——一个用例 panic 不连坐其余用例。
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Exec 槽覆盖语义 + clear 的 run_id 匹配语义（不匹配绝不清）。
    #[test]
    fn exec_slot_overwrites_and_clear_matches_run_id() {
        let _g = lock();
        let a = "run-parallel-exec-a";
        let b = "run-parallel-exec-b";
        set_exec(a, "任务A");
        assert_eq!(
            snapshot().exec.as_ref().map(|e| e.title.as_str()),
            Some("任务A")
        );
        set_exec(b, "任务B");
        let snap = snapshot();
        assert_eq!(snap.exec.as_ref().map(|e| e.run_id.as_str()), Some(b));
        assert_eq!(snap.exec.as_ref().map(|e| e.title.as_str()), Some("任务B"));

        // 迟到兜底拿着旧 run_id 来清：绝不能动新的一笔
        clear_exec(a);
        assert!(snapshot().exec.is_some(), "不匹配的 run_id 不得误清");
        clear_exec(b);
        assert!(snapshot().exec.is_none());
    }

    /// 同 key 连续轮次累加；换 key 从 1 重新计。
    #[test]
    fn side_turns_accumulate_for_same_key() {
        let _g = lock();
        let k = "side-parallel-turns";
        set_side_start(k);
        assert_eq!(snapshot().side.as_ref().map(|s| s.turns), Some(1));
        set_side_start(k);
        set_side_start(k);
        let snap = snapshot();
        assert_eq!(snap.side.as_ref().map(|s| s.key.as_str()), Some(k));
        assert_eq!(snap.side.as_ref().map(|s| s.turns), Some(3));
        clear_side(k);
        assert!(snapshot().side.is_none());

        // 换 key 是新的一段并行，不继承上一个 key 的轮数
        let k2 = "side-parallel-turns-2";
        set_side_start(k2);
        assert_eq!(snapshot().side.as_ref().map(|s| s.turns), Some(1));
        clear_side(k2);
    }

    /// 不同 key 互清不动：并行开着的另一个面板的迟到放回，不能清掉本面板。
    #[test]
    fn clear_side_only_clears_matching_key() {
        let _g = lock();
        let k1 = "side-parallel-clear-1";
        let k2 = "side-parallel-clear-2";
        set_side_start(k1);
        set_side_start(k2);
        assert_eq!(snapshot().side.as_ref().map(|s| s.key.as_str()), Some(k2));

        clear_side(k1);
        let snap = snapshot();
        assert_eq!(snap.side.as_ref().map(|s| s.key.as_str()), Some(k2));
        assert_eq!(snap.side.as_ref().map(|s| s.turns), Some(1));

        clear_side(k2);
        assert!(snapshot().side.is_none());
    }

    /// 曾用信号一次性置位、清在途也不复位（进程内用户习惯，启动重置）。
    #[test]
    fn side_ever_used_latches_and_never_resets() {
        let _g = lock();
        let k = "side-parallel-ever";
        set_side_start(k);
        assert!(snapshot().side_ever_used, "出现过伙伴轮次必须置位");
        clear_side(k);
        let snap = snapshot();
        assert!(snap.side.is_none());
        assert!(
            snap.side_ever_used,
            "在途清了，曾用信号也必须留着（用户习惯不随单轮翻转）"
        );
    }

    /// 交付箱 FIFO：take 全取空，顺序即入箱顺序。
    #[test]
    fn delivery_box_is_fifo_and_take_empties() {
        let _g = lock();
        let ids: Vec<String> = (0..3).map(|i| format!("turn-parallel-fifo-{i}")).collect();
        for (i, id) in ids.iter().enumerate() {
            enqueue_delivery(DeliveryItem {
                turn_id: id.clone(),
                reply: format!("结论{i}"),
                ts: "0".to_string(),
            });
        }
        assert!(delivery_pending() >= ids.len());

        let taken = take_deliveries();
        let mine: Vec<&DeliveryItem> = taken
            .iter()
            .filter(|it| it.turn_id.starts_with("turn-parallel-fifo-"))
            .collect();
        let got: Vec<&str> = mine.iter().map(|it| it.turn_id.as_str()).collect();
        assert_eq!(got, ids.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        assert_eq!(delivery_pending(), 0, "take 必须取空");
    }

    /// 快照是隔离副本：旧快照不随后续清理变化（证明返回的是 clone 而非引用）。
    #[test]
    fn snapshot_is_an_isolated_copy() {
        let _g = lock();
        let k = "side-parallel-snap";
        let e = "run-parallel-snap";
        set_side_start(k);
        set_exec(e, "快照隔离测试");
        let before = snapshot();
        assert!(before.side.is_some());
        assert!(before.exec.is_some());

        clear_side(k);
        clear_exec(e);
        let after = snapshot();
        assert!(after.side.is_none());
        assert!(after.exec.is_none());
        assert!(
            before.side.is_some() && before.exec.is_some(),
            "已取出的快照不得被后续清理改写"
        );
    }

    /// 清理路径幂等：对不存在的 key / run_id 是空操作，空箱 take 不 panic。
    #[test]
    fn clear_paths_are_idempotent() {
        let _g = lock();
        clear_side("side-parallel-absent");
        clear_exec("run-parallel-absent");
        clear_side("side-parallel-absent");
        let _ = take_deliveries();
        assert_eq!(delivery_pending(), 0);
    }
}
