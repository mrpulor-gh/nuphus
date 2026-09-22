//! automation_gate — 后端**资源级互斥门**（桌面自动化 / 浏览器控制 / 屏幕录制 / 主执行体）
//!
//! ## 为什么需要它
//!
//! 这些能力背后是**独占的机器资源**，进程内并发访问会直接 panic / 永久死锁：
//! - `nuphus-browser::shared::shared_client()` 是**进程级单例**（`OnceLock`），
//!   page 又是 `Arc<tokio::sync::Mutex<Page>>` —— 不可重入的 tokio Mutex 一旦被
//!   两条控制链并发持有/重入，就是永久死锁（`client.rs:2395` 注释原文）。
//! - 桌面输入（enigo / Win32 SendInput）与浏览器 CDP 都是物理独占：交错的输入序列
//!   会让「Agent 的自动化步骤」与「用户手动操作」互相污染。
//!
//! 光靠执行态（`SignalState::execution_stage`）不够：它只覆盖「主轮次」，
//! 而工具页手动调用、录制会话、插件独立运行时都不经过主轮次（B3/B4 路径）。
//!
//! ## 语义（大王铁律：无并行机制、禁止并行、系统操作更禁止并行）
//!
//! - **单槽互斥**：任一时刻至多一个持有者。`ResourceClass` 只用于诊断与文案，
//!   互斥语义对所有类别一致（粒度可分类、语义必须一致）。
//! - **非阻塞**：拿不到立即返回 [`LeaseBusy`]（稳定码 [`CODE_BUSY`]），
//!   **不排队、不等待、不降级** —— 禁止用重试 / sleep 掩盖竞态。
//! - **RAII**：租约 drop 即释放（含 panic 展开路径），异常路径不泄漏。
//! - **同所有者可重入**：`owner` 相同的再次获取直接通过（录制会话里的
//!   `rec_browser_*` 子操作就是同一所有者的内部步骤）；不同 owner 一律拒绝。
//! - **与执行态协同**：Agent 轮次（Running / Finalizing）整轮持锁，期间手动工具
//!   请求被拒；反过来录制会话 / 定时任务等执行体持锁时新轮次也被拒。
//!   **本门不拦截「追加消息（终止 / 停止）」通道** —— 那条通道必须永远可用。
//!
//! ## 实例与注入
//!
//! 门是**可实例化**的（`AutomationGate::new()`），生产环境由 `AppState` 持有唯一
//! 实例并注入各入口（与 `SignalState` 同一约定），单测各自建实例互不干扰。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// 稳定错误码：前端按此映射文案（`frontend/src/main-window/lib/api.ts`）。
pub const CODE_BUSY: &str = "automation_busy";

/// 资源类别 —— 仅用于诊断与文案；互斥语义对所有类别一致（单槽互斥）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceClass {
    /// 桌面自动化：鼠标 / 键盘 / 窗口 / 截图 / 找图找色 / OCR
    Desktop,
    /// 浏览器控制：进程级 CDP 单例（chromiumoxide）
    Browser,
    /// 屏幕录制：低层 hook 捕获真实桌面事件（录制会话）
    Recording,
    /// 主执行体：Agent 轮次 / 工作流运行 / 插件独立运行时 —— 期间上述资源全部不可用
    ExecutionBody,
}

impl ResourceClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Browser => "browser",
            Self::Recording => "recording",
            Self::ExecutionBody => "execution_body",
        }
    }

    /// 面向用户的中文释义（稳定码之外的人话）
    pub fn label(&self) -> &'static str {
        match self {
            Self::Desktop => "桌面自动化",
            Self::Browser => "浏览器控制",
            Self::Recording => "屏幕录制",
            Self::ExecutionBody => "任务执行",
        }
    }
}

/// 持有者身份 —— 同样只用于诊断与文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldKind {
    /// 执行体整体占用（Agent 轮次 / 工作流 / 插件运行时 / 定时任务）
    ExecutionBody,
    /// 手动工具单次调用（工具页 / 独立桌面命令）
    ManualTool,
    /// 录制会话及其子操作
    Recording,
}

impl HoldKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ExecutionBody => "execution_body",
            Self::ManualTool => "manual_tool",
            Self::Recording => "recording",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::ExecutionBody => "任务执行",
            Self::ManualTool => "手动工具操作",
            Self::Recording => "录制会话",
        }
    }
}

