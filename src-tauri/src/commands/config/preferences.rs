#[tauri::command]
pub fn get_language() -> Result<String, String> {
    let prefs = nuphus::config::UserPreferences::load();
    Ok(prefs.language)
}

#[tauri::command]
pub fn get_browser_cdp_url() -> Result<String, String> {
    let prefs = nuphus::config::UserPreferences::load();
    Ok(prefs.browser_cdp_url.unwrap_or_default())
}

/// Shared HTTP client for CDP probes: no_proxy — a CDP endpoint is an
/// infrastructure address and must bypass the system proxy.
fn cdp_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("http client: {e}"))
}

/// Map a reqwest failure to a user-actionable message. The raw reqwest text
/// (English, developer-oriented) goes to logs only — never to the UI.
fn classify_connect_error(base: &str, e: &reqwest::Error) -> String {
    tracing::warn!("CDP probe {base} failed: {e}");
    if e.is_timeout() {
        format!("连接 {base} 超时。请确认端点地址与端口正确，且未被防火墙拦截")
    } else if e.is_connect() {
        format!("未检测到浏览器在 {base} 监听。请先用 --remote-debugging-port 调试模式启动浏览器")
    } else {
        format!("无法连接 {base}，请确认端点地址正确")
    }
}

/// GET /json/version and return the remote browser's version string.
/// Error messages are classified by failure cause (not started / timeout /
/// wrong service) so the UI can guide the user's next step.
fn cdp_probe(base: &str) -> Result<String, String> {
    let http = cdp_http_client()?;
    let resp = http
        .get(format!("{base}/json/version"))
        .send()
        .map_err(|e| classify_connect_error(base, &e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "{base} 返回 HTTP {}：该端口上的服务不是浏览器调试端点",
            resp.status()
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|_| format!("{base} 有服务在监听，但不是浏览器调试协议，请确认端口号是否正确"))?;
    Ok(body
        .get("Browser")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string())
}

/// Test connectivity to an external-browser CDP endpoint (GET /json/version).
/// Returns the remote browser's version string on success.
#[tauri::command]
pub fn test_browser_cdp_url(url: String) -> Result<String, String> {
    let base = url.trim().trim_end_matches('/').to_string();
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(
            "CDP 端点必须是 http(s)://host:port 形式，例如 http://127.0.0.1:9222".to_string(),
        );
    }
    Ok(format!("已连接：{}", cdp_probe(&base)?))
}

/// A browser process found running with --remote-debugging-port, already
/// verified by probing its CDP endpoint.
#[derive(serde::Serialize)]
pub struct DetectedBrowser {
    /// Human-readable browser/platform name inferred from the exe path.
    pub name: String,
    /// Full path of the browser executable (identity of the platform).
    pub exe_path: String,
    pub port: u16,
    /// http://127.0.0.1:{port} — ready to persist as browser_cdp_url.
    pub url: String,
    /// Chromium version string from /json/version.
    pub version: String,
    /// Titles of currently open pages (often contain the account name).
    pub pages: Vec<String>,
    /// --user-data-dir of the process that resolved this port (identity for
    /// self-healing: DevToolsActivePort fallback after a window reopen).
    pub user_data_dir: Option<String>,
}

/// Extract the debug port and profile dir from a process command line.
/// Handles both `--flag=value` and `--flag value` forms.
/// Returns (port, user_data_dir). Port may be 0 (= random port chosen by the
/// browser — the actual port is written to DevToolsActivePort in the
/// profile dir; resolve it via `resolve_debug_port`).
fn parse_cmdline(cmd: &[std::ffi::OsString]) -> (Option<u16>, Option<std::path::PathBuf>) {
    let args: Vec<String> = cmd
        .iter()
        .map(|a| a.to_string_lossy().trim_matches('"').to_string())
        .collect();
    let mut port = None;
    let mut profile = None;
    for (i, arg) in args.iter().enumerate() {
        if let Some(v) = arg.strip_prefix("--remote-debugging-port=") {
            port = v.parse::<u16>().ok();
        } else if arg == "--remote-debugging-port" {
            port = args.get(i + 1).and_then(|v| v.parse::<u16>().ok());
        } else if let Some(v) = arg.strip_prefix("--user-data-dir=") {
            profile = Some(std::path::PathBuf::from(v));
        }
    }
    (port, profile)
}

/// Resolve the effective debug port. A literal port is returned as-is;
/// port 0 means the browser picked a random port and wrote it to
/// `<user-data-dir>/DevToolsActivePort` (first line) — observed on
/// fingerprint browsers (AdsPower SunBrowser launches with
/// `--remote-debugging-port=0`).
fn resolve_debug_port(port: u16, profile: Option<&std::path::Path>) -> Option<u16> {
    if port > 0 {
        return Some(port);
    }
    let content = std::fs::read_to_string(profile?.join("DevToolsActivePort")).ok()?;
    content
        .lines()
        .next()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
}

