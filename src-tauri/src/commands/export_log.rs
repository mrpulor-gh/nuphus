use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[tauri::command]
pub fn export_error_log(content: String, app_handle: tauri::AppHandle) -> Result<String, String> {
    // Try desktop first, fall back to Nuphus root directory
    let path = get_desktop_path()
        .or_else(|| app_handle.path().resource_dir().ok().map(|p| p.join("..")))
        .ok_or_else(|| "无法确定保存路径".to_string())?;

    let filename = format!(
        "nuphus-error-{}.log",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let full_path = path.join(&filename);

    fs::write(&full_path, &content).map_err(|e| format!("写入日志文件失败: {}", e))?;

    Ok(full_path.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
fn get_desktop_path() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .ok()
        .map(|p| PathBuf::from(p).join("Desktop"))
}

#[cfg(target_os = "macos")]
fn get_desktop_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|p| PathBuf::from(p).join("Desktop"))
}

#[cfg(target_os = "linux")]
fn get_desktop_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|p| PathBuf::from(p).join("Desktop"))
}