/// 获取失败 —— 明确拒绝（非排队、非降级）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseBusy {
    /// 请求方要什么类别
    pub class: ResourceClass,
    /// 请求方是什么身份
    pub kind: HoldKind,
    /// 当前持有者类别
    pub holder_class: ResourceClass,
    /// 当前持有者身份
    pub holder_kind: HoldKind,
    /// 当前持有者的可读标签（如 "agent round" / "workflow:xxx"）
    pub holder_label: String,
    /// 已持有时长（毫秒）
    pub held_ms: u64,
}

impl LeaseBusy {
    /// 稳定错误码（前端映射文案的依据）
    pub fn code(&self) -> &'static str {
        CODE_BUSY
    }

    /// 面向用户的一句话（不带码前缀）
    pub fn message(&self) -> String {
        format!(
            "{}正在被占用，请稍后再试（当前：{}）",
            self.holder_class.label(),
            self.holder_kind.label()
        )
    }
}

impl std::fmt::Display for LeaseBusy {
    /// 输出为 `automation_busy: <人话>` —— 稳定码前缀 + 可直接展示的文案。
    /// Tauri 的 `Err(String)` 只有字符串通道，码前缀是前端映射的唯一锚点。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", CODE_BUSY, self.message())
    }
}

/// 当前持有者快照（诊断用）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderSnapshot {
    pub class: ResourceClass,
    pub kind: HoldKind,
    pub label: String,
    pub owner: String,
    pub held_ms: u64,
}

struct Holder {
    class: ResourceClass,
    kind: HoldKind,
    label: String,
    owner: String,
    since: Instant,
    token: u64,
}

/// 资源互斥门（单槽）。生产环境唯一实例由 `AppState` 持有。
pub struct AutomationGate {
    holder: Mutex<Option<Holder>>,
    next_token: AtomicU64,
}

impl Default for AutomationGate {
    fn default() -> Self {
        Self::new()
    }
}

impl AutomationGate {
    pub fn new() -> Self {
        Self {
            holder: Mutex::new(None),
            next_token: AtomicU64::new(1),
        }
    }

    /// 互斥由本结构的 `Mutex` 提供：判定与占位在同一临界区内完成；
    /// 临界区内**不做任何 await / 跨锁调用**（与 `SignalState` 同一约定）。
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Holder>> {
        self.holder.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 非阻塞获取租约（`self: &Arc<Self>` —— 租约需持有门句柄以便 drop 释放）。
    ///
    /// - 空闲 → 占用并返回 owned 租约（drop 释放）
    /// - 同 `owner` 再次获取 → 返回**重入租约**（不占槽位，drop 无副作用）
    /// - 其他持有者 → `Err(LeaseBusy)`；调用方必须把拒绝原样返回给用户
    pub fn try_acquire(
        self: &Arc<Self>,
        class: ResourceClass,
        kind: HoldKind,
        owner: impl Into<String>,
    ) -> Result<AutomationLease, LeaseBusy> {
        let owner = owner.into();
        let mut slot = self.lock();
        if let Some(current) = slot.as_ref() {
            if current.owner == owner {
                // 同一所有者的内部子操作（如录制会话内的 rec_browser_*）
                return Ok(AutomationLease {
                    gate: None,
                    token: 0,
                    class,
                    kind,
                });
            }
            let busy = LeaseBusy {
                class,
                kind,
                holder_class: current.class,
                holder_kind: current.kind,
                holder_label: current.label.clone(),
                held_ms: current.since.elapsed().as_millis() as u64,
            };
            tracing::debug!(
                "[automation-gate] 拒绝 {} ({}) ← {} ({}) 仍持有 slot {}ms",
                class.as_str(),
                kind.as_str(),
                current.class.as_str(),
                current.owner,
                busy.held_ms
            );
            return Err(busy);
        }
        let token = self.next_token.fetch_add(1, Ordering::SeqCst);
        *slot = Some(Holder {
            class,
            kind,
            label: format!("{} ({})", class.label(), kind.label()),
            owner,
            since: Instant::now(),
            token,
        });
        drop(slot);
        Ok(AutomationLease {
            gate: Some(Arc::clone(self)),
            token,
            class,
            kind,
        })
    }

    /// 当前持有者快照（None = 空闲）
    pub fn holder(&self) -> Option<HolderSnapshot> {
        self.lock().as_ref().map(|h| HolderSnapshot {
            class: h.class,
            kind: h.kind,
            label: h.label.clone(),
            owner: h.owner.clone(),
            held_ms: h.since.elapsed().as_millis() as u64,
        })
    }

    /// 是否空闲
    pub fn is_free(&self) -> bool {
        self.lock().is_none()
    }

    /// 释放指定 token 的租约（仅 [`AutomationLease::drop`] 调用）。
    /// token 不匹配 = 迟到的释放（槽位已被新持有者占用）→ 不动槽位，只告警。
    fn release(&self, token: u64) {
        let mut slot = self.lock();
        match slot.as_ref() {
            Some(h) if h.token == token => {
                let held_ms = h.since.elapsed().as_millis() as u64;
                let holder = format!("{} (owner={})", h.class.as_str(), h.owner);
                *slot = None;
                tracing::debug!("[automation-gate] 释放 slot: {holder} 持有 {held_ms}ms");
            }
            Some(h) => tracing::warn!(
                "[automation-gate] 忽略迟到的释放：token {} != 当前 {}",
                token,
                h.token
            ),
            None => tracing::warn!("[automation-gate] 忽略释放：slot 已空（token {token}）"),
        }
    }
}

/// RAII 租约：drop 释放。`is_reentrant()` 为 true 时**不持有槽位**（同所有者重入）。
pub struct AutomationLease {
    gate: Option<Arc<AutomationGate>>,
    token: u64,
    class: ResourceClass,
    kind: HoldKind,
}

impl AutomationLease {
    pub fn class(&self) -> ResourceClass {
        self.class
    }

