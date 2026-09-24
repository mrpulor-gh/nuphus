//! 输入控制模块 - SendInput Unicode + 鼠标 + 键盘

use crate::core::*;

pub mod keyboard;
pub mod mouse;
#[cfg(windows)]
pub mod sendinput;

pub use keyboard::*;
pub use mouse::*;
#[cfg(windows)]
pub use sendinput::*;

/// Release only inputs pressed by this operation, including on cancelled
/// futures and early native errors. Reverse order keeps modifiers balanced.
#[cfg(any(windows, target_os = "linux", test))]
pub(crate) struct HeldInput<T, F: FnMut(T)> {
    held: Vec<T>,
    release: F,
}

#[cfg(any(windows, target_os = "linux", test))]
impl<T, F: FnMut(T)> HeldInput<T, F> {
    pub(crate) fn new(release: F) -> Self {
        Self {
            held: Vec::new(),
            release,
        }
    }

    pub(crate) fn hold(&mut self, input: T) {
        self.held.push(input);
    }

    pub(crate) fn release_last(&mut self) -> bool {
        if let Some(input) = self.held.pop() {
            (self.release)(input);
            true
        } else {
            false
        }
    }
}

#[cfg(any(windows, target_os = "linux", test))]
impl<T, F: FnMut(T)> Drop for HeldInput<T, F> {
    fn drop(&mut self) {
        while self.release_last() {}
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::HeldInput;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn cancelling_input_releases_only_held_inputs_in_reverse_order() {
        let released = Arc::new(Mutex::new(Vec::new()));
        let recorded = released.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut guard = HeldInput::new(move |key| recorded.lock().unwrap().push(key));
            guard.hold("ctrl");
            guard.hold("s");
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(*released.lock().unwrap(), ["s", "ctrl"]);
    }

    #[test]
    fn explicitly_released_input_is_not_released_twice() {
        let mut released = Vec::new();
        {
            let mut guard = HeldInput::new(|key| released.push(key));
            guard.hold("left-button");
            assert!(guard.release_last());
        }
        assert_eq!(released, ["left-button"]);
    }
}

/// Serialize native input, retrying initialization after a missing permission/display is fixed.
#[cfg(not(windows))]
pub fn with_enigo<T>(operation: impl FnOnce(&mut enigo::Enigo) -> Result<T>) -> Result<T> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> u8;
        }
        // CGEventPost can silently discard events after permission is revoked.
        // Check even when an input engine was initialized earlier.
        if unsafe { AXIsProcessTrusted() } == 0 {
            return Err(DesktopError::InputFailed(
                "请在系统设置→隐私与安全性→辅助功能中授权 Nuphus 后重试".into(),
            ));
        }
    }
    static INSTANCE: std::sync::Mutex<Option<SendEnigo>> = std::sync::Mutex::new(None);
    let mut instance = INSTANCE
        .lock()
        .map_err(|e| DesktopError::InputFailed(format!("input lock: {e}")))?;
    if instance.is_none() {
        let engine = enigo::Enigo::new(&enigo::Settings::default())
            .map_err(|e| DesktopError::InputFailed(format!("input initialization failed: {e}")))?;
        *instance = Some(SendEnigo(engine));
    }
    operation(
        &mut instance
            .as_mut()
            .ok_or_else(|| DesktopError::InputFailed("input initialization unavailable".into()))?
            .0,
    )
}

/// enigo 0.2 在 macOS 上 `Enigo` 内部持有 `NonNull<CGEventSource>`（core-graphics 指针），
/// 编译器推断为非 Send/Sync，导致 `OnceLock<Mutex<Enigo>>` 静态变量无法编译
/// （`shared static variables must have a type that implements Sync`）。
///
/// 安全性论证：
/// - CoreGraphics 的 CGEventSource / CGEventPost 均为线程安全 API（Apple 官方文档确认）；
/// - 所有访问经全局 `Mutex` 串行化，任意时刻仅一个线程持有可变引用；
/// - Linux 上 `Enigo` 本就 Send/Sync，包装后行为不变。
///
/// 实现 `Deref`/`DerefMut` 自动解引用：调用点 `enigo().lock()?.key(...)` 无需改动。
#[cfg(not(windows))]
pub struct SendEnigo(pub enigo::Enigo);

#[cfg(not(windows))]
unsafe impl Send for SendEnigo {}
#[cfg(not(windows))]
unsafe impl Sync for SendEnigo {}

