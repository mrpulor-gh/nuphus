//! 通用本地文件操作 —— 目前只有「另存为」。
//!
//! 前端（引用栏/文件链接的右键菜单）用 `plugin-dialog` 的 `save()` 选定目标路径，
//! 再调用本命令做同源复制。覆盖确认由保存对话框负责，这里只做存在性检查与可读错误。
//! 为什么不用 `plugin-fs`：项目未安装该插件，一条 `std::fs::copy` 比引入插件更轻。

use std::path::Path;

/// 把 `src` 复制到 `dest`（另存为），返回最终目标路径。
///
/// 截图原件落在 `%TEMP%/nuphus/captures`（`toolbar.rs` 的保存链路），系统清理后
/// 就没了；本命令是「把产出带出 Nuphus」的出口，不做移动、不改源文件。
#[tauri::command(async)]
pub fn copy_file_as(src: String, dest: String) -> Result<String, String> {
    let src_path = Path::new(&src);
    if !src_path.is_file() {
        return Err(format!("源文件不存在: {src}"));
    }

    let dest_path = Path::new(&dest);
    if let Some(parent) = dest_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(format!("目标目录不存在: {}", parent.display()));
        }
    }

    // 同路径复制：先给清晰文案（fs::copy 自己也会报错，但错误信息不直观）。
    if let (Ok(from), Ok(to)) = (
        std::fs::canonicalize(src_path),
        std::fs::canonicalize(dest_path),
    ) {
        if from == to {
            return Err("源文件与目标路径相同，无需另存".to_string());
        }
    }

    std::fs::copy(src_path, dest_path).map_err(|e| format!("另存为失败: {e}"))?;
    Ok(dest)
}