    pub fn kind(&self) -> HoldKind {
        self.kind
    }

    /// 是否为同所有者重入租约（未新占槽位，drop 不会释放别人的 slot）
    pub fn is_reentrant(&self) -> bool {
        self.token == 0
    }
}

impl Drop for AutomationLease {
    fn drop(&mut self) {
        if self.token == 0 {
            return; // 重入租约不占有槽位
        }
        if let Some(gate) = self.gate.take() {
            gate.release(self.token);
        }
    }
}

impl std::fmt::Debug for AutomationLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutomationLease")
            .field("class", &self.class.as_str())
            .field("kind", &self.kind.as_str())
            .field("reentrant", &self.is_reentrant())
            .finish()
    }
}

/// 「执行体」owner 键：每次轮次唯一（两个轮次绝不共用 owner，故绝不互相重入）。
pub fn execution_body_owner(label: &str) -> String {
    format!("execution-body:{label}:{}", uuid::Uuid::new_v4())
}

/// 工具 → 资源类别（`None` = 不触碰系统资源，无需取锁）。
///
/// 判定与工具命名前缀**同源**（`ToolRegistry::is_browser_tool` / `is_desktop_tool`），
/// 不允许各入口自行手写前缀判断：那正是「门放行的集合」与「registry 执行的集合」
/// 出现漂移的根因。
pub fn tool_resource_class(tool_name: &str) -> Option<ResourceClass> {
    if crate::tools::ToolRegistry::is_browser_tool(tool_name) {
        Some(ResourceClass::Browser)
    } else if crate::tools::ToolRegistry::is_desktop_tool(tool_name) {
        Some(ResourceClass::Desktop)
    } else {
        None
    }
}

/// 录制会话固定 owner 键：同一会话内的子操作（`rec_browser_*`）借此重入。
pub const OWNER_RECORDING: &str = "recording-session";

