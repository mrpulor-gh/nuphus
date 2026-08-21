//! Chrome/Chromium/Edge detection module
//!
//! Cross-platform detection of browser installation locations

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ChromeError {
    #[error("Chrome not found. Please install Google Chrome, Chromium, or Microsoft Edge.\nDownload: https://www.google.com/chrome/")]
    NotFound,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Find a system-installed Chrome/Chromium/Edge
///
/// Detection order: Chrome → Chromium → Edge
pub fn find_chrome() -> Result<PathBuf, ChromeError> {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            // System-level install (64-bit)
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            // System-level install (32-bit)
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            // User-level install (via both %USERNAME% and %LOCALAPPDATA% expansions)
            r"C:\Users\%USERNAME%\AppData\Local\Google\Chrome\Application\chrome.exe",
            r"%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe",
            // Beta / Dev / Canary variants
            r"C:\Program Files\Google\Chrome Beta\Application\chrome.exe",
            r"C:\Program Files\Google\Chrome Dev\Application\chrome.exe",
            r"C:\Program Files\Google\Chrome SxS\Application\chrome.exe",
            r"%LOCALAPPDATA%\Google\Chrome Beta\Application\chrome.exe",
            r"%LOCALAPPDATA%\Google\Chrome Dev\Application\chrome.exe",
            r"%LOCALAPPDATA%\Google\Chrome SxS\Application\chrome.exe",
            // Edge (system-level)
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            // Edge (user-level)
            r"%LOCALAPPDATA%\Microsoft\Edge\Application\msedge.exe",
            // Scoop package manager installs
            r"%USERPROFILE%\scoop\apps\googlechrome\current\chrome.exe",
            r"%USERPROFILE%\scoop\apps\chromium\current\chrome.exe",
            // Chocolatey package manager installs
            r"C:\Program Files\Chromium\Application\chrome.exe",
            r"C:\Program Files (x86)\Chromium\Application\chrome.exe",
        ];

        for path in &candidates {
            let expanded = expand_env_vars(path);
            if Path::new(&expanded).exists() {
                return Ok(PathBuf::from(expanded));
            }
        }

        // Try the registry
        if let Some(path) = find_chrome_from_registry() {
            return Ok(path);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ];

        for path in &candidates {
            if Path::new(path).exists() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
            "/snap/bin/chromium",
        ];

        for path in &candidates {
            if Path::new(path).exists() {
                return Ok(PathBuf::from(path));
            }
        }

        // Try the `which` command
        if let Ok(output) = std::process::Command::new("which")
            .args([
                "google-chrome",
                "chromium",
                "chromium-browser",
                "microsoft-edge",
            ])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && Path::new(trimmed).exists() {
                    return Ok(PathBuf::from(trimmed));
                }
            }
        }
    }

    Err(ChromeError::NotFound)
}

#[cfg(target_os = "windows")]
fn find_chrome_from_registry() -> Option<PathBuf> {
    use std::process::Command;

    // Try to read the Chrome / Edge install path from the registry
    let reg_paths = [
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe",
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe",
        r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe",
    ];

    for reg_path in &reg_paths {
        if let Ok(output) = Command::new("reg")
            .args(["query", reg_path, "/ve"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse the registry output to extract the path
            for line in stdout.lines() {
                if line.contains("REG_SZ") {
                    let parts: Vec<&str> = line.split("REG_SZ").collect();
                    if parts.len() >= 2 {
                        let path = parts[1].trim();
                        if Path::new(path).exists() {
                            return Some(PathBuf::from(path));
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn expand_env_vars(path: &str) -> String {
    let mut result = path.to_string();
    for (var_name, env_key) in [
        ("%USERNAME%", "USERNAME"),
        ("%LOCALAPPDATA%", "LOCALAPPDATA"),
        ("%USERPROFILE%", "USERPROFILE"),
        ("%PROGRAMFILES%", "PROGRAMFILES"),
        ("%PROGRAMFILES(X86)%", "PROGRAMFILES(X86)"),
    ] {
        if result.contains(var_name) {
            if let Ok(value) = std::env::var(env_key) {
                result = result.replace(var_name, &value);
            }
        }
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn expand_env_vars(path: &str) -> String {
    path.to_string()
}

/// Get the Nuphus browser Profile directory
pub fn get_profile_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    base.join("Nuphus").join("browser_profile_v2")
}

/// Ensure the Profile directory exists
pub fn ensure_profile_dir() -> std::io::Result<PathBuf> {
    let dir = get_profile_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_chrome() {
        // Just test that it does not panic; not guaranteed to find anything
        let _ = find_chrome();
    }

    #[test]
    fn test_profile_dir() {
        let dir = get_profile_dir();
        assert!(dir.to_string_lossy().contains("Nuphus"));
        assert!(dir.to_string_lossy().contains("browser_profile"));
    }
}