/// Whether the exe filename looks like a browser (used to prefer real
/// browser processes over incidental cmdline matches, e.g. a shell whose
/// command string merely mentions the flag).
fn is_browser_exe(exe: &std::path::Path) -> bool {
    exe.file_name()
        .map(|n| {
            let n = n.to_string_lossy().to_lowercase();
            n.contains("chrome") || n.contains("browser") || n.contains("edge")
        })
        .unwrap_or(false)
}

/// Infer a user-readable browser/platform name from the executable path.
/// Fingerprint platforms install under their own branded directories, so
/// the path is the identity. Known keywords first, generic Chrome/Edge last
/// (their names also appear inside fingerprint-browser install paths).
fn infer_browser_name(exe: &std::path::Path) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("adspower", "AdsPower"),
        ("比特", "比特浏览器"),
        ("bitbrowser", "比特浏览器"),
        ("hubstudio", "HubStudio"),
        ("紫鸟", "紫鸟浏览器"),
        ("zibird", "紫鸟浏览器"),
        ("vmlogin", "VMLogin"),
        ("morelogin", "MoreLogin"),
        ("gologin", "GoLogin"),
        ("dolphin", "Dolphin Anty"),
        ("incogniton", "Incogniton"),
        ("yunlogin", "云登浏览器"),
        ("云登", "云登浏览器"),
        ("maskfog", "MaskFog"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
    ];
    let path = exe.to_string_lossy().to_lowercase();
    for (kw, name) in KNOWN {
        if path.contains(kw) {
            return name.to_string();
        }
    }
    exe.parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "未知浏览器".to_string())
}

/// Detect browsers currently running with a CDP debug port.
///
/// Mechanism: any Chromium-based browser (including every fingerprint
/// browser) must pass --remote-debugging-port on its process command line,
/// so enumerating processes finds candidates without guessing ports or
/// integrating per-vendor APIs. A literal port is used directly; port 0
/// (random — how AdsPower launches its SunBrowser) is resolved via the
/// DevToolsActivePort file in the process's --user-data-dir. Every
/// candidate's endpoint is probed before being returned, so every entry in
/// the result is connectable right now.
#[tauri::command]
pub fn detect_cdp_browsers() -> Result<Vec<DetectedBrowser>, String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );

    // Collect unique ports + a representative (exe path, profile dir) per port.
    // Browser-like exes win over incidental matches (e.g. a shell quoting the flag).
    let mut candidates: std::collections::HashMap<
        u16,
        (std::path::PathBuf, Option<std::path::PathBuf>),
    > = std::collections::HashMap::new();
    for proc in sys.processes().values() {
        let (port, profile) = parse_cmdline(proc.cmd());
        let Some(port) = port.and_then(|p| resolve_debug_port(p, profile.as_deref())) else {
            continue;
        };
        let Some(exe) = proc.exe() else { continue };
        candidates
            .entry(port)
            .and_modify(|e| {
                if is_browser_exe(exe) && !is_browser_exe(&e.0) {
                    *e = (exe.to_path_buf(), profile.clone());
                }
            })
            .or_insert_with(|| (exe.to_path_buf(), profile));
    }

    let mut found = Vec::new();
    for (port, (exe, profile)) in candidates {
        let url = format!("http://127.0.0.1:{port}");
        // Skip endpoints that don't answer the CDP handshake.
        let Ok(version) = cdp_probe(&url) else {
            continue;
        };
        // Page titles often carry the account name — best-effort, never fatal.
        let pages = cdp_http_client()
            .and_then(|http| {
                http.get(format!("{url}/json/list"))
                    .send()
                    .map_err(|e| e.to_string())
            })
            .and_then(|resp| resp.json::<serde_json::Value>().map_err(|e| e.to_string()))
            .ok()
            .and_then(|v| v.as_array().cloned())
            .map(|items| {
                items
                    .iter()
                    .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("page"))
                    .filter_map(|p| p.get("title").and_then(|t| t.as_str()))
                    .filter(|t| !t.is_empty())
                    .take(5)
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        found.push(DetectedBrowser {
            name: infer_browser_name(&exe),
            exe_path: exe.to_string_lossy().into_owned(),
            port,
            url,
            version,
            pages,
            user_data_dir: profile.map(|p| p.to_string_lossy().into_owned()),
        });
    }
    found.sort_by_key(|b| b.port);
    tracing::info!("CDP browser detection found {} candidate(s)", found.len());
    Ok(found)
}

/// Persist the external-browser CDP endpoint (with optional identity) and apply
/// it to all live channels (direct shared client + pooled MCP child).
/// Empty string = managed Chrome (identity cleared). A URL without identity
/// (legacy/manual path) also clears any stale identity — the identity must
/// describe the browser the URL points to.
#[tauri::command]
pub async fn set_browser_cdp_url(
    url: String,
    identity: Option<nuphus::config::BrowserIdentity>,
) -> Result<String, String> {
    let trimmed = url.trim().to_string();
    if !trimmed.is_empty() && !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(
            "CDP 端点必须是 http(s)://host:port 形式，例如 http://127.0.0.1:9222".to_string(),
        );
    }
    // Identity only makes sense alongside a real endpoint.
    let identity = identity.filter(|_| !trimmed.is_empty());

    let mut prefs = nuphus::config::UserPreferences::load();
    prefs.browser_cdp_url = Some(trimmed.clone());
    prefs.browser_identity = identity.clone();
    prefs.save().map_err(|e| e.to_string())?;

    nuphus::tools::browser_tools::apply_browser_cdp_url(
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.clone())
        },
        identity,
    )
    .await?;

    tracing::info!(
        "Browser CDP url set to: {}",
        if trimmed.is_empty() {
            "<managed chrome>"
        } else {
            &trimmed
        }
    );
    Ok(trimmed)
}

