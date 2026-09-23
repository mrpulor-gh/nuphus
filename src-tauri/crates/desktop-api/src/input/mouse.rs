//! 鼠标控制 — Win32 原生 / macOS CoreGraphics / Linux enigo

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
            GetCursorPos(&mut pt).map_err(|e| DesktopError::InputFailed(e.to_string()))?;
            if !pointer_reached(Point { x: pt.x, y: pt.y }, Point { x, y }) {
                return Err(DesktopError::InputFailed(format!(
                    "鼠标移动校验失败：目标({x}, {y})，实际({}, {})——请检查坐标换算/屏幕 DPI/会话限制",
                    pt.x, pt.y
                )));
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::move_to(x, y).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
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
    #[cfg(target_os = "macos")]
    {
        macos::position()
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
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
    #[cfg(not(target_os = "macos"))]
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_LEFTUP);
        unsafe {
            mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::click_at(x, y, "left", 1).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
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
    #[cfg(not(target_os = "macos"))]
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_RIGHTUP);
        unsafe {
            mouse_event(MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::click_at(x, y, "right", 1).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
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

/// Middle button, preserving the same screen-coordinate contract as left/right click.
pub async fn middle_click(x: i32, y: i32) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    move_to(x, y).await?;
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_MIDDLEUP);
        unsafe {
            mouse_event(MOUSEEVENTF_MIDDLEDOWN, 0, 0, 0, 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::click_at(x, y, "middle", 1).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        use enigo::{Button, Direction, Mouse};
        super::with_enigo(|engine| {
            engine
                .button(Button::Middle, Direction::Click)
                .map_err(|error| DesktopError::InputFailed(error.to_string()))
        })
    }
}

/// 滚动鼠标滚轮
///
/// `direction`: "up" / "down" / "left" / "right"；`amount`: 滚轮格数。
pub async fn scroll(direction: &str, amount: i32) -> Result<()> {
    let _ = scroll_delta(direction, amount)?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL,
        };
        // Win32 vertical positive means up; horizontal positive means right.
        let delta: i32 = if matches!(direction, "up" | "right") {
            120
        } else {
            -120
        };
        let flags = if matches!(direction, "left" | "right") {
            MOUSEEVENTF_HWHEEL
        } else {
            MOUSEEVENTF_WHEEL
        };
        for _ in 0..amount.max(0) {
            unsafe {
                mouse_event(flags, 0, 0, delta, 0);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use enigo::{Axis, Mouse};
        let delta = scroll_delta(direction, amount)?;
        let axis = if matches!(direction, "left" | "right") {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        super::with_enigo(|engine| {
            engine
                .scroll(delta, axis)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))
        })
    }
}

/// 拖拽
pub async fn drag(start: Point, end: Point) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    move_to(start.x, start.y).await?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            mouse_event, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        };
        let mut held = super::HeldInput::new(|flags| unsafe { mouse_event(flags, 0, 0, 0, 0) });
        held.hold(MOUSEEVENTF_LEFTUP);
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
        held.release_last();
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        macos::drag(start, end).await
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        // 拖拽实现：Press → 移动（移动期间不持锁，避免阻塞其他输入）→ Release。
        // 用作用域块让 MutexGuard 在 await 前自然 drop——clippy await_holding_lock
        // 对显式 drop(e) 仍告警（版本差异），作用域块是跨 clippy 版本稳定的写法。
        use enigo::{Button, Direction, Mouse};
        let mut release_error = None;
        let mut held = super::HeldInput::new(|button| {
            if let Err(error) = super::with_enigo(|engine| {
                engine
                    .button(button, Direction::Release)
                    .map_err(|error| DesktopError::InputFailed(error.to_string()))
            }) {
                release_error = Some(error);
            }
        });
        // Register before sending: a native error can follow a successful down.
        held.hold(Button::Left);
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
            move_to(x, y).await?;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        held.release_last();
        drop(held);
        match release_error {
            Some(error) => Err(error),
            None => Ok(()),
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
        "up" | "left" => Ok(-amount),
        "down" | "right" => Ok(amount),
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
        assert_eq!(scroll_delta("left", 3).unwrap(), -3);
        assert_eq!(scroll_delta("right", 3).unwrap(), 3);
        assert_eq!(scroll_delta("down", 0).unwrap(), 0);
        assert!(scroll_delta("up", -1).is_err());
        assert!(scroll_delta("sideways", 3).is_err());
    }

    #[test]
    fn pointer_verification_uses_global_coordinates_including_negative_displays() {
        let requested = Point { x: -1200, y: 400 };
        assert!(pointer_reached(Point { x: -1198, y: 399 }, requested));
        assert!(!pointer_reached(Point { x: -1200, y: 800 }, requested));
        assert!(!pointer_reached(Point { x: 1200, y: 400 }, requested));
        assert!(!pointer_reached(
            Point { x: i32::MIN, y: 0 },
            Point { x: i32::MAX, y: 0 }
        ));
    }
}

#[cfg(target_os = "macos")]
pub async fn click_at(x: i32, y: i32, button: &str, clicks: i32) -> Result<()> {
    macos::click_at(x, y, button, clicks).await
}

