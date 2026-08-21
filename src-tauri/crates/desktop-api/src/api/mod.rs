//! 统一 API 层 - see / find / do / say + 降级策略
//!
//! Only compiled when `http-server` feature is enabled.

#[cfg(feature = "http-server")]
pub mod http;

#[cfg(feature = "http-server")]
mod api_impl {
    use crate::core::{Point, Result, Scope, SessionHandle, Target};
    use crate::input::InputEngine;
    use crate::platform::WindowManager;
    use crate::vision::{FindResult, PerceiveWhat, Perception, Query, VisionEngine};

    /// 统一 API 入口
    pub struct UnifiedApi {
        vision: VisionEngine,
        input: InputEngine,
        window_mgr: WindowManager,
    }

    impl UnifiedApi {
        pub fn new() -> Self {
            let cleanup = crate::utils::cleanup::CleanupQueue::new();
            Self {
                vision: VisionEngine::new(cleanup.clone()),
                input: InputEngine::new(),
                window_mgr: WindowManager::new(),
            }
        }

        // ─────────────────────────────── see ───────────────────────────────

        /// 感知 - 截图 + 分析
        pub async fn see(
            &self,
            session: &SessionHandle,
            scope: Scope,
            what: PerceiveWhat,
        ) -> Result<Perception> {
            let target = session.target.read().await;
            self.vision.see(&*target, scope, what).await
        }

        // ─────────────────────────────── find ───────────────────────────────

        /// 查找 - 找图/找字/找色
        pub async fn find(&self, session: &SessionHandle, query: &Query) -> Result<FindResult> {
            // 先截图
            let target = session.target.read().await;
            let frame = self.vision.capture(&*target, Scope::Window).await?;

            // 查找
            self.vision.find(&frame, query).await
        }

        // ─────────────────────────────── do ───────────────────────────────

        /// 操作 - 点击/拖拽/按键
        pub async fn do_(&self, session: &SessionHandle, action: Action) -> Result<DoResult> {
            let mut target = session.target.write().await;

            match action {
                Action::Click { x, y } => {
                    self.input.click(&mut *target, Point { x, y }).await?;
                    Ok(DoResult::Success)
                }
                Action::DoubleClick { x, y } => {
                    self.input.click(&mut *target, Point { x, y }).await?;
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    self.input.click(&mut *target, Point { x, y }).await?;
                    Ok(DoResult::Success)
                }
                Action::Drag { start, end } => {
                    self.input.drag(&mut *target, start, end).await?;
                    Ok(DoResult::Success)
                }
                Action::Press { key } => {
                    self.input.press(&mut *target, &key).await?;
                    Ok(DoResult::Success)
                }
                Action::Hotkey { keys } => {
                    let keys_ref: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
                    self.input.hotkey(&mut *target, &keys_ref).await?;
                    Ok(DoResult::Success)
                }
            }
        }

        // ─────────────────────────────── say ───────────────────────────────

        /// 沟通 - 发送消息 + 验证送达
        pub async fn say(&self, session: &SessionHandle, text: &str) -> Result<SayResult> {
            let mut target = session.target.write().await;

            // 策略1: 直接发送
            match self.input.send_text(text, &mut *target).await {
                Ok(()) => {
                    // 验证: 截图输入区域确认文字出现
                    // TODO: 验证逻辑
                    return Ok(SayResult::Delivered);
                }
                Err(e) => {
                    tracing::warn!("直接发送失败: {}, 启动降级", e);
                }
            }

            // 降级1: 剪贴板 + 粘贴
            // TODO: 剪贴板方案

            // 降级2: 逐字符发送
            // TODO

            // 降级3: 截图分析失败原因，报告人工
            Ok(SayResult::Failed("all strategies exhausted".to_string()))
        }
    }

    // ─────────────────────────────── 类型 ───────────────────────────────

    /// 操作类型
    #[derive(Debug, Clone)]
    pub enum Action {
        Click { x: i32, y: i32 },
        DoubleClick { x: i32, y: i32 },
        Drag { start: Point, end: Point },
        Press { key: String },
        Hotkey { keys: Vec<String> },
    }

    /// 操作结果
    #[derive(Debug, Clone)]
    pub enum DoResult {
        Success,
        Verified { before: String, after: String },
        Failed(String),
    }

    /// 沟通结果
    #[derive(Debug, Clone)]
    pub enum SayResult {
        Delivered,
        Partial { sent: String, failed: String },
        Failed(String),
    }
}

#[cfg(feature = "http-server")]
pub use api_impl::*;
