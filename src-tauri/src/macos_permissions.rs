//! macOS privacy permission checks used by the workflow experience.
//!
//! TCC does not expose one universal "all files" or "all automation" check.
//! Those capabilities are therefore reported as on-demand and are never used
//! to block workflow mode. Accessibility is required for desktop control;
//! screen recording is only required when capturing or inspecting screenshots. Microphone status
//! is read passively through AVFoundation and only serves chat-side voice input,
//! so it never gates workflow mode.

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

#[cfg(target_os = "macos")]
const SCREEN_RECORDING_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
#[cfg(target_os = "macos")]
const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
#[cfg(target_os = "macos")]
const MICROPHONE_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
#[cfg(target_os = "macos")]
const FILES_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";
#[cfg(target_os = "macos")]
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
fn microphone_status_granted(status: objc2_av_foundation::AVAuthorizationStatus) -> bool {
    status == objc2_av_foundation::AVAuthorizationStatus::Authorized
}

#[cfg(target_os = "macos")]
fn microphone_permission_granted() -> bool {
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};

    // This only reads TCC's current status. It does not open an input device,
    // create a cpal stream, or trigger microphone activity in the menu bar.
    let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
        tracing::warn!("AVMediaTypeAudio is unavailable");
        return false;
    };
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
    microphone_status_granted(status)
}

/// Read Accessibility trust without prompting or opening a device.
#[cfg(target_os = "macos")]
pub(crate) fn accessibility_permission_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

#[cfg(target_os = "macos")]
fn report() -> MacosPermissionReport {
    let screen_recording = unsafe { CGPreflightScreenCaptureAccess() };
    let accessibility = accessibility_permission_granted();
    let microphone = microphone_permission_granted();

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
                required_for_workflow: false,
                title: "录屏",
                description: "桌面截图和视觉识别需要此权限；仅使用辅助功能控件的工作流无需授权。",
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
                // 麦克风只服务聊天侧语音输入，不属于工作流执行的前置条件：
                // src/workflow/** 无 speech/stt 引用，src/tools/** 也无语音类工具。
                // 标为必需会让未使用语音的用户每次进入 workflow 都收到权限提示。
                required_for_workflow: false,
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

/// Return the current macOS privacy permission state without touching devices.
#[tauri::command]
pub async fn get_macos_permission_status() -> Result<MacosPermissionReport, String> {
    tauri::async_runtime::spawn_blocking(report)
        .await
        .map_err(|error| format!("macOS permission status task failed: {error}"))
}

/// Request a concrete permission where macOS exposes a request API.
#[tauri::command]
pub async fn request_macos_permission(id: String) -> Result<MacosPermissionReport, String> {
    tauri::async_runtime::spawn_blocking(move || request_macos_permission_blocking(id))
        .await
        .map_err(|error| format!("macOS permission request task failed: {error}"))?
}

fn request_macos_permission_blocking(id: String) -> Result<MacosPermissionReport, String> {
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
            // A permission status check must not start an audio stream. The
            // settings page is the explicit user flow for microphone access.
            "microphone" => {}
            "files_and_folders" | "automation" => {}
            _ => return Err(format!("未知的 macOS 权限: {id}")),
        }
        open_macos_permission_settings_blocking(&id)?;
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
pub async fn open_macos_permission_settings(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || open_macos_permission_settings_blocking(&id))
        .await
        .map_err(|error| format!("打开 macOS 系统设置任务失败: {error}"))?
}

fn open_macos_permission_settings_blocking(id: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let url = match id {
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

    #[cfg(target_os = "macos")]
    #[test]
    fn microphone_does_not_gate_workflow_mode() {
        let value = report();
        let microphone = value
            .permissions
            .iter()
            .find(|permission| permission.id == "microphone")
            .expect("microphone permission must be present");
        assert!(!microphone.required_for_workflow);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn semantic_workflow_only_requires_accessibility() {
        let value = report();
        let required: Vec<_> = value
            .permissions
            .iter()
            .filter(|permission| permission.required_for_workflow)
            .map(|permission| permission.id.as_str())
            .collect();
        assert_eq!(required, ["accessibility"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_authorized_microphone_status_is_granted() {
        use objc2_av_foundation::AVAuthorizationStatus;

        assert!(microphone_status_granted(AVAuthorizationStatus::Authorized));
        assert!(!microphone_status_granted(
            AVAuthorizationStatus::NotDetermined
        ));
        assert!(!microphone_status_granted(AVAuthorizationStatus::Denied));
        assert!(!microphone_status_granted(
            AVAuthorizationStatus::Restricted
        ));
    }
}
