//! Desktop Client — Rust native desktop control
//!
//! Desktop automation based on desktop-api crate (Win32 + xcap).
//! All features implemented natively in Rust, zero Python dependencies.

use crate::Result;
use serde_json::Value;
use std::path::PathBuf;

#[cfg(not(windows))]
use desktop_api::SendEnigo;
use desktop_api::{
    capture, clipboard as desk_clip, input, FindResult, Frame, FrameSource, Locator, Query, Scope,
    Target,
};
#[cfg(windows)]
use desktop_api::{sendinput, WindowManager};

use crate::desktop::YoloDetector;

// enigo 0.2: text/key/scroll 是 Keyboard/Mouse trait 方法，调用需 import（Linux/macOS）
// Direction 用全路径 enigo::Direction（避免 unused import）
#[cfg(not(windows))]
use enigo::{Keyboard, Mouse};

/// Desktop client — native Rust desktop control
#[derive(Clone)]
pub struct DesktopClient {
    /// Window manager (with LRU cache) — Windows-only（Linux 走 linux_window X11 实现）
    #[cfg(windows)]
    window_manager: std::sync::Arc<std::sync::Mutex<WindowManager>>,
    /// YOLO icon detector
    yolo: std::sync::Arc<YoloDetector>,
}

impl Default for DesktopClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopClient {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            window_manager: std::sync::Arc::new(std::sync::Mutex::new(WindowManager::new())),
            yolo: std::sync::Arc::new(YoloDetector::new()),
        }
    }

    fn result_ok(value: impl serde::Serialize) -> Result<Value> {
        Ok(serde_json::json!({ "success": true, "result": value }))
    }

    /// Execute osascript and return stdout (macOS), prompt for accessibility permission on failure
    #[cfg(target_os = "macos")]
    fn osascript(script: &str) -> Result<String> {
        let output = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
            .map_err(|e| {
                desktop_api::DesktopError::InputFailed(format!(
                    "osascript failed: {}. 请检查 系统设置→隐私与安全性→辅助功能 中是否已授权。",
                    e
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = if stderr.contains("not allowed") || stderr.contains("permission") {
                format!("macOS 辅助功能权限不足：{}. 请在 系统设置→隐私与安全性→辅助功能 中授权后重试。", stderr.trim())
            } else {
                format!("osascript error: {}", stderr.trim())
            };
            return Err(desktop_api::DesktopError::InputFailed(msg).into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn result_err(msg: impl Into<String>) -> Result<Value> {
        Ok(serde_json::json!({ "success": false, "error": msg.into() }))
    }

    // ══════════════════════════════════════════════
    //  Methods below use Rust native desktop-api implementation
    // ══════════════════════════════════════════════

    /// Mouse click — cross-platform (Win32 native / macOS enigo)
    pub async fn mouse_click(&self, x: i32, y: i32, button: &str, clicks: i32) -> Result<Value> {
        input::mouse::move_to(x, y).await?;
        for _ in 0..clicks {
            match button {
                "right" => input::mouse::right_click(x, y).await?,
                _ => input::mouse::click(x, y).await?,
            }
            if clicks > 1 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        Self::result_ok(serde_json::json!({ "x": x, "y": y, "button": button, "clicks": clicks }))
    }

    /// Mouse hover — cross-platform (Win32 native / macOS enigo)
    pub async fn mouse_hover(&self, x: i32, y: i32) -> Result<Value> {
        input::mouse::move_to(x, y).await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        Self::result_ok(serde_json::json!({ "x": x, "y": y, "action": "hover" }))
    }

    /// Mouse move — cross-platform (Win32 native / macOS enigo)
    pub async fn mouse_move(&self, x: i32, y: i32, _duration: f64) -> Result<Value> {
        input::mouse::move_to(x, y).await?;
        Self::result_ok(serde_json::json!({ "x": x, "y": y }))
    }

    /// Get mouse position — cross-platform (Win32 native / macOS enigo)
    pub async fn mouse_position(&self) -> Result<Value> {
        let pt = input::mouse::position().await?;
        Self::result_ok(serde_json::json!({ "x": pt.x, "y": pt.y }))
    }

    /// Mouse drag — cross-platform (Win32 native / macOS enigo)
    pub async fn mouse_drag(
        &self,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<Value> {
        let start = desktop_api::Point {
            x: start_x,
            y: start_y,
        };
        let end = desktop_api::Point { x: end_x, y: end_y };
        input::mouse::drag(start, end).await?;
        Self::result_ok(serde_json::json!({
            "start": { "x": start_x, "y": start_y },
            "end": { "x": end_x, "y": end_y }
        }))
    }

    /// Mouse scroll — Win32 SendInput / macOS & Linux enigo
    pub async fn mouse_scroll(&self, direction: &str, amount: i32) -> Result<Value> {
        #[cfg(windows)]
        {
            use ::windows::Win32::UI::Input::KeyboardAndMouse::{
                SendInput, INPUT, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
            };
            let delta: u32 = match direction {
                "up" => (amount * 120) as u32,
                _ => ((-amount) * 120) as u32,
            };
            let scroll_input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: delta,
                        dwFlags: MOUSEEVENTF_WHEEL,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            unsafe {
                SendInput(&[scroll_input], std::mem::size_of::<INPUT>() as i32);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let mut e = Self::enigo_handle()
                .lock()
                .map_err(|e| format!("enigo: {e}"))?;
            let len = match direction {
                "up" => amount,
                _ => amount,
            };
            e.scroll(len, enigo::Axis::Vertical)
                .map_err(|e| format!("scroll: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            // Linux: 使用 enigo XTest 模拟滚轮 (X11 only, Wayland 不可用)
            // enigo 0.2 的 scroll 方向由 Axis::Vertical 正负决定
            let mut e = Self::enigo_handle()
                .lock()
                .map_err(|e| format!("enigo: {e}"))?;
            let len = match direction {
                "up" => amount,
                _ => -amount,
            };
            // XTest scroll 使用 button 4/5 (up/down), enigo 封装了此逻辑
            e.scroll(len, enigo::Axis::Vertical)
                .map_err(|e| format!("scroll: {e}"))?;
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            return Err(DesktopError::PlatformNotSupported.into());
        }
        Self::result_ok(serde_json::json!({ "direction": direction, "amount": amount }))
    }

    /// Keyboard text input — Windows: IME native / macOS: enigo
    pub async fn keyboard_type_unicode(&self, text: &str) -> Result<Value> {
        #[cfg(windows)]
        {
            sendinput::nuphus_input(text, &sendinput::InputSession::default())?;
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let mut e = Self::enigo_handle()
                .lock()
                .map_err(|e| format!("enigo: {e}"))?;
            e.text(text).map_err(|e| format!("text: {e}"))?;
        }
        #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
        {
            return Err(DesktopError::PlatformNotSupported.into());
        }
        Self::result_ok(serde_json::json!({ "chars": text.len() }))
    }

    /// Keyboard key press — cross-platform via input::keyboard
    pub async fn keyboard_press(&self, key: &str) -> Result<Value> {
        input::keyboard::press(key).await?;
        Self::result_ok(serde_json::json!({ "key": key }))
    }

    /// Keyboard hotkey — cross-platform via input::keyboard
    pub async fn keyboard_hotkey(&self, keys: Vec<String>) -> Result<Value> {
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        input::keyboard::hotkey(&key_refs).await?;
        Self::result_ok(serde_json::json!({ "keys": keys }))
    }

    /// Send text input to window — Windows: AttachThreadInput ensures target receives input
    /// Caller must ensure the target window is foreground first (ensure_foreground).
    #[cfg_attr(not(windows), allow(unused_variables))] // hwnd 仅 Windows 使用
    pub async fn input_send(&self, text: &str, hwnd: i32, press_enter: bool) -> Result<Value> {
        #[cfg(windows)]
        {
            // Attach to target window's thread to prevent input from going to wrong window
            // (defends against focus-stealing by notifications, IME popups, etc.)
            use windows::Win32::Foundation::HWND;
            use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
            use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
            let handle = HWND(hwnd as isize);
            let target_tid = unsafe { GetWindowThreadProcessId(handle, None) };
            let current_tid = unsafe { GetCurrentThreadId() };
            unsafe {
                _ = AttachThreadInput(current_tid, target_tid, true);
            }

            sendinput::nuphus_input(text, &sendinput::InputSession::default())?;
            if press_enter {
                use ::windows::Win32::UI::Input::KeyboardAndMouse::{
                    keybd_event, KEYEVENTF_KEYUP, VK_RETURN,
                };
                unsafe {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    keybd_event(VK_RETURN.0 as u8, 0, Default::default(), 0);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    keybd_event(VK_RETURN.0 as u8, 0, KEYEVENTF_KEYUP, 0);
                }
            }

            unsafe {
                _ = AttachThreadInput(current_tid, target_tid, false);
            }
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            // 不能持有 MutexGuard 跨 await（future 需 Send）：text 完成后立即释放锁，
            // sleep 后再重新取锁执行 key。
            {
                let mut e = Self::enigo_handle()
                    .lock()
                    .map_err(|e| format!("enigo: {e}"))?;
                if !text.is_empty() {
                    e.text(text).map_err(|e| format!("text: {e}"))?;
                }
            }
            if press_enter {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let mut e = Self::enigo_handle()
                    .lock()
                    .map_err(|e| format!("enigo: {e}"))?;
                e.key(enigo::Key::Return, enigo::Direction::Click)
                    .map_err(|e| format!("enter: {e}"))?;
            }
        }
        #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
        {
            return Err(DesktopError::PlatformNotSupported.into());
        }
        Self::result_ok(serde_json::json!({ "chars": text.len(), "enter": press_enter }))
    }

    /// macOS / Linux enigo helper
    /// （命名为 enigo_handle 避免与 enigo crate 同名遮蔽；返回 &'static Mutex 供 .lock() 借用，
    ///  不可返回 Arc——临时 Arc 会在语句结束 drop 导致 MutexGuard 悬垂 E0716）
    #[cfg(not(windows))]
    fn enigo_handle() -> &'static std::sync::Mutex<SendEnigo> {
        static INST: std::sync::OnceLock<std::sync::Mutex<SendEnigo>> = std::sync::OnceLock::new();
        INST.get_or_init(|| {
            // SendEnigo: macOS 上 Enigo 非 Send（CGEventSource 指针），经 Mutex 串行化后包装为 Send+Sync。
            std::sync::Mutex::new(SendEnigo(
                enigo::Enigo::new(&enigo::Settings::default()).expect("enigo init failed"),
            ))
        })
    }

    /// Screenshot - save as BMP format to unified directory
    ///
    /// Path rules:
    /// - User-specified path: use specified path (still BMP)
    /// - No path specified: save to ~/.nuphus/captures/screen_{timestamp}.bmp
    pub async fn screenshot(&self, path: Option<&str>, region: Option<Value>) -> Result<Value> {
        let scope = match region {
            Some(ref r) => {
                let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let w = r.get("width").and_then(|v| v.as_u64()).unwrap_or(1920) as u32;
                let h = r.get("height").and_then(|v| v.as_u64()).unwrap_or(1080) as u32;
                Scope::Element { x, y, w, h }
            }
            None => Scope::Fullscreen,
        };

        // 截图
        let dummy_target = Target::Tui {
            hwnd: 0,
            title: String::new(),
        };
        let frame = capture::capture(&dummy_target, scope).await?;

        // Determine save path — force .bmp extension
        let save_path = if let Some(p) = path {
            let mut pb = PathBuf::from(p);
            // Force extension replacement to .bmp
            pb.set_extension("bmp");
            pb
        } else {
            let captures_dir = Self::captures_dir()?;
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            captures_dir.join(format!("screen_{}.bmp", timestamp))
        };

        // Ensure directory exists
        if let Some(parent) = save_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // Save as BMP
        Self::save_frame_as_bmp(&frame, &save_path)?;

        let (screen_x, screen_y) = match &region {
            Some(r) => (
                r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            ),
            None => (0, 0),
        };

        Self::result_ok(serde_json::json!({
            "width": frame.width,
            "height": frame.height,
            "screen_x": screen_x,
            "screen_y": screen_y,
            "path": save_path.display().to_string(),
        }))
    }

    /// Window screenshot - BMP format
    pub async fn window_screenshot(
        &self,
        title: Option<&str>,
        hwnd: Option<i32>,
        path: Option<&str>,
    ) -> Result<Value> {
        let hwnd_val = match (hwnd, title) {
            (Some(h), _) => h as isize,
            #[cfg(windows)]
            (_, Some(t)) => {
                let mut wm = self
                    .window_manager
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let target = wm.find(t)?;
                match target {
                    desktop_api::Target::Window { hwnd, .. } => hwnd,
                    _ => return Self::result_err("target is not a window"),
                }
            }
            #[cfg(not(windows))]
            (_, Some(_t)) => {
                return Self::result_err("按标题查找窗口仅支持 Windows，请传入 hwnd");
            }
            (None, None) => return Self::result_err("hwnd or title required"),
        };

        #[cfg(windows)]
        let target = desktop_api::Target::Window {
            hwnd: hwnd_val,
            title: String::new(),
            verified: false,
            gfx_backend: desktop_api::GfxBackend::Unknown,
        };
        // 非 Windows：Target::Window 变体不存在（cfg(windows)），capture 会忽略 target 回退全屏
        #[cfg(not(windows))]
        let target = desktop_api::Target::Tui {
            hwnd: hwnd_val,
            title: String::new(),
        };
        let frame = capture::capture(&target, Scope::Window).await?;

        // Determine save path — force .bmp extension
        let save_path = if let Some(p) = path {
            let mut pb = PathBuf::from(p);
            pb.set_extension("bmp");
            pb
        } else {
            let captures_dir = Self::captures_dir()?;
            let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
            captures_dir.join(format!("window_{}_{}.bmp", hwnd_val, timestamp))
        };

        if let Some(parent) = save_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        Self::save_frame_as_bmp(&frame, &save_path)?;

        // 获取窗口屏幕坐标（ClientToScreen 得到客户区左上角屏幕位置）
        let (screen_x, screen_y) = {
            #[cfg(windows)]
            {
                use windows::Win32::Foundation::{HWND, POINT, RECT};
                use windows::Win32::Graphics::Gdi::ClientToScreen;
                use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
                let hwnd_ptr = HWND(hwnd_val);
                let mut client = RECT::default();
                let mut origin = POINT { x: 0, y: 0 };
                unsafe {
                    _ = GetClientRect(hwnd_ptr, &mut client);
                }
                unsafe {
                    _ = ClientToScreen(hwnd_ptr, &mut origin);
                }
                (origin.x, origin.y)
            }
            #[cfg(not(windows))]
            (0, 0)
        };

        Self::result_ok(serde_json::json!({
            "width": frame.width,
            "height": frame.height,
            "screen_x": screen_x,
            "screen_y": screen_y,
            "path": save_path.display().to_string(),
            "format": "bmp",
            "hwnd": hwnd_val,
        }))
    }

    /// Get unified screenshot directory
    ///
    /// Priority: NUPHUS_CAPTURES_DIR env var > data_local_dir/nuphus/captures
    fn captures_dir() -> Result<PathBuf> {
        captures_dir_path().map_err(|e| crate::NuphusError::Tool(e.to_string()))
    }

    /// Save Frame as BMP file
    fn save_frame_as_bmp(frame: &desktop_api::Frame, path: &PathBuf) -> Result<()> {
        use std::io::Write;

        let w = frame.width;
        let h = frame.height;
        let row_size = (w * 3).div_ceil(4) * 4; // BMP row size must be a multiple of 4
        let padding = row_size - w * 3;
        let pixel_data_size = row_size * h;
        let file_size = 54 + pixel_data_size; // 14 + 40 字节头

        let mut file = std::fs::File::create(path)
            .map_err(|e| crate::NuphusError::Tool(format!("create bmp file failed: {}", e)))?;

        // BMP file header (14 bytes)
        let file_header: [u8; 14] = [
            b'B',
            b'M', // 签名
            (file_size & 0xFF) as u8,
            ((file_size >> 8) & 0xFF) as u8,
            ((file_size >> 16) & 0xFF) as u8,
            ((file_size >> 24) & 0xFF) as u8,
            0,
            0,
            0,
            0, // 保留
            54,
            0,
            0,
            0, // 数据偏移
        ];
        file.write_all(&file_header)
            .map_err(|e| crate::NuphusError::Tool(e.to_string()))?;

        // DIB header (BITMAPINFOHEADER, 40 bytes)
        let dib_header: [u8; 40] = [
            40,
            0,
            0,
            0, // 头大小
            (w & 0xFF) as u8,
            ((w >> 8) & 0xFF) as u8,
            ((w >> 16) & 0xFF) as u8,
            ((w >> 24) & 0xFF) as u8,
            (h & 0xFF) as u8,
            ((h >> 8) & 0xFF) as u8,
            ((h >> 16) & 0xFF) as u8,
            ((h >> 24) & 0xFF) as u8,
            1,
            0, // 平面数
            24,
            0, // 位深 (24bit RGB)
            0,
            0,
            0,
            0, // 压缩 (无)
            (pixel_data_size & 0xFF) as u8,
            ((pixel_data_size >> 8) & 0xFF) as u8,
            ((pixel_data_size >> 16) & 0xFF) as u8,
            ((pixel_data_size >> 24) & 0xFF) as u8,
            0,
            0,
            0,
            0, // X pixels per meter
            0,
            0,
            0,
            0, // Y pixels per meter
            0,
            0,
            0,
            0, // 颜色数
            0,
            0,
            0,
            0, // 重要颜色数
        ];
        file.write_all(&dib_header)
            .map_err(|e| crate::NuphusError::Tool(e.to_string()))?;

        // Pixel data (BGR format, bottom to top)
        let pixels = &frame.pixels;
        for row in (0..h).rev() {
            for col in 0..w {
                let idx = ((row * w + col) * 4) as usize;
                let r = pixels[idx];
                let g = pixels[idx + 1];
                let b = pixels[idx + 2];
                // BMP uses BGR
                file.write_all(&[b, g, r])
                    .map_err(|e| crate::NuphusError::Tool(e.to_string()))?;
            }
            // Row padding
            if padding > 0 {
                file.write_all(&vec![0u8; padding as usize])
                    .map_err(|e| crate::NuphusError::Tool(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Screen size
    pub async fn screen_size(&self) -> Result<Value> {
        use xcap::Monitor;
        let monitors = Monitor::all()
            .map_err(|e| crate::NuphusError::Tool(format!("monitor query failed: {}", e)))?;
        let primary = monitors
            .into_iter()
            .next()
            .ok_or_else(|| crate::NuphusError::Tool("no monitor found".to_string()))?;
        // xcap 0.9: width()/height() 返回 Result（0.0.14 直接返回 u32）
        let width = primary
            .width()
            .map_err(|e| crate::NuphusError::Tool(format!("monitor width failed: {e}")))?;
        let height = primary
            .height()
            .map_err(|e| crate::NuphusError::Tool(format!("monitor height failed: {e}")))?;
        Self::result_ok(serde_json::json!({
            "width": width,
            "height": height,
        }))
    }

    /// Get process name from PID (executable file name) — Windows-only
    #[cfg(windows)]
    fn process_name_from_pid(pid: u32) -> Option<String> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Threading::OpenProcess;
            use windows::Win32::System::Threading::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

            unsafe {
                let handle =
                    match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
                        Ok(h) => h,
                        Err(_) => return None,
                    };
                let mut buf = [0u16; 260];
                let result = windows::Win32::System::ProcessStatus::GetModuleBaseNameW(
                    handle, None, &mut buf,
                );
                _ = CloseHandle(handle);
                if result > 0 {
                    Some(String::from_utf16_lossy(&buf[..result as usize]))
                } else {
                    None
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            // macOS: 通过 ps 命令获取进程名
            if let Ok(output) = std::process::Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "comm="])
                .output()
            {
                if output.status.success() {
                    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
            None
        }
        #[cfg(target_os = "linux")]
        {
            // Linux: 读取 /proc/{pid}/comm
            let comm_path = format!("/proc/{}/comm", pid);
            if let Ok(name) = std::fs::read_to_string(&comm_path) {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            None
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            None
        }
    }

    /// Classify window type based on process name, class name and title
    /// Window type classification (used by Windows window info) — Windows-only
    #[cfg(windows)]
    fn classify_window(process_name: &str, class_name: &str, title: &str) -> &'static str {
        // 1) 按 process_name 识别（优先级最高）
        match process_name.to_lowercase().as_str() {
            "explorer.exe" if !title.is_empty() => return "folder",
            "explorer.exe" => return "desktop",
            "code.exe" | "code-oss.exe" => return "ide",
            "devenv.exe" | "devenv" => return "ide",
            "chrome.exe" | "msedge.exe" | "firefox.exe" | "opera.exe" | "brave.exe" => {
                return "browser"
            }
            "wechat.exe" | "wechatdevtools.exe" => return "messenger",
            "slack.exe" | "discord.exe" => return "messenger",
            "outlook.exe" | "winmail.exe" => return "email",
            "taskmgr.exe" => return "system_tool",
            "wt.exe"
            | "windowsterminal.exe"
            | "powershell.exe"
            | "cmd.exe"
            | "pwsh.exe"
            | "windows-terminal.exe" => return "terminal",
            _ => {}
        }

        // 2) 按 class_name 识别（覆盖 Electron/Qt/Console 等跨进程识别）
        match class_name {
            "ConsoleWindowClass" => return "terminal",
            "#32770" => return "dialog",
            "Chrome_WidgetWin_1" | "Chrome_WidgetWin_2" | "CefBrowserWindow" => return "browser",
            "Qt5QWindowIcon" | "Qt6QWindowIcon" => return "native_app",
            "Windows.UI.Core.CoreWindow" => return "modern_app",
            "CabinetWClass" => return "folder",
            c if c.starts_with("HwndWrapper") => return "native_app", // WPF
            _ => {}
        }

        // 3) 按标题启发式识别（兜底）
        let lower_title = title.to_lowercase();
        if lower_title.contains(" - visual studio code")
            || lower_title.contains(" — visual studio code")
            || lower_title.contains(" - vs code")
        {
            return "ide";
        }
        if lower_title.contains(" - powershell") || lower_title.contains("命令提示符") {
            return "terminal";
        }

        "generic"
    }

    /// 检测 UWP 窗口是否被系统 cloaked（挂起/幽灵）
    #[cfg(windows)]
    fn is_window_cloaked(hwnd: i32) -> bool {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        let mut cloaked: u32 = 0;
        let hwnd_ptr = HWND(hwnd as isize);
        unsafe {
            let result = DwmGetWindowAttribute(
                hwnd_ptr,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            result.is_ok() && cloaked != 0
        }
    }

    /// List windows
    pub async fn windows_list(&self) -> Result<Value> {
        #[cfg(windows)]
        {
            let wm = self
                .window_manager
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let windows = wm.list_all()?;
            let simplified: Vec<Value> = windows
                .iter()
                .map(|w| {
                    let process_name = if w.process_id > 0 {
                        Self::process_name_from_pid(w.process_id).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    serde_json::json!({
                        "hwnd": w.hwnd,
                        "title": w.title.chars().take(60).collect::<String>(),
                        "x": w.x, "y": w.y,
                        "width": w.width, "height": w.height,
                        "process_id": w.process_id,
                        "process_name": process_name,
                        "cloaked": Self::is_window_cloaked(w.hwnd as i32),
                    })
                })
                .collect();
            Self::result_ok(simplified)
        }
        #[cfg(target_os = "macos")]
        {
            // 单次 osascript 调用获取所有窗口信息，替代逐窗口多次调用
            let script = r#"tell app "System Events"
    set output to ""
    repeat with proc in every process whose background only is false
        repeat with w in every window of proc
            try
                set wid to id of w
                set ttl to title of w
                set {px, py} to position of w
                set {pw, ph} to size of w
                set output to output & wid & "|||" & ttl & "|||" & px & "|||" & py & "|||" & pw & "|||" & ph & "|||" & (name of proc) & "\n"
            end try
        end repeat
    end repeat
    return output
end tell"#;
            let output = Self::osascript(script)?;
            let mut windows = Vec::new();
            for line in output.lines() {
                let parts: Vec<&str> = line.split("|||").collect();
                if parts.len() >= 7 {
                    if let (Ok(hwnd), Ok(x), Ok(y), Ok(w), Ok(h)) = (
                        parts[0].trim().parse::<i64>(),
                        parts[2].trim().parse::<i32>(),
                        parts[3].trim().parse::<i32>(),
                        parts[4].trim().parse::<i32>(),
                        parts[5].trim().parse::<i32>(),
                    ) {
                        let title = parts[1].trim().to_string();
                        let process_name = parts[6].trim().to_string();
                        windows.push(serde_json::json!({
                            "hwnd": hwnd, "title": title,
                            "x": x, "y": y,
                            "width": w, "height": h,
                            "process_name": process_name,
                        }));
                    }
                }
            }
            Self::result_ok(windows)
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop::linux_window::windows_list()
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    /// Activate window — returns whether the window is now foreground
    pub async fn window_activate(&self, hwnd: i32) -> Result<Value> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
            use windows::Win32::UI::WindowsAndMessaging::{
                GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SetForegroundWindow,
                SetWindowPos, ShowWindow, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
                SW_RESTORE,
            };
            let handle = HWND(hwnd as isize);
            unsafe {
                // 最小化则还原（等动画完成再置前，否则 SetForegroundWindow 大概率失败）
                if IsIconic(handle).as_bool() {
                    if !ShowWindow(handle, SW_RESTORE).as_bool() {
                        tracing::warn!("[desktop] ShowWindow(SW_RESTORE) failed for hwnd={}", hwnd);
                    }
                    // 等待还原动画完成（Windows 通常需要 300-400ms）
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                }
                // 优先用 AttachThreadInput + SetForegroundWindow（标准路径）
                let target_tid = GetWindowThreadProcessId(handle, None);
                let current_tid = GetCurrentThreadId();
                _ = AttachThreadInput(current_tid, target_tid, true);
                if !SetForegroundWindow(handle).as_bool() {
                    tracing::warn!("[desktop] SetForegroundWindow failed for hwnd={}", hwnd);
                }
                _ = AttachThreadInput(current_tid, target_tid, false);
                // 验证是否真的到了前台
                let fg = GetForegroundWindow();
                if fg != handle {
                    // SetForegroundWindow 失败（前台锁/权限），回退方案：
                    // 1. SetWindowPos HWND_TOP 确保 Z-order 在最前
                    _ = SetWindowPos(
                        handle,
                        HWND_TOP,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOMOVE | SWP_SHOWWINDOW,
                    );
                    // 2. 再试一次 SetForegroundWindow
                    _ = AttachThreadInput(current_tid, target_tid, true);
                    if !SetForegroundWindow(handle).as_bool() {
                        tracing::warn!(
                            "[desktop] SetForegroundWindow(macOS fallback) failed for hwnd={}",
                            hwnd
                        );
                    }
                    _ = AttachThreadInput(current_tid, target_tid, false);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            // Final verification
            let fg = unsafe { GetForegroundWindow().0 as isize };
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "foreground": HWND(fg) == handle }))
        }
        #[cfg(target_os = "macos")]
        {
            // 最多重试 3 次激活 + 验证
            let mut fg = false;
            for i in 0..3 {
                if i > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                let _ = Self::osascript(&format!(
                    r#"tell app "System Events" to set frontmost of window id {} to true"#,
                    hwnd
                ));
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                // 验证：查询该窗口是否在最前
                let check = Self::osascript(&format!(
                    r#"tell app "System Events" to get value of attribute "AXMain" of window id {}"#, hwnd
                )).unwrap_or_default();
                if check == "true" || check == "1" {
                    fg = true;
                    break;
                }
            }
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "foreground": fg }))
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop::linux_window::window_activate(hwnd)
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    /// Check if window is currently foreground (no activation attempt)
    pub async fn window_is_foreground(&self, hwnd: i32) -> Result<Value> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let fg = unsafe { GetForegroundWindow() };
            Self::result_ok(
                serde_json::json!({ "hwnd": hwnd, "foreground": HWND(hwnd as isize) == fg }),
            )
        }
        #[cfg(target_os = "macos")]
        {
            let check = Self::osascript(&format!(
                r#"tell app "System Events" to get value of attribute "AXMain" of window id {}"#,
                hwnd
            ))
            .unwrap_or_default();
            let fg = check == "true" || check == "1";
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "foreground": fg }))
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop::linux_window::window_is_foreground(hwnd)
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    /// Get current foreground window hwnd (0 = unknown/none)
    /// 用于操作后的前台变化检测（弹窗/跳转识别），不参与输入路由。
    pub async fn foreground_hwnd(&self) -> Result<Value> {
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let fg = unsafe { GetForegroundWindow() };
            Self::result_ok(serde_json::json!({ "hwnd": fg.0 as isize }))
        }
        #[cfg(target_os = "macos")]
        {
            // macOS: osascript 查询当前最前窗口
            let script = r#"tell app "System Events" to get id of first window of (first process whose frontmost is true)"#;
            match Self::osascript(script) {
                Ok(id) => {
                    if let Ok(hwnd) = id.trim().parse::<i64>() {
                        return Self::result_ok(serde_json::json!({ "hwnd": hwnd as isize }));
                    }
                }
                Err(e) => {
                    tracing::warn!("[desktop] macOS foreground_hwnd 查询失败: {e}");
                }
            }
            Self::result_ok(serde_json::json!({
                "hwnd": 0,
                "note": "macOS 前台窗口查询未成功，已返回 0 降级。请确认 系统设置→隐私与安全性→辅助功能 中已授权 Nuphus。"
            }))
        }
        #[cfg(target_os = "linux")]
        {
            // Linux X11: 通过 _NET_ACTIVE_WINDOW 获取当前活动窗口
            match crate::desktop::linux_window::foreground_hwnd() {
                Ok(val) => {
                    if let Some(hwnd) = val
                        .get("result")
                        .and_then(|r| r.get("hwnd"))
                        .and_then(|v| v.as_i64())
                    {
                        return Self::result_ok(serde_json::json!({ "hwnd": hwnd }));
                    }
                }
                Err(e) => {
                    tracing::warn!("[desktop] Linux foreground_hwnd 查询失败: {e}");
                }
            }
            Self::result_ok(serde_json::json!({
                "hwnd": 0,
                "note": "Linux 前台窗口查询未成功，已返回 0 降级。如使用 Wayland 桌面环境，窗口管理功能不可用，请切换到 X11 会话。"
            }))
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    /// Clear clipboard — wipe residual content (passwords/tokens) after paste
    pub async fn clipboard_clean(&self) -> Result<Value> {
        desk_clip::write_text("")?;
        Self::result_ok(serde_json::json!({ "cleared": true }))
    }

    /// Write clipboard
    pub async fn clipboard_write(&self, text: &str) -> Result<Value> {
        desk_clip::write_text(text)?;
        Self::result_ok(serde_json::json!({ "chars": text.len() }))
    }

    // ══════════════════════════════════════════════
    //  OCR — PaddleOCR (ONNX) / Vision (Chat API)
    // ══════════════════════════════════════════════

    /// OCR recognition via engine: "paddle" (PP-OCRv4 ONNX) or "vision" (Chat API)
    /// When boxes=true, returns bounding boxes for each text block.
    pub async fn ocr(
        &self,
        engine: &str,
        image_path: &str,
        boxes: bool,
        prompt: Option<&str>,
    ) -> Result<Value> {
        match engine {
            "vision" => {
                let path = image_path.to_string();
                let prompt = prompt.map(|s| s.to_string());
                let result = tokio::time::timeout(
                    // vision 走远端模型：推理系主模型（如 k3）单次可达 50s+，30s 会误杀
                    std::time::Duration::from_secs(120),
                    tokio::task::spawn_blocking(move || {
                        super::vision_ocr::vision_ocr(&path, prompt.as_deref())
                            .map_err(crate::NuphusError::Tool)
                    }),
                )
                .await
                .map_err(|_| crate::NuphusError::Tool("视觉模型超时（120秒）".to_string()))?
                .map_err(|e| crate::NuphusError::Tool(format!("视觉模型线程异常: {e}")))?;

                match result {
                    Ok(text) => {
                        let resp = serde_json::json!({ "text": text, "engine": "vision" });
                        Self::result_ok(resp)
                    }
                    Err(e) => Self::result_err(format!("视觉模型调用失败: {e}")),
                }
            }
            _ => {
                // "paddle" (default)
                let path = image_path.to_string();
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::task::spawn_blocking(move || -> std::result::Result<(String, Option<Vec<serde_json::Value>>), String> {
                        let mut engine = super::paddle_ocr::PaddleOcr::new()
                            .map_err(|e| e.to_string())?;
                        if boxes {
                            let blocks = engine.ocr_with_boxes(&path)
                                .map_err(|e| e.to_string())?;
                            let text: String = blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
                            let blocks_json: Vec<serde_json::Value> = blocks.iter().map(|b| serde_json::json!({
                                "text": b.text, "x": b.x, "y": b.y, "w": b.w, "h": b.h
                            })).collect();
                            Ok((text, Some(blocks_json)))
                        } else {
                            let text = engine.ocr(&path)
                                .map_err(|e| e.to_string())?;
                            Ok((text, None))
                        }
                    })
                ).await
                .map_err(|_| crate::NuphusError::Tool("OCR 超时（30秒）".to_string()))?
                .map_err(|e| crate::NuphusError::Tool(format!("OCR 线程崩溃: {e}")))?;

                match result {
                    Ok((text, blocks)) => {
                        let mut resp = serde_json::json!({ "text": text, "engine": "paddle" });
                        if let Some(b) = blocks {
                            resp["blocks"] = serde_json::json!(b);
                        }
                        Self::result_ok(resp)
                    }
                    Err(e) => Self::result_err(e.to_string()),
                }
            }
        }
    }

    /// UI 感知 — OCR + YOLO 并行检测 → 合并结果
    ///
    /// 加载截图文件，并行执行 PaddleOCR（文字检测）和 YOLO（元素检测），
    /// 通过 ui_perception::merge() 合并去重后返回统一 JSON。
    pub async fn perceive(&self, image_path: &str) -> Result<Value> {
        let img_path_ocr = image_path.to_string();
        let img_path_yolo = image_path.to_string();
        let yolo = self.yolo.clone();

        // 并行执行 OCR 和 YOLO
        let (ocr_result, yolo_result) = tokio::join!(
            tokio::task::spawn_blocking(
                move || -> crate::Result<Vec<crate::desktop::paddle_ocr::OcrBlock>> {
                    let mut engine = crate::desktop::paddle_ocr::PaddleOcr::new().map_err(|e| {
                        crate::NuphusError::Tool(format!("PaddleOCR 初始化失败: {e}"))
                    })?;
                    engine
                        .ocr_with_boxes(&img_path_ocr)
                        .map_err(|e| crate::NuphusError::Tool(format!("PaddleOCR 失败: {e}")))
                }
            ),
            tokio::task::spawn_blocking(
                move || -> crate::Result<Vec<desktop_api::vision::Element>> {
                    // 加载图片 → Frame（RGBA）
                    let img = image::open(&img_path_yolo).map_err(|e| {
                        crate::NuphusError::Tool(format!("打开图片失败 {img_path_yolo}: {e}"))
                    })?;
                    let rgba = img.to_rgba8();
                    let (w, h) = (rgba.width(), rgba.height());
                    let frame = Frame {
                        id: uuid::Uuid::new_v4(),
                        pixels: rgba.into_raw(),
                        width: w,
                        height: h,
                        scope: Scope::Fullscreen,
                        timestamp: chrono::Utc::now(),
                        source: FrameSource::Screenshot,
                    };
                    yolo.detect(&frame)
                        .map_err(|e| crate::NuphusError::Tool(format!("YOLO 检测失败: {e}")))
                }
            ),
        );

        let ocr_blocks =
            ocr_result.map_err(|e| crate::NuphusError::Tool(format!("OCR 线程崩溃: {e}")))??;
        let yolo_elements =
            yolo_result.map_err(|e| crate::NuphusError::Tool(format!("YOLO 线程崩溃: {e}")))??;

        // 合并
        let elements = crate::desktop::ui_perception::merge(&ocr_blocks, &yolo_elements);

        let json_elements: Vec<Value> = elements
            .iter()
            .map(|el| {
                let center = el.rect.center();
                serde_json::json!({
                    "id": el.id,
                    "kind": format!("{:?}", el.kind).to_lowercase(),
                    "text": el.text,
                    "rect": { "x": el.rect.x, "y": el.rect.y, "w": el.rect.w, "h": el.rect.h },
                    "center": { "x": center.x, "y": center.y },
                    "confidence": el.confidence,
                    "source": format!("{:?}", el.source).to_lowercase(),
                })
            })
            .collect();

        Self::result_ok(serde_json::json!({
            "elements": json_elements,
            "count": json_elements.len(),
            "ocr_count": ocr_blocks.len(),
            "yolo_count": yolo_elements.len(),
        }))
    }

    pub async fn window_move(&self, hwnd: i32, x: i32, y: i32) -> Result<Value> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, HWND_TOP, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
            };
            unsafe {
                _ = SetWindowPos(
                    HWND(hwnd as isize),
                    HWND_TOP,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW,
                );
            }
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "x": x, "y": y }))
        }
        #[cfg(target_os = "macos")]
        {
            Self::osascript(&format!(
                r#"tell app "System Events" to set position of window id {} to {{{}, {}}}"#,
                hwnd, x, y
            ))?;
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "x": x, "y": y }))
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop::linux_window::window_move(hwnd, x, y)
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    /// Resize window
    pub async fn window_resize(&self, hwnd: i32, width: i32, height: i32) -> Result<Value> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                SetWindowPos, HWND_TOP, SWP_NOMOVE, SWP_NOZORDER, SWP_SHOWWINDOW,
            };
            unsafe {
                _ = SetWindowPos(
                    HWND(hwnd as isize),
                    HWND_TOP,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_SHOWWINDOW,
                );
            }
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "width": width, "height": height }))
        }
        #[cfg(target_os = "macos")]
        {
            Self::osascript(&format!(
                r#"tell app "System Events" to set size of window id {} to {{{}, {}}}"#,
                hwnd, width, height
            ))?;
            Self::result_ok(serde_json::json!({ "hwnd": hwnd, "width": width, "height": height }))
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop::linux_window::window_resize(hwnd, width, height)
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    /// Get window details
    pub async fn window_info(&self, hwnd: i32) -> Result<Value> {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::{HWND, POINT, RECT};
            use windows::Win32::Graphics::Gdi::ClientToScreen;
            use windows::Win32::UI::WindowsAndMessaging::{
                GetClassNameW, GetClientRect, GetWindowRect, GetWindowTextW,
                GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed,
            };
            let hwnd_ptr = HWND(hwnd as isize);

            // 标题
            let mut buf = [0u16; 512];
            let len = unsafe { GetWindowTextW(hwnd_ptr, &mut buf) };
            let title = String::from_utf16_lossy(&buf[..len as usize]);

            // 可见性 & 状态
            let visible = unsafe { IsWindowVisible(hwnd_ptr).as_bool() };
            let minimized = unsafe { IsIconic(hwnd_ptr).as_bool() };
            let maximized = unsafe { IsZoomed(hwnd_ptr).as_bool() };

            // 窗口坐标
            let mut rect = RECT::default();
            unsafe {
                _ = GetWindowRect(hwnd_ptr, &mut rect);
            }
            let mut client = RECT::default();
            unsafe {
                _ = GetClientRect(hwnd_ptr, &mut client);
            }
            let mut client_origin = POINT { x: 0, y: 0 };
            unsafe {
                _ = ClientToScreen(hwnd_ptr, &mut client_origin);
            }

            // 进程 ID
            let mut pid: u32 = 0;
            unsafe {
                _ = GetWindowThreadProcessId(hwnd_ptr, Some(&mut pid));
            }

            // 进程名
            let process_name = Self::process_name_from_pid(pid).unwrap_or_default();

            // 窗口类名
            let mut class_buf = [0u16; 256];
            let class_len = unsafe { GetClassNameW(hwnd_ptr, &mut class_buf) };
            let class_name = if class_len > 0 {
                String::from_utf16_lossy(&class_buf[..class_len as usize])
            } else {
                String::new()
            };

            // 窗口类型分类
            let window_type = Self::classify_window(&process_name, &class_name, &title);

            // cloaked 检测（UWP 幽灵窗口）
            let cloaked = Self::is_window_cloaked(hwnd);

            Self::result_ok(serde_json::json!({
                "hwnd": hwnd, "title": title,
                "visible": visible, "minimized": minimized, "maximized": maximized,
                "window": { "x": rect.left, "y": rect.top, "width": rect.right - rect.left, "height": rect.bottom - rect.top },
                "client": { "x": client_origin.x, "y": client_origin.y, "width": client.right - client.left, "height": client.bottom - client.top },
                "process_id": pid,
                "process_name": process_name,
                "class_name": class_name,
                "window_type": window_type,
                "cloaked": cloaked,
            }))
        }
        #[cfg(target_os = "macos")]
        {
            // 单次 osascript 获取全部窗口信息
            let script = format!(
                r#"tell app "System Events"
    try
        set w to window id {}
        set ttl to title of w
        set {{px, py}} to position of w
        set {{pw, ph}} to size of w
        -- 检查最小化/隐藏状态 (AXMinimized, role 非空 = visible)
        set isMin to false
        try
            set attrs to attributes of w
            repeat with a in attrs
                if name of a is "AXMinimized" then
                    if value of a is true then set isMin to true
                    exit repeat
                end if
            end repeat
        end try
        return ttl & "|||" & px & "|||" & py & "|||" & pw & "|||" & ph & "|||" & isMin
    on error
        return ""
    end try
end tell"#,
                hwnd
            );
            let output = Self::osascript(&script)?;
            if output.is_empty() {
                return Ok(serde_json::json!({
                    "hwnd": hwnd, "title": "",
                    "visible": false, "minimized": false, "maximized": false,
                    "window": { "x": 0, "y": 0, "width": 0, "height": 0 },
                }));
            }
            let parts: Vec<&str> = output.split("|||").collect();
            let (title, x, y, w, h, minimized) = if parts.len() >= 6 {
                (
                    parts[0].to_string(),
                    parts[1].trim().parse::<i32>().unwrap_or(0),
                    parts[2].trim().parse::<i32>().unwrap_or(0),
                    parts[3].trim().parse::<i32>().unwrap_or(0),
                    parts[4].trim().parse::<i32>().unwrap_or(0),
                    parts[5].trim() == "true",
                )
            } else {
                (String::new(), 0, 0, 0, 0, false)
            };
            Self::result_ok(serde_json::json!({
                "hwnd": hwnd, "title": title,
                "visible": true,
                "minimized": minimized,
                "maximized": false,
                "window": { "x": x, "y": y, "width": w, "height": h },
                "note": "maximized detection limited on macOS"
            }))
        }
        #[cfg(target_os = "linux")]
        {
            crate::desktop::linux_window::window_info(hwnd)
        }
        #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
        {
            Err(DesktopError::PlatformNotSupported.into())
        }
    }

    // ══════════════════════════════════════════════
    //  Vision / Locate — Rust native implementation
    // ══════════════════════════════════════════════

    /// Find image — search for template image on screen
    pub async fn find_image(
        &self,
        template_path: &str,
        region: Option<Value>,
        threshold: Option<f64>,
    ) -> Result<Value> {
        // Extract region offset (for converting crop coordinates back to screen coordinates)
        let (region_x, region_y) = region.as_ref().map_or((0i32, 0i32), |r| {
            (
                r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            )
        });

        // Determine screenshot scope
        let scope = match &region {
            Some(r) => {
                let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let w = r.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let h = r.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                Scope::Element { x, y, w, h }
            }
            None => Scope::Fullscreen,
        };

        // Screenshot directly to Frame (no temporary BMP file)
        let dummy_target = Target::Tui {
            hwnd: 0,
            title: String::new(),
        };
        let frame = capture::capture(&dummy_target, scope).await?;

        // Handle multiple templates (separated by |)
        let templates: Vec<&str> = template_path
            .split('|')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if templates.is_empty() {
            return Ok(serde_json::json!({"found": false, "x": 0, "y": 0, "confidence": 0.0}));
        }

        // 使用 desktop-api Locator（粗扫→精扫 MAD 匹配）
        let locate = Locator::new();
        let min_confidence = threshold.unwrap_or(0.9) as f32;

        // 记录所有模板中最接近的候选（即使未达 threshold，用于失败诊断）
        let mut diag: Option<(FindResult, String)> = None;
        // 记录第一个加载失败的模板（文件不存在/不可读/解码失败，用于诊断）
        let mut load_error: Option<String> = None;

        for tpl in &templates {
            let name = std::path::Path::new(tpl)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            let template_bytes = match std::fs::read(tpl) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("[find_image] 模板文件读取失败 {}: {}", tpl, e);
                    if load_error.is_none() {
                        load_error = Some(name);
                    }
                    continue;
                }
            };

            let query = Query::Image(template_bytes);
            match locate.find_with_fallback(&frame, &query).await {
                Ok(result) if result.found && result.confidence >= min_confidence => {
                    if let Some(mut rect) = result.rect {
                        // Restore to screen coordinates
                        rect.x += region_x;
                        rect.y += region_y;
                        return Ok(serde_json::json!({
                            "found": true,
                            "x": rect.x,
                            "y": rect.y,
                            "w": rect.w,
                            "h": rect.h,
                            "confidence": (result.confidence * 10000.0).round() / 10000.0,
                            "template": name,
                        }));
                    }
                }
                Ok(result) => {
                    // 未达 threshold：记录最接近候选，供诊断
                    match &diag {
                        None => diag = Some((result, name)),
                        Some((best, _)) if result.confidence > best.confidence => {
                            diag = Some((result, name))
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    // 模板解码失败等：记录诊断，继续尝试后续模板
                    tracing::warn!("[find_image] 模板匹配异常 {}: {}", tpl, e);
                    if load_error.is_none() {
                        load_error = Some(name);
                    }
                    continue;
                }
            }
        }

        // 全部模板均未达 threshold：返回最接近候选位置与置信度（found=false）
        if let Some((r, name)) = diag {
            if let Some(mut rect) = r.rect {
                rect.x += region_x;
                rect.y += region_y;
                return Ok(serde_json::json!({
                    "found": false,
                    "x": rect.x,
                    "y": rect.y,
                    "w": rect.w,
                    "h": rect.h,
                    "confidence": (r.confidence * 10000.0).round() / 10000.0,
                    "template": name,
                    "diagnostic": "not found: closest candidate below threshold",
                }));
            }
        }

        // 模板文件存在但无法加载/解码（如文件损坏、格式不支持）：给出明确诊断
        if let Some(name) = load_error {
            return Ok(serde_json::json!({
                "found": false,
                "x": 0, "y": 0, "w": 0, "h": 0,
                "confidence": 0.0,
                "template": name,
                "diagnostic": "template file not found or unreadable",
            }));
        }

        Ok(serde_json::json!({"found": false, "x": 0, "y": 0, "confidence": 0.0}))
    }

    /// Find color — search for specific color on screen
    pub async fn find_color(
        &self,
        color: &str,
        region: Option<Value>,
        direction: Option<&str>,
    ) -> Result<Value> {
        let captures_dir = Self::captures_dir()?;
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
        let screenshot_path = captures_dir.join(format!("find_color_{}.bmp", timestamp));
        let screenshot_path_str = screenshot_path.display().to_string();

        let (region_x, region_y, region_w, region_h) = match region {
            Some(r) => {
                let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let w = r.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let h = r.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                (x, y, w, h)
            }
            None => (0, 0, 0, 0),
        };

        let direction_val = direction.unwrap_or("left_top");

        self.screenshot(Some(&screenshot_path_str), None).await?;

        super::vision::find_color(
            &screenshot_path_str,
            color,
            region_x,
            region_y,
            region_w,
            region_h,
            direction_val,
        )
    }

    /// Find multi-color — find color pattern
    pub async fn find_multi_color(
        &self,
        anchor: &str,
        offsets: &str,
        region: Option<Value>,
        min_match_ratio: Option<f64>,
        direction: Option<&str>,
    ) -> Result<Value> {
        let captures_dir = Self::captures_dir()?;
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
        let screenshot_path = captures_dir.join(format!("find_multi_color_{}.bmp", timestamp));
        let screenshot_path_str = screenshot_path.display().to_string();

        let (region_x, region_y, region_w, region_h) = match region {
            Some(r) => {
                let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let w = r.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let h = r.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                (x, y, w, h)
            }
            None => (0, 0, 0, 0),
        };

        let ratio = min_match_ratio.unwrap_or(0.9);
        let direction_val = direction.unwrap_or("left_top");

        self.screenshot(Some(&screenshot_path_str), None).await?;

        super::vision::find_multi_color(
            &screenshot_path_str,
            anchor,
            offsets,
            ratio,
            region_x,
            region_y,
            region_w,
            region_h,
            direction_val,
        )
    }

    /// Find text — dictionary-based sliding window text search
    ///
    /// Supports multi-word combination search, separated by `|`, e.g. "系统|文件|统文"
    pub async fn find_text(
        &self,
        dict_name: &str,
        words: &str,
        region: Option<Value>,
        sim_threshold: Option<f32>,
    ) -> Result<Value> {
        let captures_dir = Self::captures_dir()?;
        std::fs::create_dir_all(&captures_dir)
            .map_err(|e| crate::NuphusError::Tool(format!("创建截图目录失败: {e}")))?;
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
        let screenshot_path = captures_dir.join(format!("find_text_{}.bmp", timestamp));
        let screenshot_path_str = screenshot_path.display().to_string();

        let (region_x, region_y, _region_w, _region_h) = match region.clone() {
            Some(r) => {
                let x = r.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = r.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let w = r.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let h = r.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                (x, y, w, h)
            }
            None => (0, 0, 0, 0),
        };

        // Screenshot
        self.screenshot(Some(&screenshot_path_str), region.clone())
            .await?;

        // Load screenshot
        let (sw, sh, pixels) = super::vision::load_bmp(&screenshot_path_str)
            .map_err(|e| crate::NuphusError::Tool(format!("读取截图失败: {e}")))?;

        // Auto-detect foreground color
        use crate::desktop::dict_ocr;
        let analysis = dict_ocr::analyze_region(&pixels, sw, sh);
        let fg = dict_ocr::ColorSpec::new(
            analysis.foreground.r,
            analysis.foreground.g,
            analysis.foreground.b,
            analysis.foreground.dr,
            analysis.foreground.dg,
            analysis.foreground.db,
        );

        // Load dictionary
        let dict_dir = Self::dict_dir();
        let dict_path = dict_dir.join(format!("{}.dict", dict_name));
        if !dict_path.exists() {
            return Err(crate::NuphusError::Tool(format!(
                "字库 '{dict_name}' 不存在。请先在字库管理中创建/加载该字库。"
            )));
        }
        let store = dict_ocr::store::DictStore::load(&dict_path)
            .map_err(|e| crate::NuphusError::Tool(format!("加载字库失败: {e}")))?;

        let all_templates: Vec<dict_ocr::CharTemplate> =
            store.all().values().flat_map(|v| v.clone()).collect();
        if all_templates.is_empty() {
            return Self::result_ok(serde_json::json!({
                "found": false, "text": "", "matches": []
            }));
        }

        let min_sim = sim_threshold.unwrap_or(1.0);

        // Determine target word list to search for
        let word_list: Vec<&str> = words
            .split('|')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        // Collect all individual characters involved in target words
        let target_chars: std::collections::HashSet<char> =
            word_list.iter().flat_map(|w| w.chars()).collect();

        // Only search character templates used in target words (performance optimization)
        let templates_to_search: Vec<&dict_ocr::CharTemplate> = if target_chars.is_empty() {
            all_templates.iter().collect()
        } else {
            all_templates
                .iter()
                .filter(|t| {
                    let tc: Vec<char> = t.char.chars().collect();
                    tc.len() == 1 && target_chars.contains(&tc[0])
                })
                .collect()
        };

        if templates_to_search.is_empty() {
            return Self::result_ok(serde_json::json!({
                "found": false, "text": "", "matches": []
            }));
        }

        // Sliding window match
        struct Match {
            x: i32,
            w: u32,
            sim: f32,
            ch: String,
        }
        let mut hits = Vec::new();

        for tmpl in &templates_to_search {
            let results = dict_ocr::search_screen(&pixels, sw, sh, tmpl, &fg, min_sim);
            for r in results {
                hits.push(Match {
                    x: r.x,
                    w: r.width,
                    sim: r.confidence,
                    ch: tmpl.char.clone(),
                });
            }
        }

        // Dedup (overlapping positions keep highest similarity)
        hits.sort_by(|a, b| {
            b.sim
                .partial_cmp(&a.sim)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut deduped: Vec<Match> = Vec::new();
        for h in &hits {
            let overlap = deduped
                .iter()
                .any(|d| (h.x - d.x).unsigned_abs() < (h.w + d.w) / 4);
            if !overlap {
                deduped.push(Match {
                    x: h.x,
                    w: h.w,
                    sim: h.sim,
                    ch: h.ch.clone(),
                });
            }
        }
        deduped.sort_by_key(|a| a.x);

        let full_text: String = deduped.iter().map(|m| m.ch.as_str()).collect();

        // Max x interval (char width x 2 as reasonable upper bound)
        let max_tmpl_w = all_templates
            .iter()
            .map(|t| t.width as u32)
            .max()
            .unwrap_or(20);
        let max_x_interval = (max_tmpl_w * 2) as i32;

        // Scan matches for each target word
        let mut all_matches = Vec::new();
        for word in &word_list {
            let target_chars: Vec<char> = word.chars().collect();
            if target_chars.is_empty() {
                continue;
            }

            let mut i = 0;
            while i + target_chars.len() <= deduped.len() {
                let mut ok = true;
                for (k, tc) in target_chars.iter().enumerate() {
                    let fc: Vec<char> = deduped[i + k].ch.chars().collect();
                    if fc.len() != 1 || fc[0] != *tc {
                        ok = false;
                        break;
                    }
                    if k > 0 {
                        let interval = deduped[i + k].x - deduped[i + k - 1].x;
                        if interval <= 0 || interval > max_x_interval {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    let slice = &deduped[i..i + target_chars.len()];
                    let min_x = slice.iter().map(|m| m.x).min().unwrap_or(0);
                    let max_x = slice.iter().map(|m| m.x + m.w as i32).max().unwrap_or(0);
                    let avg_sim = slice.iter().map(|m| m.sim).sum::<f32>() / slice.len() as f32;
                    all_matches.push(serde_json::json!({
                        "word": word,
                        "x": min_x + region_x,
                        "y": region_y,
                        "width": (max_x - min_x) as u32,
                        "confidence": (avg_sim * 10000.0).round() / 10000.0,
                    }));
                    i += target_chars.len();
                } else {
                    i += 1;
                }
            }
        }

        let found = !all_matches.is_empty();
        let _ = std::fs::remove_file(&screenshot_path);
        let hint = if !found {
            Some("未在屏幕区域中找到目标文字。原因可能：1) 字库中不含需要的字符（请先用桌面取字添加到字库）；2) 文字未显示在截取区域内；3) 颜色或背景干扰导致匹配失败。")
        } else {
            None
        };
        Self::result_ok(serde_json::json!({
            "found": found,
            "text": full_text,
            "matches": all_matches,
            "match_count": all_matches.len(),
            "fallback_hint": hint,
        }))
    }

    /// Get dictionary directory
    fn dict_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("NUPHUS_DICT_DIR") {
            return PathBuf::from(dir);
        }
        if let Ok(dir) = std::env::var("NUPHUS_DATA_DIR") {
            return PathBuf::from(dir).join("dicts");
        }
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Nuphus")
            .join("dicts")
    }
}

/// Get unified screenshot directory
///
/// Priority: NUPHUS_CAPTURES_DIR env var > system temp dir /nuphus/captures
///
/// 截图属于临时文件，用完即删。默认存到系统 temp 目录，OS 会定期清理。
/// 如需持久保留，通过 workflow screenshots 目录或设置 NUPHUS_CAPTURES_DIR。
pub fn captures_dir_path() -> std::io::Result<PathBuf> {
    if let Ok(dir) = std::env::var("NUPHUS_CAPTURES_DIR") {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    Ok(std::env::temp_dir().join("nuphus").join("captures"))
}