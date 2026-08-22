// desktop-api v2.0 - Nuphus 核心基础设施
// 感知 · 查找 · 操作 · 沟通

#[cfg(feature = "http-server")]
pub mod api;
pub mod clipboard;
pub mod core;
pub mod input;
// platform（WindowManager 等）为 Windows 专属：hwnd/Target::Window 仅在 cfg(windows) 下存在。
// Linux/macOS 编译时该模块整体不编译，避免 Target::Window 引用错误。
#[cfg(windows)]
pub mod platform;
pub mod utils;
pub mod vision;

pub use core::*;
pub use input::*;
#[cfg(windows)]
pub use platform::*;
pub use vision::*;

#[cfg(feature = "http-server")]
pub use http_server_entry::DesktopApi;

#[cfg(feature = "http-server")]
mod http_server_entry {
    use crate::core::{Result, SessionHandle, SessionManager, TargetSpec};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use uuid::Uuid;

    /// 桌面 API 主入口
    pub struct DesktopApi {
        sessions: Arc<RwLock<SessionManager>>,
    }

    impl DesktopApi {
        pub fn new() -> Self {
            Self {
                sessions: Arc::new(RwLock::new(SessionManager::new())),
            }
        }

        /// 创建会话 - 自动识别目标类型
        pub async fn create_session(&self, target: TargetSpec) -> Result<SessionHandle> {
            let mut manager = self.sessions.write().await;
            manager.create(target).await
        }

        /// 获取会话
        pub async fn get_session(&self, id: Uuid) -> Option<SessionHandle> {
            let manager = self.sessions.read().await;
            manager.get(id)
        }

        /// 关闭会话 - 自动清理资源
        pub async fn close_session(&self, id: Uuid) -> Result<()> {
            let mut manager = self.sessions.write().await;
            manager.close(id).await
        }
    }

    impl Default for DesktopApi {
        fn default() -> Self {
            Self::new()
        }
    }
}
