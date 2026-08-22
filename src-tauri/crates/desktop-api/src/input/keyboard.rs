//! 键盘控制 — Win32 原生 / macOS enigo / Linux PlatformNotSupported

use crate::core::*;
#[cfg(not(windows))]
use super::SendEnigo;

#[cfg(not(windows))]
fn enigo() -> &'static std::sync::Mutex<SendEnigo> {
    static INST: std::sync::OnceLock<std::sync::Mutex<SendEnigo>> = std::sync::OnceLock::new();
    INST.get_or_init(|| {
        // enigo 0.2: `Enigo::new` 返回 Result（构造可能失败），此处 panic 仅发生在
        // 平台输入初始化不可用（无显示服务器等），与后续调用失败语义一致。
        // SendEnigo: macOS 上 Enigo 非 Send（CGEventSource 指针），经 Mutex 串行化后包装为 Send+Sync。
        std::sync::Mutex::new(SendEnigo(
            enigo::Enigo::new(&enigo::Settings::default()).expect("enigo init failed"),
        ))
    })
}

/// 发送按键
pub async fn press(key: &str) -> Result<()> {
    let vk = key_to_vk(key)?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
        unsafe {
            keybd_event(vk as u8, 0, Default::default(), 0);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        unsafe {
            keybd_event(vk as u8, 0, KEYEVENTF_KEYUP, 0);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use enigo::{Direction, Keyboard};
        let ek = vk_to_enigo(vk)?;
        enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .key(ek, Direction::Click)
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

/// 组合键
pub async fn hotkey(keys: &[&str]) -> Result<()> {
    let vks: Vec<u16> = keys
        .iter()
        .map(|k| key_to_vk(k))
        .collect::<Result<Vec<_>>>()?;
    #[cfg(windows)]
    {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
        for &vk in &vks {
            unsafe {
                keybd_event(vk as u8, 0, Default::default(), 0);
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        for &vk in vks.iter().rev() {
            unsafe {
                keybd_event(vk as u8, 0, KEYEVENTF_KEYUP, 0);
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use enigo::{Direction, Keyboard};
        let mut e = enigo()
            .lock()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        let ekeys: Vec<enigo::Key> = vks
            .iter()
            .map(|&v| vk_to_enigo(v))
            .collect::<Result<Vec<_>>>()?;
        for k in &ekeys {
            e.key(*k, Direction::Press)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        }
        for k in ekeys.iter().rev() {
            e.key(*k, Direction::Release)
                .map_err(|e| DesktopError::InputFailed(e.to_string()))?;
        }
        Ok(())
    }
    #[cfg(all(not(windows), not(any(target_os = "macos", target_os = "linux"))))]
    {
        Err(DesktopError::PlatformNotSupported)
    }
}

fn key_to_vk(key: &str) -> Result<u16> {
    match key.to_lowercase().as_str() {
        "enter" | "return" => Ok(0x0D),
        "tab" => Ok(0x09),
        "esc" | "escape" => Ok(0x1B),
        "space" => Ok(0x20),
        "backspace" => Ok(0x08),
        "delete" | "del" => Ok(0x2E),
        "up" => Ok(0x26),
        "down" => Ok(0x28),
        "left" => Ok(0x25),
        "right" => Ok(0x27),
        "home" => Ok(0x24),
        "end" => Ok(0x23),
        "pageup" => Ok(0x21),
        "pagedown" => Ok(0x22),
        "ctrl" | "control" => Ok(0x11),
        "shift" => Ok(0x10),
        "alt" => Ok(0x12),
        "win" | "command" | "meta" => Ok(0x5B),
        "f1" => Ok(0x70),
        "f2" => Ok(0x71),
        "f3" => Ok(0x72),
        "f4" => Ok(0x73),
        "f5" => Ok(0x74),
        "f6" => Ok(0x75),
        "f7" => Ok(0x76),
        "f8" => Ok(0x77),
        "f9" => Ok(0x78),
        "f10" => Ok(0x79),
        "f11" => Ok(0x7A),
        "f12" => Ok(0x7B),
        "a" => Ok(0x41),
        "b" => Ok(0x42),
        "c" => Ok(0x43),
        "d" => Ok(0x44),
        "e" => Ok(0x45),
        "f" => Ok(0x46),
        "g" => Ok(0x47),
        "h" => Ok(0x48),
        "i" => Ok(0x49),
        "j" => Ok(0x4A),
        "k" => Ok(0x4B),
        "l" => Ok(0x4C),
        "m" => Ok(0x4D),
        "n" => Ok(0x4E),
        "o" => Ok(0x4F),
        "p" => Ok(0x50),
        "q" => Ok(0x51),
        "r" => Ok(0x52),
        "s" => Ok(0x53),
        "t" => Ok(0x54),
        "u" => Ok(0x55),
        "v" => Ok(0x56),
        "w" => Ok(0x57),
        "x" => Ok(0x58),
        "y" => Ok(0x59),
        "z" => Ok(0x5A),
        "0" => Ok(0x30),
        "1" => Ok(0x31),
        "2" => Ok(0x32),
        "3" => Ok(0x33),
        "4" => Ok(0x34),
        "5" => Ok(0x35),
        "6" => Ok(0x36),
        "7" => Ok(0x37),
        "8" => Ok(0x38),
        "9" => Ok(0x39),
        _ => Err(DesktopError::InputFailed(format!("unknown key: {}", key))),
    }
}

#[cfg(not(windows))]
fn vk_to_enigo(vk: u16) -> Result<enigo::Key> {
    use enigo::Key::*;
    Ok(match vk {
        0x0D => Return,
        0x09 => Tab,
        0x1B => Escape,
        0x20 => Space,
        0x08 => Backspace,
        0x2E => Delete,
        0x26 => UpArrow,
        0x28 => DownArrow,
        0x25 => LeftArrow,
        0x27 => RightArrow,
        0x24 => Home,
        0x23 => End,
        0x21 => PageUp,
        0x22 => PageDown,
        0x11 => Control,
        0x10 => Shift,
        0x12 => Alt,
        0x5B => Meta,
        0x70 => F1,
        0x71 => F2,
        0x72 => F3,
        0x73 => F4,
        0x74 => F5,
        0x75 => F6,
        0x76 => F7,
        0x77 => F8,
        0x78 => F9,
        0x79 => F10,
        0x7A => F11,
        0x7B => F12,
        0x41..=0x5A => {
            let c = char::from_u32((vk - 0x41) as u32 + 'a' as u32).unwrap_or('a');
            Unicode(c)
        }
        0x30..=0x39 => {
            let c = char::from_u32((vk - 0x30) as u32 + '0' as u32).unwrap_or('0');
            Unicode(c)
        }
        _ => {
            return Err(DesktopError::InputFailed(format!(
                "unsupported key code: 0x{:X}",
                vk
            )))
        }
    })
}