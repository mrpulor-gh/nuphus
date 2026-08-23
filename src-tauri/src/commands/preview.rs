// preview.rs — 文件预览命令（桌面端 AI 回复路径点击预览）
// read_file：读取文本内容（≤2MB）供前端内联渲染
// open_path：系统默认程序打开
// reveal_path：文件管理器定位

use std::path::Path;

/// 读取文件文本内容（≤2MB），供前端预览覆盖层渲染。
/// 超过 2MB 或读取失败时返回错误信息，前端提示改用「系统打开」。
#[tauri::command]
pub fn read_file(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在：{}", path));
    }
    if !p.is_file() {
        return Err(format!("不是文件（可能是文件夹）：{}", path));
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("读取文件信息失败：{}", e))?;
    if meta.len() > 2 * 1024 * 1024 {
        return Err("文件超过 2MB，无法内联预览，请用「系统打开」查看".to_string());
    }
    std::fs::read_to_string(p).map_err(|e| format!("读取文件失败：{}", e))
}

/// 读取文件为 base64 字符串（≤8MB），供前端内联预览图片等二进制内容。
/// 超过 8MB 或读取失败时返回错误信息，前端提示改用「系统打开」。
#[tauri::command]
pub fn read_file_base64(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在：{}", path));
    }
    if !p.is_file() {
        return Err(format!("不是文件（可能是文件夹）：{}", path));
    }
    let meta = std::fs::metadata(p).map_err(|e| format!("读取文件信息失败：{}", e))?;
    if meta.len() > 8 * 1024 * 1024 {
        return Err("文件超过 8MB，无法内联预览，请用「系统打开」查看".to_string());
    }
    let bytes = std::fs::read(p).map_err(|e| format!("读取文件失败：{}", e))?;
    use base64::Engine as _;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 用系统默认程序打开文件/文件夹。
/// 先做存在性检查：路径不存在时直接报错（而非让系统弹框/静默），
/// 避免「点了打不开」无反馈的体验。
#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("路径不存在，无法打开：{}", path));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &path])
            .spawn()
            .map_err(|e| format!("系统打开失败：{}", e))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("系统打开失败：{}", e))?;
    }
    Ok(())
}

/// 在文件管理器中定位文件。
/// Windows 用 explorer /select,<path>（合并为单参数以正确处理含空格路径）。
/// 同样先做存在性检查（explorer 对不存在路径会静默打开默认目录，具误导性）。
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("路径不存在，无法定位：{}", path));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path))
            .spawn()
            .map_err(|e| format!("在文件夹中显示失败：{}", e))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(dir) = Path::new(&path).parent() {
            std::process::Command::new("xdg-open")
                .arg(dir)
                .spawn()
                .map_err(|e| format!("在文件夹中显示失败：{}", e))?;
        }
    }
    Ok(())
}
