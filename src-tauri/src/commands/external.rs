//! external.rs — 用系统默认浏览器打开外链。
//!
//! 背景：桌面端 WebView 不处理 `target="_blank"`（既不开新窗也不报错），
//! 页面里的外链点了毫无反应。前端把这类点击统一拦下并调用本命令，
//! 由系统浏览器接管 —— 与 `preview.rs` 的 `reveal_path` 同一套做法。
//!
//! 安全边界：只放行 `http` / `https`，拒绝控制字符与超长 URL；
//! 参数逐个传参、不经 shell 拼接（Windows 走 `cmd /C start "" <url>`，
//! 空的窗口标题参数可避免 `start` 把 URL 当标题解析）。

/// 允许打开的外链（纯函数，便于单测）。
///
/// 规则：非空、长度 ≤ 2048、无控制字符（含换行/制表）、scheme 为 http/https
/// 且带非空 host。
pub fn is_allowed_external_url(url: &str) -> bool {
    let u = url.trim();
    if u.is_empty() || u.len() > 2048 {
        return false;
    }
    if u.chars().any(|c| c.is_control()) {
        return false;
    }
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
        .or_else(|| u.strip_prefix("HTTP://"))
        .or_else(|| u.strip_prefix("HTTPS://"));
    let Some(rest) = rest else {
        return false;
    };
    // host 部分必须非空，且不含空白（`http:// /x` 之类形态直接拒绝）
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.chars().any(char::is_whitespace)
}

/// 用系统默认浏览器打开外链。失败返回可读原因（前端仅提示，不打断）。
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    let url = url.trim().to_string();
    if !is_allowed_external_url(&url) {
        return Err(format!("不支持的链接（仅允许 http/https）：{url}"));
    }

    #[cfg(target_os = "windows")]
    {
        // 第二个参数是窗口标题（空串）→ 防止 `start` 把 URL 当标题吞掉
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
            .map_err(|e| format!("打开链接失败：{e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开链接失败：{e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开链接失败：{e}"))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_allowed_external_url;

    #[test]
    fn allows_http_and_https() {
        assert!(is_allowed_external_url(
            "https://github.com/mrpulor-gh/nuphus"
        ));
        assert!(is_allowed_external_url("http://example.com/a?b=1#c"));
        assert!(is_allowed_external_url("HTTPS://Example.com"));
    }

    #[test]
    fn rejects_non_http_schemes_and_malformed() {
        assert!(!is_allowed_external_url("mailto:a@b.com"));
        assert!(!is_allowed_external_url("file:///C:/Windows"));
        assert!(!is_allowed_external_url("javascript:alert(1)"));
        assert!(!is_allowed_external_url(""));
        assert!(!is_allowed_external_url("https://"));
        assert!(!is_allowed_external_url("http:// /x"));
    }

    #[test]
    fn rejects_control_chars_and_overlong() {
        assert!(!is_allowed_external_url("https://example.com/\nnext"));
        assert!(!is_allowed_external_url("https://example.com/\u{0}"));
        let long = format!("https://example.com/{}", "a".repeat(3000));
        assert!(!is_allowed_external_url(&long));
    }
}