/// Current external-browser connection as shown on the settings page: the
/// persisted endpoint plus the picked browser's identity (all `None`/empty in
/// managed-Chrome mode or for legacy URL-only configs).
#[derive(serde::Serialize)]
pub struct BrowserConnection {
    pub url: String,
    pub name: Option<String>,
    pub exe_path: Option<String>,
    pub user_data_dir: Option<String>,
}

#[tauri::command]
pub fn get_browser_connection() -> Result<BrowserConnection, String> {
    let prefs = nuphus::config::UserPreferences::load();
    let url = prefs.browser_cdp_url.unwrap_or_default();
    let (name, exe_path, user_data_dir) = match prefs.browser_identity {
        Some(id) => (Some(id.name), Some(id.exe_path), id.user_data_dir),
        None => (None, None, None),
    };
    Ok(BrowserConnection {
        url,
        name,
        exe_path,
        user_data_dir,
    })
}

#[tauri::command]
pub fn set_language(lang: String) -> Result<String, String> {
    let mut prefs = nuphus::config::UserPreferences::load();
    prefs.language = lang.clone();
    prefs.save().map_err(|e| e.to_string())?;
    tracing::info!("Language set to: {}", lang);
    Ok(lang)
}

#[tauri::command]
pub fn set_project_dir(
    path: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<String, String> {
    let mut prefs = nuphus::config::UserPreferences::load();
    let old_path = prefs.project_dir.clone();
    prefs.project_dir = path.clone();
    prefs.save().map_err(|e| e.to_string())?;

    // 如果有活跃 session，push 一条 system message 通知 LLM 路径变更
    if let Ok(mut guard) = state.runtime.lock() {
        if let Some(agent) = guard.leader_agent.as_mut() {
            let msg = if old_path.is_empty() {
                format!("## 项目目录已设置\n项目目录已设置为: **{}**", path)
            } else {
                format!(
                    "## 项目目录已变更\n项目目录已从 **{}** 变更为 **{}**",
                    old_path, path
                )
            };
            agent.session_mut().push_system(msg);
            tracing::info!(
                "Project dir change notification pushed to session: {} -> {}",
                old_path,
                path
            );
        }
    }

    tracing::info!("Project dir set to: {}", path);
    Ok(path)
}

/// Sync project bookmarks from frontend (localStorage-backed, no server persistence needed)
#[tauri::command]
pub fn set_project_bookmarks(bookmarks: Vec<serde_json::Value>) -> Result<(), String> {
    tracing::info!("Project bookmarks synced: {} entries", bookmarks.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(args: &[&str]) -> Vec<std::ffi::OsString> {
        args.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn parse_literal_port_and_profile() {
        let (port, profile) = parse_cmdline(&cmd(&[
            "chrome.exe",
            "--remote-debugging-port=9222",
            "--user-data-dir=C:\\tmp\\prof",
        ]));
        assert_eq!(port, Some(9222));
        assert_eq!(profile, Some(std::path::PathBuf::from("C:\\tmp\\prof")));
    }

    #[test]
    fn parse_quoted_user_data_dir() {
        let (port, profile) = parse_cmdline(&cmd(&[
            "sunbrowser.exe",
            "\"--user-data-dir=C:\\.ADSPOWER_GLOBAL\\cache\\k1ffh0or\"",
            "--remote-debugging-port=0",
        ]));
        assert_eq!(port, Some(0));
        assert_eq!(
            profile,
            Some(std::path::PathBuf::from(
                "C:\\.ADSPOWER_GLOBAL\\cache\\k1ffh0or"
            ))
        );
    }

    #[test]
    fn resolve_random_port_via_devtools_active_port() {
        // AdsPower SunBrowser launches with --remote-debugging-port=0; the real
        // port lands in <user-data-dir>/DevToolsActivePort (first line).
        let dir = std::env::temp_dir().join(format!("nuphus-dap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("DevToolsActivePort"),
            "54738\n/devtools/browser/abc\n",
        )
        .unwrap();
        assert_eq!(resolve_debug_port(0, Some(&dir)), Some(54738));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_random_port_without_profile_is_none() {
        assert_eq!(resolve_debug_port(0, None), None);
    }

    #[test]
    fn infer_name_identifies_fingerprint_platform() {
        let exe = std::path::Path::new(
            r"C:\Users\x\AppData\Roaming\adspower_global\cwd_global\chrome_150\sunbrowser.exe",
        );
        assert_eq!(infer_browser_name(exe), "AdsPower");
    }
}
