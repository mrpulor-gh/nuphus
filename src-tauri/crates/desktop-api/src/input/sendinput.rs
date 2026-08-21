//! Nuphus Input — 输入法理念的 Unicode 文本输入
//!
//! 核心设计：把文本输入当作「输入法会话」处理，而非简单按键。
//! 1. 焦点准备：AttachThreadInput 确保跨线程焦点正确转移
//! 2. 编码转换：UTF-8 → UTF-16，正确处理 BMP 外字符（surrogate pairs）
//! 3. 逐字发送：每个码点 KeyDown + KeyUp，KEYEVENTF_UNICODE 绕过键盘布局
//! 4. 会话结束：可选 Enter 提交，带延迟确保应用处理完成

use crate::core::*;

use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_TYPE, KEYBDINPUT, KEYBD_EVENT_FLAGS, VIRTUAL_KEY,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_UNICODE: u32 = 0x0004;
const KEYEVENTF_KEYUP: u32 = 0x0002;

/// 输入法会话配置
#[derive(Debug, Clone)]
pub struct InputSession {
    /// 目标窗口句柄（None = 当前焦点窗口）
    pub target_hwnd: Option<isize>,
    /// 发送后是否按 Enter
    pub press_enter: bool,
    /// 字符间延迟（毫秒）
    pub char_delay_ms: u64,
    /// 发送后额外延迟（毫秒）
    pub post_delay_ms: u64,
    /// 是否强制激活窗口（即使已在前台）
    pub force_activate: bool,
    /// 是否验证目标窗口在foreground（不一致则拒绝发送）
    pub verify_foreground: bool,
}

impl Default for InputSession {
    fn default() -> Self {
        Self {
            target_hwnd: None,
            press_enter: false,
            char_delay_ms: 5,
            post_delay_ms: 50,
            force_activate: false,
            verify_foreground: false,
        }
    }
}

/// Nuphus 统一输入接口 — 输入法理念
///
/// 流程：焦点准备 → 编码转换 → 逐字注入 → 可选提交 → 延迟等待
pub fn nuphus_input(text: &str, session: &InputSession) -> Result<usize> {
    #[cfg(windows)]
    {
        // ── 1. 焦点准备 ──
        let target = prepare_focus(
            session.target_hwnd,
            session.force_activate,
            session.verify_foreground,
        )?;
        if session.verify_foreground && !target.verified {
            return Err(DesktopError::InputFailed(
                "Target window is not in foreground, input rejected".to_string(),
            ));
        }
        // RAII guard：任何退出路径（成功或中途 SendInput 失败）都会 detach
        // 线程输入附属，避免错误路径泄漏 AttachThreadInput 连接。
        let _attach_guard = ThreadInputGuard {
            target_tid: target.attached_hwnd,
        };

        // ── 2. 编码转换：UTF-8 → UTF-16 码点序列 ──
        let codepoints = encode_utf16_codepoints(text);

        // ── 3. 逐字注入 ──
        let mut total_sent = 0;
        for cp in &codepoints {
            let inputs = make_unicode_inputs(*cp);
            let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            if sent == 0 {
                return Err(DesktopError::InputFailed(format!(
                    "SendInput failed for codepoint U+{:04X}",
                    cp
                )));
            }
            total_sent += sent as usize;

            // 字符间延迟
            if session.char_delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(session.char_delay_ms));
            }
        }

        // ── 4. 可选提交（Enter）──
        if session.press_enter {
            let enter_inputs = make_enter_inputs();
            let sent = unsafe { SendInput(&enter_inputs, std::mem::size_of::<INPUT>() as i32) };
            if sent == 0 {
                return Err(DesktopError::InputFailed(
                    "SendInput Enter failed".to_string(),
                ));
            }
            total_sent += sent as usize;
        }

        // ── 5. 发送后延迟，确保应用处理完成 ──
        if session.post_delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(session.post_delay_ms));
        }

        // ── 6. 恢复焦点由 `_attach_guard` 的 Drop 在函数退出时统一处理 ──

        Ok(total_sent)
    }

    #[cfg(not(windows))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 便捷函数：发送到指定窗口
pub fn input_to_window(text: &str, hwnd: isize, press_enter: bool) -> Result<usize> {
    nuphus_input(
        text,
        &InputSession {
            target_hwnd: Some(hwnd),
            press_enter,
            ..Default::default()
        },
    )
}

