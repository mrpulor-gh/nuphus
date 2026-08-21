// desktop.rs — Desktop tool independent commands
// Bypasses ToolRegistry / execute_tool / mock middleware
// Directly calls nuphus::desktop::DesktopClient
//
// ⚠ DesktopClient returns { success: true, result: data } wrapper format
// All commands use unwrap_result() to extract clean data for frontend

/// Extract raw data from DesktopClient's { success, result/error } wrapper
fn unwrap_result(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        Ok(value.get("result").cloned().unwrap_or(value))
    } else {
        Err(value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// Mouse current position — returns { x, y }
#[tauri::command]
pub async fn desktop_mouse_position() -> Result<serde_json::Value, String> {
    let client = nuphus::desktop::DesktopClient::new();
    let raw = client.mouse_position().await.map_err(|e| e.to_string())?;
    unwrap_result(raw)
}

/// 写入剪贴板
#[tauri::command]
pub async fn desktop_clipboard_write(text: String) -> Result<serde_json::Value, String> {
    let client = nuphus::desktop::DesktopClient::new();
    let raw = client
        .clipboard_write(&text)
        .await
        .map_err(|e| e.to_string())?;
    unwrap_result(raw)
}
