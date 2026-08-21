//! 鼠标控制 — Win32 原生 / macOS enigo / Linux PlatformNotSupported

use crate::core::*;

/// 创建共享 enigo 实例 (macOS / Linux)
#[cfg(not(windows))]
fn enigo() -> std::sync::Mutex<enigo::Enigo> {
    static INST: std::sync::OnceLock<std::sync::Mutex<enigo::Enigo>> = std::sync::OnceLock::new();
    INST.get_or_init(|| std::sync::Mutex::new(enigo::Enigo::new(&enigo::Settings::default())))
        .clone()
}

/// 移动鼠标到指定坐标
pub async fn move_to(x: i32, y: i32) -> Result<()> {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
        unsafe {
            // SetCursorPos 失败时返回 Err（如系统拦截 / 会话限制）——必须检查，不能静默吞掉
            if SetCursorPos(x, y).is_err() {
                return Err(DesktopError::InputFailed(format!(
                    "SetCursorPos({x}, {y}) 被系统拒绝，鼠标未移动"
                )));
            }
            // 移动后自校验：读取实际光标位置，确认到达目标（容差 2px 防 DPI 舍入）
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            if (pt.x - x).abs() > 2 || (pt.y - y).abs() > 2 {
                return Err(DesktopError::InputFailed(format!(
                    "鼠标移动校验失败：目标({x}, {y})，实际({}, {})——请检查坐标换算/屏幕 DPI/会话限制",
                    pt.x, pt.y
                )));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .move_mouse(x as i32, y as i32)
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 获取鼠标位置
pub async fn position() -> Result<Point> {
    #[cfg(windows)]
    {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut pt = POINT::default();
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return Err(DesktopError::InputFailed("GetCursorPos 失败".to_string()));
            }
        }
        Ok(Point { x: pt.x, y: pt.y })
    }
    #[cfg(not(windows))]
    {
        let pos = enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .location()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        Ok(Point {
            x: pos.0 as i32,
            y: pos.1 as i32,
        })
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 点击鼠标
pub async fn click(x: i32, y: i32) -> Result<()> {
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .button_click(enigo::MouseButton::Left)
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 右键点击
pub async fn right_click(x: i32, y: i32) -> Result<()> {
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        };
        unsafe {
            mouse_event(MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        unsafe {
            mouse_event(MOUSEEVENTF_RIGHTUP, 0, 0, 0, 0);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .button_click(enigo::MouseButton::Right)
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 滚动鼠标滚轮
///
/// `direction`: "up" / "down"；`amount`: 滚轮格数（每格 120 delta）。
pub async fn scroll(direction: &str, amount: i32) -> Result<()> {
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{mouse_event, MOUSEEVENTF_WHEEL};
        let delta: i32 = if direction == "up" { 120 } else { -120 };
        for _ in 0..amount.max(0) {
            unsafe {
                mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta, 0);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (direction, amount);
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 拖拽
pub async fn drag(start: Point, end: Point) -> Result<()> {
    move_to(start.x, start.y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        }
        let steps = 20;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let x = (start.x as f32 + (end.x as f32 - start.x as f32) * t) as i32;
            let y = (start.y as f32 + (end.y as f32 - start.y as f32) * t) as i32;
            move_to(x, y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let mut e = enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        e.button_down(enigo::MouseButton::Left)
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        drop(e);
        for i in 1..=20 {
            let t = i as f32 / 20.0;
            let x = (start.x as f32 + (end.x as f32 - start.x as f32) * t) as i32;
            let y = (start.y as f32 + (end.y as f32 - start.y as f32) * t) as i32;
            move_to(x, y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut e = enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        e.button_up(enigo::MouseButton::Left)
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}