#[cfg(any(windows, target_os = "macos", test))]
fn pointer_reached(actual: Point, expected: Point) -> bool {
    i64::from(actual.x).abs_diff(i64::from(expected.x)) <= 2
        && i64::from(actual.y).abs_diff(i64::from(expected.y)) <= 2
}

// CoreGraphics uses the same top-left global coordinate system as AX and our captures.
// Never re-read Cocoa mouseLocation to synthesize a button event at a different point.
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    type Event = *const c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: Event) -> Event;
        fn CGEventGetLocation(event: Event) -> CGPoint;
        fn CGEventCreateMouseEvent(source: Event, kind: u32, point: CGPoint, button: u32) -> Event;
        fn CGEventSetIntegerValueField(event: Event, field: u32, value: i64);
        fn CGEventPost(tap: u32, event: Event);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: Event);
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }
    static INPUT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn check_permission() -> Result<()> {
        if unsafe { AXIsProcessTrusted() } == 0 {
            return Err(DesktopError::InputFailed(
                "请在系统设置→隐私与安全性→辅助功能中授权 Nuphus 后重试".into(),
            ));
        }
        Ok(())
    }
    pub fn position() -> Result<Point> {
        let event = unsafe { CGEventCreate(std::ptr::null()) };
        if event.is_null() {
            return Err(DesktopError::InputFailed("CGEventCreate failed".into()));
        }
        let point = unsafe { CGEventGetLocation(event) };
        unsafe {
            CFRelease(event);
        }
        Ok(Point {
            x: point.x.round() as i32,
            y: point.y.round() as i32,
        })
    }
    fn post(point: Point, kind: u32, button: u32, click_count: i64) -> Result<()> {
        let event = unsafe {
            CGEventCreateMouseEvent(
                std::ptr::null(),
                kind,
                CGPoint {
                    x: point.x as f64,
                    y: point.y as f64,
                },
                button,
            )
        };
        if event.is_null() {
            return Err(DesktopError::InputFailed(
                "CGEventCreateMouseEvent failed".into(),
            ));
        }
        unsafe {
            CGEventSetIntegerValueField(event, 1, click_count);
            CGEventPost(0, event);
            CFRelease(event);
        }
        Ok(())
    }
    async fn verify_position(expected: Point) -> Result<()> {
        for _ in 0..20 {
            let actual = position()?;
            if pointer_reached(actual, expected) {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let actual = position()?;
        Err(DesktopError::InputFailed(format!(
            "鼠标位置校验失败：请求屏幕坐标 ({}, {})，实际 ({}, {})",
            expected.x, expected.y, actual.x, actual.y
        )))
    }
    pub async fn move_to(x: i32, y: i32) -> Result<()> {
        let _guard = INPUT.lock().await;
        check_permission()?;
        let point = Point { x, y };
        post(point, 5, 0, 0)?;
        verify_position(point).await
    }
    struct Release {
        point: Point,
        kind: u32,
        button: u32,
        count: i64,
        armed: bool,
    }
    impl Drop for Release {
        fn drop(&mut self) {
            if self.armed {
                let _ = post(self.point, self.kind, self.button, self.count);
            }
        }
    }
    pub async fn click_at(x: i32, y: i32, button: &str, clicks: i32) -> Result<()> {
        let (button, down, up) = match button {
            "left" => (0, 1, 2),
            "right" => (1, 3, 4),
            "middle" => (2, 25, 26),
            _ => return Err(DesktopError::InputFailed("unknown mouse button".into())),
        };
        if !(1..=2).contains(&clicks) {
            return Err(DesktopError::InputFailed("clicks must be 1 or 2".into()));
        }
        let _guard = INPUT.lock().await;
        check_permission()?;
        let point = Point { x, y };
        post(point, 5, button, 0)?;
        verify_position(point).await?;
        for count in 1..=clicks {
            check_permission()?;
            let mut release = Release {
                point,
                kind: up,
                button,
                count: count.into(),
                armed: true,
            };
            post(point, down, button, count.into())?;
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            post(point, up, button, count.into())?;
            release.armed = false;
            if count < clicks {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
        }
        Ok(())
    }
    pub async fn drag(start: Point, end: Point) -> Result<()> {
        let _guard = INPUT.lock().await;
        check_permission()?;
        post(start, 5, 0, 0)?;
        verify_position(start).await?;
        let mut release = Release {
            point: start,
            kind: 2,
            button: 0,
            count: 1,
            armed: true,
        };
        post(start, 1, 0, 1)?;
        for step in 1..=20 {
            check_permission()?;
            let point = Point {
                x: (i64::from(start.x) + (i64::from(end.x) - i64::from(start.x)) * step / 20)
                    as i32,
                y: (i64::from(start.y) + (i64::from(end.y) - i64::from(start.y)) * step / 20)
                    as i32,
            };
            release.point = point;
            post(point, 6, 0, 1)?;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        post(end, 2, 0, 1)?;
        release.armed = false;
        verify_position(end).await
    }
}
