//! resource_gate — 资源互斥门的 **Tauri 层适配**（获取入口统一收口）
//!
//! 门本体在 core：`nuphus::automation_gate`（可实例化、语义见其模块文档）。
//! 本模块只做三件事：
//! 1. 定义「哪些工具/命令触碰系统资源」的分类（分类判定唯一来源，避免各处各写一套前缀判断）；
//! 2. 为「执行体」与「录制会话」提供统一的获取入口（owner 键与文案只在这里定义）；
//! 3. 提供把 [`nuphus::automation_gate::LeaseBusy`] 转成前端可映射错误串的收口。
//!
//! **纪律**：任何新的「会操作桌面自动化 / 浏览器控制 / 录制」或「占用主执行体」的
//! 生产入口，都在这里登记分类或复用获取入口，不允许在业务代码里手写前缀判断。

use nuphus::automation_gate::{
    execution_body_owner, AutomationGate, AutomationLease, HoldKind, ResourceClass, OWNER_RECORDING,
};
use std::sync::Arc;

/// 工具是否是「系统自动化」工具（桌面 / 浏览器）—— 手动入口需要拿资源锁的判据。
///
/// 判定委托 core 的 [`nuphus::automation_gate::tool_resource_class`]（其内部与
/// `ToolRegistry::is_*_tool` 同源）：这里**不重复写前缀**，避免门与 registry 漂移。
pub fn tool_touches_automation(tool_name: &str) -> bool {
    nuphus::automation_gate::tool_resource_class(tool_name).is_some()
}

/// Compatibility helper for callers and tests that only need the lease.
pub fn acquire_execution_body(
    gate: &Arc<AutomationGate>,
    label: &str,
) -> Result<AutomationLease, String> {
    acquire_execution_body_with_owner(gate, label).map(|(lease, _)| lease)
}

/// 工具 → 资源类别（仅用于诊断文案；互斥语义单槽一致）。
pub fn class_of_tool(tool_name: &str) -> ResourceClass {
    nuphus::automation_gate::tool_resource_class(tool_name).unwrap_or(ResourceClass::Desktop)
}

/// 统一获取「执行体」租约 —— Agent 轮次 / 工作流 / 插件运行时 / 定时任务共用。
///
/// `label` 只进诊断日志（如 `"submit_user_message"`）；owner 键每次独立，
/// 因此两个执行体**绝不**互相重入（这正是防双跑的关键）。
pub fn acquire_execution_body_with_owner(
    gate: &Arc<AutomationGate>,
    label: &str,
) -> Result<(AutomationLease, String), String> {
    let owner = execution_body_owner(label);
    gate.try_acquire(
        ResourceClass::ExecutionBody,
        HoldKind::ExecutionBody,
        owner.clone(),
    )
    .map(|lease| (lease, owner))
    .map_err(|busy| {
        tracing::warn!("[resource-gate] 拒绝执行体 {label}：{busy}");
        busy.to_string()
    })
}

/// 统一获取「录制会话」租约 —— 会话期间一直持有，直到 `RecSession` 被清空/替换。
pub fn acquire_recording_session(gate: &Arc<AutomationGate>) -> Result<AutomationLease, String> {
    gate.try_acquire(
        ResourceClass::Recording,
        HoldKind::Recording,
        OWNER_RECORDING,
    )
    .map_err(|busy| {
        tracing::warn!("[resource-gate] 拒绝开启录制会话：{busy}");
        busy.to_string()
    })
}

/// 录制会话的**内部子操作**（`rec_browser_*` 触碰浏览器单例等）。
///
/// 同一 owner 键 → 重入通过：录制会话已独占资源，其子操作天然串行，
/// 不会与其它执行体并行（这正是允许重入的前提）。
pub fn acquire_recording_sub_op(
    gate: &Arc<AutomationGate>,
    class: ResourceClass,
    label: &str,
) -> Result<AutomationLease, String> {
    gate.try_acquire(class, HoldKind::Recording, OWNER_RECORDING)
        .map_err(|busy| {
            tracing::warn!("[resource-gate] 拒绝录制子操作 {label}：{busy}");
            busy.to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_tool_classification() {
        assert!(tool_touches_automation("desktop_screenshot"));
        assert!(tool_touches_automation("browser_navigate"));
        assert!(!tool_touches_automation("Read"));
        assert!(!tool_touches_automation("system_shell"));
        assert_eq!(class_of_tool("browser_click"), ResourceClass::Browser);
        assert_eq!(class_of_tool("desktop_input"), ResourceClass::Desktop);
    }

    /// 执行体持锁期间：手动工具（工具页路径）被拒；录制子操作（同 owner）可重入。
    #[test]
    fn execution_body_blocks_manual_tool_but_not_its_own_sub_ops() {
        let gate = Arc::new(AutomationGate::new());
        let body = acquire_execution_body(&gate, "submit_user_message").expect("空闲应获取成功");

        let denied = gate.try_acquire(
            class_of_tool("desktop_screenshot"),
            HoldKind::ManualTool,
            nuphus::automation_gate::OWNER_MANUAL_TOOL,
        );
        assert_eq!(
            denied.err().map(|e| e.code()),
            Some(nuphus::automation_gate::CODE_BUSY)
        );

        let recording = acquire_recording_session(&gate);
        assert!(recording.is_err(), "执行体在跑时不得开启录制会话");

        drop(body);
        let session = acquire_recording_session(&gate).expect("执行体退出后录制可开始");
        let sub = acquire_recording_sub_op(&gate, ResourceClass::Browser, "rec_browser_poll")
            .expect("录制子操作应可重入");
        assert!(sub.is_reentrant());
        assert!(
            acquire_execution_body(&gate, "submit_user_message").is_err(),
            "录制会话进行中不得开出新轮次"
        );
        drop(sub);
        drop(session);
        assert!(gate.is_free());
    }
}