/// 手动工具操作 owner 键：工具页每次调用独立（同一次并发双击也各自成 owner → 互斥）。
pub const OWNER_MANUAL_TOOL: &str = "manual-tool";

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// ① 资源互斥：两个入口同时请求同一资源 → 恰有一方成功，另一方拿到稳定拒绝码。
    ///
    /// 并发而非串行地抢（`tokio::spawn` + `Barrier` 保证同一起跑线），
    /// 循环多轮以覆盖 dispatch 交错。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_entries_race_exactly_one_wins() {
        for _ in 0..50 {
            let gate = Arc::new(AutomationGate::new());
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let mut handles = Vec::new();
            for idx in 0..2 {
                let gate = gate.clone();
                let barrier = barrier.clone();
                handles.push(tokio::spawn(async move {
                    barrier.wait().await;
                    let owner = format!("manual-{idx}");
                    gate.try_acquire(ResourceClass::Desktop, HoldKind::ManualTool, owner)
                }));
            }
            let mut wins = 0;
            let mut losses = 0;
            for h in handles {
                // 租约必须活着到判定之后，否则会提前释放让另一方也成功
                match h.await.expect("join") {
                    Ok(lease) => {
                        assert!(!lease.is_reentrant(), "不同 owner 不可能重入");
                        wins += 1;
                        drop(lease);
                    }
                    Err(e) => {
                        assert_eq!(e.code(), CODE_BUSY, "拒绝必须带稳定码");
                        assert_eq!(e.holder_class, ResourceClass::Desktop);
                        assert!(
                            e.to_string().starts_with(CODE_BUSY),
                            "Display 必须以稳定码开头: {e}"
                        );
                        losses += 1;
                    }
                }
            }
            assert_eq!((wins, losses), (1, 1), "恰有一方成功");
            assert!(gate.is_free(), "持有者释放后门必须回到空闲");
        }
    }

    /// ② 执行期手动工具被拒：执行体持锁期间，手动操作拿到明确拒绝（不排队、不降级）。
    #[test]
    fn manual_tool_rejected_while_execution_body_holds() {
        let gate = Arc::new(AutomationGate::new());
        let body = gate
            .try_acquire(
                ResourceClass::ExecutionBody,
                HoldKind::ExecutionBody,
                execution_body_owner("agent-round"),
            )
            .expect("空闲时应获取成功");

        let denied = gate.try_acquire(
            ResourceClass::Desktop,
            HoldKind::ManualTool,
            OWNER_MANUAL_TOOL,
        );
        let err = denied.err().expect("执行体持锁期间手动工具必须被拒");
        assert_eq!(err.code(), CODE_BUSY);
        assert_eq!(err.kind, HoldKind::ManualTool);
        assert_eq!(err.holder_kind, HoldKind::ExecutionBody);
        assert!(
            err.message().contains("正在被占用"),
            "拒绝文案必须可直接展示: {}",
            err.message()
        );

        // 浏览器类别同样被拒（语义一致：粒度可分类，互斥语义一致）
        assert!(gate
            .try_acquire(
                ResourceClass::Browser,
                HoldKind::ManualTool,
                OWNER_MANUAL_TOOL
            )
            .is_err());

        drop(body);
        // 执行体退出 → 手动工具立刻可用（无需重试/等待）
        assert!(gate
            .try_acquire(
                ResourceClass::Desktop,
                HoldKind::ManualTool,
                OWNER_MANUAL_TOOL
            )
            .is_ok());
    }

    /// ③ 重入语义：同一 owner（录制会话）在持锁期间可重复获取；不同 owner 一律拒绝。
    #[test]
    fn same_owner_reenters_other_owner_rejected() {
        let gate = Arc::new(AutomationGate::new());
        let session = gate
            .try_acquire(
                ResourceClass::Recording,
                HoldKind::Recording,
                OWNER_RECORDING,
            )
            .expect("空闲时应获取成功");

        // 录制会话内的子操作（rec_browser_* 触碰浏览器单例）：同 owner → 重入
        let sub = gate
            .try_acquire(ResourceClass::Browser, HoldKind::Recording, OWNER_RECORDING)
            .expect("同一所有者应可重入");
        assert!(sub.is_reentrant(), "重入租约不占槽位");

        // 手动工具 / 轮次是不同 owner → 拒绝
        assert!(
            gate.try_acquire(
                ResourceClass::Browser,
                HoldKind::ManualTool,
                OWNER_MANUAL_TOOL
            )
            .is_err(),
            "录制中手动操作浏览器必须被拒"
        );
        assert!(
            gate.try_acquire(
                ResourceClass::ExecutionBody,
                HoldKind::ExecutionBody,
                execution_body_owner("agent-round")
            )
            .is_err(),
            "录制中不得开出新轮次（旧执行体仍在跑 = 双跑）"
        );

        drop(sub); // 重入租约 drop 不得释放会话槽位
        assert!(!gate.is_free(), "重入租约 drop 不能释放持有者的槽位");
        drop(session);
        assert!(gate.is_free(), "会话结束（含子操作）后释放");
    }

    /// ④ 与执行态协同 + append 通道不被门拦截。
    ///
    /// 门**只在资源入口判定**，不触碰 `SignalState`：持锁期间
    /// 「追加消息（终止 / 停止）」通道的读写（队列 / 阶段 / 暂停决策）必须照常可用。
    #[test]
    fn append_channel_stays_usable_while_slot_held() {
        use crate::state::{ExecutionStage, SignalState};

        let gate = Arc::new(AutomationGate::new());
        let signals = crate::state::new_shared_signals();
        let _body = gate
            .try_acquire(
                ResourceClass::ExecutionBody,
                HoldKind::ExecutionBody,
                execution_body_owner("agent-round"),
            )
            .expect("空闲时应获取成功");

        // 轮次在跑（Running）：追加通道 = 队列写入 + 阶段读取
        SignalState::set_execution_stage(&signals, ExecutionStage::Running);
        assert!(
            crate::mobile_append::enqueue(&signals, "追加：请停止".to_string()),
            "持锁期间追加必须入队（终止/停止走同一通道）"
        );
        assert_eq!(
            SignalState::read(&signals).append_queue,
            vec!["追加：请停止".to_string()]
        );
        assert_eq!(
            SignalState::execution_stage(&signals),
            ExecutionStage::Running
        );

        // 终止决策同样写入共享信号（不经门）
        crate::agent::pause::set_pause_action_id(&signals, "act-1");
        crate::agent::pause::set_pause_decision(
            &signals,
            "act-1",
            crate::agent::pause::PauseDecision::Terminate,
        );
        assert_eq!(
            crate::agent::pause::check_pause_decision(&signals, "act-1"),
            Some(crate::agent::pause::PauseDecision::Terminate),
            "终止决策必须可被主循环读到（消费一次）"
        );

        // 收尾期：追加被拒（既有语义），但门与阶段判定互不干扰
        SignalState::set_execution_stage(&signals, ExecutionStage::Finalizing);
        assert!(!SignalState::execution_stage(&signals).accepts_append());
    }

    /// 手动入口的资源分类与工具命名前缀同源（registry 是唯一判定者），
    /// 非自动化工具不应被门拦截（否则工具页的手动文件/记忆类操作会被无谓拒绝）。
    #[test]
    fn tool_resource_class_follows_registry_prefixes() {
        assert_eq!(
            tool_resource_class("desktop_screenshot"),
            Some(ResourceClass::Desktop)
        );
        assert_eq!(
            tool_resource_class("desktop_input"),
            Some(ResourceClass::Desktop)
        );
        assert_eq!(
            tool_resource_class("browser_navigate"),
            Some(ResourceClass::Browser)
        );
        assert_eq!(
            tool_resource_class("browser_import_cookies"),
            Some(ResourceClass::Browser)
        );
        assert_eq!(tool_resource_class("Read"), None);
        assert_eq!(tool_resource_class("system_shell"), None);
        assert_eq!(tool_resource_class("web_search"), None);
    }

    /// ⑤ 诊断快照：持有者信息可用于排查「谁把我挡住了」。
    #[test]
    fn holder_snapshot_reports_current_owner() {
        let gate = Arc::new(AutomationGate::new());
        assert!(gate.holder().is_none());
        let _lease = gate
            .try_acquire(ResourceClass::Browser, HoldKind::ManualTool, "tool-page")
            .expect("空闲时应获取成功");
        let snap = gate.holder().expect("持有时应有快照");
        assert_eq!(snap.class, ResourceClass::Browser);
        assert_eq!(snap.kind, HoldKind::ManualTool);
        assert_eq!(snap.owner, "tool-page");
    }

    /// ⑥ 非阻塞契约：拒绝必须**立即**返回（不得 sleep / 排队）。
    #[tokio::test]
    async fn rejection_is_immediate_not_queued() {
        let gate = Arc::new(AutomationGate::new());
        let _body = gate
            .try_acquire(
                ResourceClass::ExecutionBody,
                HoldKind::ExecutionBody,
                execution_body_owner("agent-round"),
            )
            .expect("空闲时应获取成功");
        let started = Instant::now();
        let err = gate
            .try_acquire(
                ResourceClass::Desktop,
                HoldKind::ManualTool,
                OWNER_MANUAL_TOOL,
            )
            .err()
            .expect("必须被拒");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(50),
            "拒绝必须是即时的（实测 {:?}），不得等待/重试",
            started.elapsed()
        );
        assert_eq!(err.code(), CODE_BUSY);
    }
}
