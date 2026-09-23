//! 鼠标控制 — Win32 原生 / macOS enigo / Linux PlatformNotSupported

use crate::core::*;

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
        use enigo::{Coordinate, Mouse};
        super::with_enigo(|engine| {
            engine
                .move_mouse(x, y, Coordinate::Abs)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
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
        use enigo::Mouse;
        let pos = super::with_enigo(|engine| {
            engine
                .location()
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })?;
        Ok(Point { x: pos.0, y: pos.1 })
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
        use enigo::{Button, Direction, Mouse};
        super::with_enigo(|engine| {
            engine
                .button(Button::Left, Direction::Click)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
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
        use enigo::{Button, Direction, Mouse};
        super::with_enigo(|engine| {
            engine
                .button(Button::Right, Direction::Click)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
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
    let _ = scroll_delta(direction, amount)?;
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
        use enigo::{Axis, Mouse};
        let delta = scroll_delta(direction, amount)?;
        super::with_enigo(|engine| {
            engine
                .scroll(delta, Axis::Vertical)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
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
            if let Err(error) = move_to(x, y).await {
                unsafe {
                    mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
                }
                return Err(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // 拖拽实现：Press → 移动（移动期间不持锁，避免阻塞其他输入）→ Release。
        // 用作用域块让 MutexGuard 在 await 前自然 drop——clippy await_holding_lock
        // 对显式 drop(e) 仍告警（版本差异），作用域块是跨 clippy 版本稳定的写法。
        use enigo::{Button, Direction, Mouse};
        {
            super::with_enigo(|engine| {
                engine
                    .button(Button::Left, Direction::Press)
                    .map_err(|e| DesktopError::InputFailed(e.to_string()))
            })?;
        }
        for i in 1..=20 {
            let t = i as f32 / 20.0;
            let x = (start.x as f32 + (end.x as f32 - start.x as f32) * t) as i32;
            let y = (start.y as f32 + (end.y as f32 - start.y as f32) * t) as i32;
            if let Err(error) = move_to(x, y).await {
                let _ = super::with_enigo(|engine| {
                    engine
                        .button(Button::Left, Direction::Release)
                        .map_err(|e| DesktopError::InputFailed(e.to_string()))
                });
                return Err(error);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        {
            super::with_enigo(|engine| {
                engine
                    .button(Button::Left, Direction::Release)
                    .map_err(|e| DesktopError::InputFailed(e.to_string()))
            })
        }
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

// Enigo defines positive vertical lengths as down and negative lengths as up.
fn scroll_delta(direction: &str, amount: i32) -> Result<i32> {
    if amount < 0 {
        return Err(DesktopError::InputFailed(
            "scroll amount must be non-negative".into(),
        ));
    }
    match direction {
        "up" => Ok(-amount),
        "down" => Ok(amount),
        _ => Err(DesktopError::InputFailed(format!(
            "unknown scroll direction: {direction}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_direction_matches_native_input_contract() {
        assert_eq!(scroll_delta("up", 3).unwrap(), -3);
        assert_eq!(scroll_delta("down", 3).unwrap(), 3);
        assert_eq!(scroll_delta("down", 0).unwrap(), 0);
        assert!(scroll_delta("up", -1).is_err());
        assert!(scroll_delta("sideways", 3).is_err());
    }
}
