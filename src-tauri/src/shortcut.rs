//! Windows 便携模式桌面快捷方式自建。
//!
//! npm 一键安装（`npm i -g @nuphus/nuphus-desktop`）或手工拷贝的便携包，
//! 其 exe 不经过 NSIS 安装器 → 不会自动生成桌面快捷方式 → 用户找不到应用入口。
//! 这里在应用启动时检测"便携模式"：exe 不在任何标准安装目录下即判定为便携，
//! 然后用 PowerShell WScript.Shell COM 在用户桌面创建 Nuphus.lnk 指向自身 exe。
//! 幂等：仅当 .lnk 不存在时创建；NSIS 安装（perUser/Program Files）场景自动跳过。

use std::path::Path;

/// 启动时调用。仅 Windows + 发布构建 + 便携模式下才可能建快捷方式，其余情况立即返回。
pub fn ensure_portable_desktop_shortcut() {
    #[cfg(target_os = "windows")]
    {
        // 开发构建跳过：cargo run 跑的是 target/debug，不该给桌面留图标
        if cfg!(debug_assertions) {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        if !is_portable_path(&exe) {
            return; // 标准安装目录 → 安装器已建快捷方式
        }
        let Some(desktop) = dirs::desktop_dir() else { return };
        let lnk = desktop.join("Nuphus.lnk");
        if lnk.exists() {
            return; // 幂等：已存在不重复创建
        }
        if let Err(e) = create_shortcut(&exe, &lnk) {
            tracing::warn!("Failed to create portable desktop shortcut: {e}");
        }
    }
}

/// 便携模式判定：exe 是否不在任何标准安装目录下。
#[cfg(target_os = "windows")]
fn is_portable_path(exe: &Path) -> bool {
    let exe_str = exe.to_string_lossy().to_lowercase();
    // perMachine 安装：Program Files / Program Files (x86)
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(dir) = std::env::var(var) {
            let dir = dir.trim().to_lowercase();
            if !dir.is_empty() && exe_str.starts_with(&dir) {
                return false;
            }
        }
    }
    // perUser 安装（NSIS currentUser）：%LOCALAPPDATA%\Programs
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let programs = Path::new(local.trim())
            .join("Programs")
            .to_string_lossy()
            .to_lowercase();
        if !programs.is_empty() && exe_str.starts_with(&programs) {
            return false;
        }
    }
    true
}

/// 用 PowerShell WScript.Shell COM 创建 .lnk。路径中的单引号按 PS 转义规则翻倍。
#[cfg(target_os = "windows")]
fn create_shortcut(exe: &Path, lnk: &Path) -> std::io::Result<()> {
    let esc = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{lnk}');\
         $s.TargetPath='{exe}';\
         $s.WorkingDirectory='{wd}';\
         $s.IconLocation='{exe},0';\
         $s.Description='Nuphus 桌面助手';\
         $s.Save()",
        lnk = esc(lnk),
        exe = esc(exe),
        wd = esc(exe.parent().unwrap_or(exe)),
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .status()?;
    if status.success() {
        tracing::info!("Created portable desktop shortcut: {}", lnk.display());
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "powershell exited with {status}"
        )))
    }
}
