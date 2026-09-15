//! 剪贴板控制 — Win32 原生 / 非 Windows arboard (macOS + Linux)

use crate::core::*;

#[cfg(windows)]
use ::windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
#[cfg(windows)]
use ::windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
#[cfg(windows)]
use ::windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
#[cfg(windows)]
use ::windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

/// 读取 Windows 文件剪贴板（CF_HDROP，格式号 15）。返回绝对路径列表。
/// 剪贴板内容不是「文件复制」（纯文本 / 位图截图 / HTML）时返回空列表。
#[cfg(windows)]
pub fn read_file_paths() -> Result<Vec<String>> {
    use ::windows::Win32::System::DataExchange::GetClipboardData;
    use ::windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        if OpenClipboard(HWND::default()).is_err() {
            return Err(DesktopError::InputFailed(
                "clipboard open failed".to_string(),
            ));
        }
        let result = GetClipboardData(15).and_then(|handle| {
            let hglobal = HGLOBAL(handle.0 as *mut _);
            let hdrop = HDROP(handle.0);
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return Err(::windows::core::Error::from_win32());
            }
            let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let len = DragQueryFileW(hdrop, index, None);
                let mut buf = vec![0u16; len as usize + 1];
                let written = DragQueryFileW(hdrop, index, Some(&mut buf));
                buf.truncate(written as usize);
                paths.push(String::from_utf16_lossy(&buf));
            }
            let _ = GlobalUnlock(hglobal);
            Ok(paths)
        });
        let _ = CloseClipboard();
        result.map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
}

#[cfg(not(windows))]
pub fn read_file_paths() -> Result<Vec<String>> {
    Err(DesktopError::PlatformNotSupported)
}

/// 读取剪贴板文本
pub fn read_text() -> Result<String> {
    #[cfg(windows)]
    {
        unsafe {
            if OpenClipboard(HWND::default()).is_err() {
                return Err(DesktopError::InputFailed(
                    "clipboard open failed".to_string(),
                ));
            }
            let handle = GetClipboardData(13);
            let result = match handle {
                Ok(h) => {
                    let ptr = h.0 as *const u16;
                    let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                    let slice = std::slice::from_raw_parts(ptr, len);
                    Ok(String::from_utf16_lossy(slice))
                }
                Err(_) => Err(DesktopError::InputFailed(
                    "clipboard read failed".to_string(),
                )),
            };
            let _ = CloseClipboard();
            result
        }
    }
    #[cfg(not(windows))]
    {
        arboard::Clipboard::new()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .get_text()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
            .map(|t| t.to_string())
    }
}

/// 写入剪贴板文本
pub fn write_text(text: &str) -> Result<()> {
    #[cfg(windows)]
    {
        unsafe {
            if OpenClipboard(HWND::default()).is_err() {
                return Err(DesktopError::InputFailed(
                    "clipboard open failed".to_string(),
                ));
            }
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let size = (utf16.len() + 1) * 2;
            let h_global = match GlobalAlloc(GHND, size) {
                Ok(h) => h,
                Err(_) => {
                    let _ = CloseClipboard();
                    return Err(DesktopError::InputFailed(
                        "clipboard alloc failed".to_string(),
                    ));
                }
            };
            let dest = GlobalLock(h_global) as *mut u16;
            if dest.is_null() {
                let _ = GlobalFree(h_global);
                let _ = CloseClipboard();
                return Err(DesktopError::InputFailed(
                    "clipboard lock failed".to_string(),
                ));
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), dest, utf16.len());
            *dest.add(utf16.len()) = 0;
            let _ = GlobalUnlock(h_global);
            let _ = EmptyClipboard();
            let _ = SetClipboardData(13, HANDLE(h_global.0 as isize));
            let _ = CloseClipboard();
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        arboard::Clipboard::new()
            .map_err(|e| DesktopError::InputFailed(e.to_string()))?
            .set_text(text.to_owned())
            .map_err(|e| DesktopError::InputFailed(e.to_string()))
    }
}
