//! 跨进程自动化锁（protocol v2，与 nuphus-mcp coordination v2 同构）。
//!
//! 主仓库直连通道与各 nuphus-mcp 实例通过同一组 OS 文件锁互斥：
//! - `browser_*` 取 machine 共享锁（后台浏览器操作可并行）；
//! - `desktop_*` 等取 machine 排他锁（独占桌面）。
//!
//! 锁文件位于 `{data_dir}/Nuphus/nuphus-mcp/locks-v2/`，永不 unlink；
//! OS 锁在进程退出/崩溃时立即释放，无 TTL、无心跳。
//!
//! v1（`automation.lock` JSON lease + TTL）已废弃：v1 与 v2 互不相识，
//! 升级时必须停掉所有旧实例，不能混用。

use crate::utils::coordination::{Coordination, ResourceGuard};

/// 跨进程自动化锁。
pub struct AutomationLock {
    coordination: Coordination,
}

/// 锁守卫：drop 即释放（OS 锁 + 进程内引用计数）。
pub type LockGuard = ResourceGuard;

impl AutomationLock {
    pub fn new() -> Self {
        Self {
            coordination: Coordination::default(),
        }
    }

    /// 获取本工具的 machine 锁。`browser_*` 共享，其余排他。
    pub fn acquire(&self, tool: &str) -> Result<LockGuard, String> {
        self.coordination.machine(!tool.starts_with("browser_"))
    }
}

impl Default for AutomationLock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_browser_calls_share_but_desktop_is_exclusive() {
        let lock = AutomationLock {
            coordination: Coordination::at(std::env::temp_dir().join(format!(
                "nuphus-machine-gate-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ))),
        };
        let a = lock.acquire("browser_wait_for").unwrap();
        let b = lock.acquire("browser_snapshot").unwrap();
        assert!(lock.acquire("desktop_mouse").is_err());
        drop((a, b));
        let desktop = lock.acquire("desktop_mouse").unwrap();
        assert!(lock.acquire("browser_snapshot").is_err());
        drop(desktop);
        assert!(lock.acquire("browser_snapshot").is_ok());
    }
}
