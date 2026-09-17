//! macOS privacy permission checks used by the workflow experience.
//!
//! TCC does not expose one universal "all files" or "all automation" check.
//! Those capabilities are therefore reported as on-demand and are never used
//! to block workflow mode. Screen recording, Accessibility, and microphone
//! access have concrete probes and are treated as required workflow permissions.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacosPermissionReport {
    pub platform_supported: bool,
    pub permissions: Vec<MacosPermissionItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacosPermissionItem {
    pub id: String,
    pub status: &'static str,
    pub required_for_workflow: bool,
    pub title: &'static str,
    pub description: &'static str,
    pub settings_url: Option<&'static str>,
}

const SCREEN_RECORDING_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const MICROPHONE_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
const FILES_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";
const AUTOMATION_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation";

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[cfg(target_os = "macos")]
fn microphone_probe() -> bool {
    use crate::speech::mic::MicCapture;
    use std::sync::mpsc;

    // Opening and immediately dropping a stream is the same permission gate
    // used by speech-to-text. It also triggers Apple's microphone prompt when
    // the user has not answered it yet.
    let (tx, _rx) = mpsc::channel();
    MicCapture::start(tx).is_ok()
}

#[cfg(target_os = "macos")]
fn report() -> MacosPermissionReport {
    let screen_recording = unsafe { CGPreflightScreenCaptureAccess() };
    let accessibility = unsafe { AXIsProcessTrusted() };
    let microphone = microphone_probe();

    MacosPermissionReport {
        platform_supported: true,
        permissions: vec![
            MacosPermissionItem {
                id: "screen_recording".to_string(),
                status: if screen_recording {
                    "granted"
                } else {
                    "missing"
                },
                required_for_workflow: true,
                title: "录屏",
                description: "桌面截图、视觉识别和工作流录制需要此权限。",
                settings_url: Some(SCREEN_RECORDING_SETTINGS),
            },
            MacosPermissionItem {
                id: "accessibility".to_string(),
                status: if accessibility { "granted" } else { "missing" },
                required_for_workflow: true,
                title: "辅助功能",
                description: "鼠标键盘控制、窗口操作和桌面自动化需要此权限。",
                settings_url: Some(ACCESSIBILITY_SETTINGS),
            },
            MacosPermissionItem {
                id: "microphone".to_string(),
                status: if microphone { "granted" } else { "missing" },
                required_for_workflow: true,
                title: "麦克风",
                description: "语音输入和语音转文字需要此权限。",
                settings_url: Some(MICROPHONE_SETTINGS),
            },
            MacosPermissionItem {
                id: "files_and_folders".to_string(),
                status: "on_demand",
                required_for_workflow: false,
                title: "文件与文件夹",
                description: "访问桌面、文稿或下载目录时，macOS 会按目录请求授权。",
                settings_url: Some(FILES_SETTINGS),
            },
            MacosPermissionItem {
                id: "automation".to_string(),
                status: "on_demand",
                required_for_workflow: false,
                title: "自动化",
                description: "控制其他应用时，macOS 可能按目标应用单独请求授权。",
                settings_url: Some(AUTOMATION_SETTINGS),
            },
        ],
    }
}

#[cfg(not(target_os = "macos"))]
fn report() -> MacosPermissionReport {
    MacosPermissionReport {
        platform_supported: false,
        permissions: Vec::new(),
    }
}

/// Return the current macOS privacy permission state.
#[tauri::command]
pub fn get_macos_permission_status() -> MacosPermissionReport {
    report()
}

/// Request a concrete permission where macOS exposes a request API.
#[tauri::command]
pub fn request_macos_permission(id: String) -> Result<MacosPermissionReport, String> {
    #[cfg(target_os = "macos")]
    {
        match id.as_str() {
            "screen_recording" => {
                let _ = unsafe { CGRequestScreenCaptureAccess() };
            }
            "accessibility" => {
                // Accessibility has no reliable Rust-level request dialog. The
                // settings page opened below is the supported user flow.
            }
            "microphone" => {
                let _ = microphone_probe();
            }
            "files_and_folders" | "automation" => {}
            _ => return Err(format!("未知的 macOS 权限: {id}")),
        }
        open_macos_permission_settings(id)?;
        return Ok(report());
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        Ok(report())
    }
}

/// Open the relevant macOS Privacy & Security pane.
#[tauri::command]
pub fn open_macos_permission_settings(id: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let url = match id.as_str() {
            "screen_recording" => SCREEN_RECORDING_SETTINGS,
            "accessibility" => ACCESSIBILITY_SETTINGS,
            "microphone" => MICROPHONE_SETTINGS,
            "files_and_folders" => FILES_SETTINGS,
            "automation" => AUTOMATION_SETTINGS,
            _ => return Err(format!("未知的 macOS 权限: {id}")),
        };

        std::process::Command::new("open")
            .arg(url)
            .status()
            .map_err(|e| format!("打开 macOS 系统设置失败: {e}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "macOS 系统设置未能打开".to_string())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        Err("当前平台不支持 macOS 系统权限设置".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_macos_report_is_safe_for_frontend() {
        let value = report();
        #[cfg(not(target_os = "macos"))]
        assert!(!value.platform_supported && value.permissions.is_empty());
        #[cfg(target_os = "macos")]
        assert!(value.platform_supported && value.permissions.len() == 5);
    }

    #[test]
    fn permission_ids_are_unique() {
        let value = report();
        let mut ids = std::collections::HashSet::new();
        for permission in value.permissions {
            assert!(ids.insert(permission.id));
        }
    }
}