#[cfg(not(windows))]
impl std::ops::Deref for SendEnigo {
    type Target = enigo::Enigo;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(not(windows))]
impl std::ops::DerefMut for SendEnigo {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// 输入引擎
pub struct InputEngine;

impl Default for InputEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl InputEngine {
    pub fn new() -> Self {
        Self
    }

    /// 发送文本 - 自动选择最优策略
    #[cfg_attr(not(windows), allow(unused_variables))]
    pub async fn send_text(&self, text: &str, target: &mut Target) -> Result<()> {
        // 确保目标激活
        self.ensure_active(target).await?;

        match target {
            #[cfg(windows)]
            Target::Window { .. } => {
                // Windows: SendInput Unicode
                sendinput::send_unicode_text(text)?;
            }
            Target::Tui { .. } => {
                // 终端：Windows 走 SendInput；其他平台不支持
                #[cfg(windows)]
                {
                    sendinput::send_unicode_text(text)?;
                }
                #[cfg(not(windows))]
                {
                    return Err(DesktopError::PlatformNotSupported);
                }
            }
            Target::Browser { .. } => {
                // 浏览器: Playwright 输入
                // TODO: 调用 browser 模块
            }
        }

        Ok(())
    }

    /// 点击 - 自动激活 + 点击
    pub async fn click(&self, target: &mut Target, point: Point) -> Result<()> {
        self.ensure_active(target).await?;
        mouse::click(point.x, point.y).await
    }

    /// 激活目标窗口到前台（幂等：已验证则跳过）。
    ///
    /// 独立暴露的窗口激活入口（原为 ensure_active 内部能力），供
    /// 需要"仅激活不操作"的消费方（如 nuphus-mcp 的 desktop_window_activate）使用。
    pub async fn activate(&self, target: &mut Target) -> Result<()> {
        self.ensure_active(target).await
    }

    /// 拖拽
    pub async fn drag(&self, target: &mut Target, start: Point, end: Point) -> Result<()> {
        self.ensure_active(target).await?;
        mouse::drag(start, end).await
    }

    /// 按键
    pub async fn press(&self, target: &mut Target, key: &str) -> Result<()> {
        self.ensure_active(target).await?;
        keyboard::press(key).await
    }

    /// 组合键
    pub async fn hotkey(&self, target: &mut Target, keys: &[&str]) -> Result<()> {
        self.ensure_active(target).await?;
        keyboard::hotkey(keys).await
    }

    /// 确保目标激活 (内建)
    async fn ensure_active(&self, target: &mut Target) -> Result<()> {
        if target.is_verified() {
            return Ok(());
        }

        #[cfg(windows)]
        {
            use ::windows::Win32::Foundation::HWND;
            use ::windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
            use std::time::Duration;
            use tokio::time::sleep;

            let hwnd = match target {
                Target::Window { hwnd, .. } => *hwnd,
                Target::Tui { hwnd, .. } => *hwnd,
                _ => return Ok(()),
            };

            let handle = HWND(hwnd);
            unsafe {
                let _ = SetForegroundWindow(handle);
            }
            sleep(Duration::from_millis(100)).await;

            // 验证是否前台
            if !self.is_foreground(hwnd) {
                // 强制前台: AttachThreadInput
                self.force_foreground(hwnd).await?;
            }
        }

        target.verify();
        Ok(())
    }

    #[cfg(windows)]
    fn is_foreground(&self, hwnd: isize) -> bool {
        use ::windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        unsafe {
            let fg = GetForegroundWindow();
            fg.0 as isize == hwnd
        }
    }

    #[cfg(windows)]
    async fn force_foreground(&self, hwnd: isize) -> Result<()> {
        use ::windows::Win32::Foundation::HWND;
        use ::windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use ::windows::Win32::UI::WindowsAndMessaging::{
            GetWindowThreadProcessId, SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        use std::time::Duration;
        use tokio::time::sleep;

        let handle = HWND(hwnd);
        let target_tid = unsafe { GetWindowThreadProcessId(handle, None) };
        let current_tid = unsafe { GetCurrentThreadId() };

        unsafe {
            let _ = AttachThreadInput(current_tid, target_tid, true);
            let _ = ShowWindow(handle, SW_RESTORE);
            let _ = SetForegroundWindow(handle);
            let _ = AttachThreadInput(current_tid, target_tid, false);
        }

        sleep(Duration::from_millis(200)).await;
        Ok(())
    }
}