/// 便捷函数：发送到当前焦点窗口
pub fn input_to_focus(text: &str, press_enter: bool) -> Result<usize> {
    nuphus_input(
        text,
        &InputSession {
            press_enter,
            ..Default::default()
        },
    )
}

// ============================================================================
// 内部实现
// ============================================================================

struct FocusResult {
    attached_hwnd: Option<u32>,
    verified: bool,
}

/// RAII guard：Drop 时 detach 线程输入附属，确保成功、错误、提前返回等
/// 所有退出路径都释放连接（而不只是成功路径）。
struct ThreadInputGuard {
    target_tid: Option<u32>,
}

impl Drop for ThreadInputGuard {
    fn drop(&mut self) {
        if let Some(tid) = self.target_tid {
            unsafe {
                let _ = AttachThreadInput(GetCurrentThreadId(), tid, false);
            }
        }
    }
}

/// 准备窗口焦点，返回需要 detach 的线程 ID
fn prepare_focus(target_hwnd: Option<isize>, force: bool, verify: bool) -> Result<FocusResult> {
    unsafe {
        if let Some(hwnd_val) = target_hwnd {
            let hwnd = HWND(hwnd_val);

            // 如果最小化则还原
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            // 验证模式：检查窗口是否在 foreground，不一致直接返回
            if verify {
                let is_fg = is_foreground_window(hwnd_val);
                return Ok(FocusResult {
                    attached_hwnd: None,
                    verified: is_fg,
                });
            }

            // 跨线程焦点：AttachThreadInput
            let target_tid = GetWindowThreadProcessId(hwnd, None);
            let current_tid = GetCurrentThreadId();
            let mut attached = None;

            if target_tid != current_tid {
                let _ = AttachThreadInput(current_tid, target_tid, true);
                attached = Some(target_tid);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            // 激活窗口
            if force || !is_foreground_window(hwnd_val) {
                let _ = SetForegroundWindow(hwnd);
                std::thread::sleep(std::time::Duration::from_millis(150));
            }

            Ok(FocusResult {
                attached_hwnd: attached,
                verified: true,
            })
        } else {
            // 无目标窗口，使用当前焦点
            Ok(FocusResult {
                attached_hwnd: None,
                verified: true,
            })
        }
    }
}

/// 检查指定窗口是否在前台
unsafe fn is_foreground_window(hwnd: isize) -> bool {
    use ::windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let fg = GetForegroundWindow();
    fg.0 == hwnd
}

/// UTF-8 → UTF-16 码点序列（正确处理 surrogate pairs）
fn encode_utf16_codepoints(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// 为单个 Unicode 码点创建 KeyDown + KeyUp 输入对
fn make_unicode_inputs(ch: u16) -> [INPUT; 2] {
    [
        // KeyDown
        INPUT {
            r#type: INPUT_TYPE(INPUT_KEYBOARD),
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0), // KEYEVENTF_UNICODE 模式下必须为 0
                    wScan: ch,
                    dwFlags: KEYBD_EVENT_FLAGS(KEYEVENTF_UNICODE),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        // KeyUp
        INPUT {
            r#type: INPUT_TYPE(INPUT_KEYBOARD),
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0), // KEYEVENTF_UNICODE 模式下必须为 0
                    wScan: ch,
                    dwFlags: KEYBD_EVENT_FLAGS(KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ]
}

/// 创建 Enter 键的 KeyDown + KeyUp 输入
fn make_enter_inputs() -> [INPUT; 2] {
    const VK_RETURN: u16 = 0x0D;
    [
        // KeyDown
        INPUT {
            r#type: INPUT_TYPE(INPUT_KEYBOARD),
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(VK_RETURN),
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        // KeyUp
        INPUT {
            r#type: INPUT_TYPE(INPUT_KEYBOARD),
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(VK_RETURN),
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(KEYEVENTF_KEYUP),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ]
}

// ============================================================================
// 兼容旧接口（deprecated）
// ============================================================================

/// 发送 Unicode 文本到当前焦点窗口（旧接口，建议用 nuphus_input）
pub fn send_unicode_text(text: &str) -> Result<usize> {
    nuphus_input(text, &InputSession::default())
}

/// 逐字符发送，带间隔（旧接口，建议用 nuphus_input）
pub fn send_unicode_chars(text: &str, interval_ms: u64) -> Result<usize> {
    nuphus_input(
        text,
        &InputSession {
            char_delay_ms: interval_ms,
            ..Default::default()
        },
    )
}
