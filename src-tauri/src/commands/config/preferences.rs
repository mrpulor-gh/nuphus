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

/// 项目目录展示名（路径末段）：兼容正反斜杠与结尾分隔符；空路径 → 空串。
/// 展示名规则唯一实现在 utils（书签 / 会话归属自动组共用同一规则）。
fn project_dir_display_name(dir: &str) -> String {
    nuphus::utils::dir_display_name(dir)
}

/// 项目记忆标签（`memory/{tag}.md` 的文件名）；空目录 → "default"。
fn project_tag_of(dir: &str) -> String {
    nuphus::utils::derive_project_tag_from_dir(dir).unwrap_or_else(|| "default".to_string())
}

/// 项目目录状态载荷（前端「项目中心」数据源）。
fn project_dir_payload(path: &str) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "name": project_dir_display_name(path),
        "tag": project_tag_of(path),
    })
}

/// 项目目录**状态**文案 —— 写入会话 ACTIVE REMINDERS（每轮随 user 消息注入）。
///
/// 用状态式表述而非「已切换」事件：模型每一轮都能直接看到当前工作目录与记忆文件，
/// 因此切换发生在任何时刻（含执行中 agent 不在槽内）都不会错过通知。
fn project_state_reminder(path: &str) -> String {
    if path.trim().is_empty() {
        return "当前项目目录：未设置（相对路径以运行目录为准）。".to_string();
    }
    format!(
        "当前项目目录：{}（项目「{}」，记忆文件 memory/{}.md）——相对路径以该目录为基准；其它项目的路径与既有结论不再沿用。",
        path,
        project_dir_display_name(path),
        project_tag_of(path)
    )
}

/// 读取当前项目目录（项目中心初始化数据源）。
#[tauri::command]
pub fn get_project_dir() -> Result<serde_json::Value, String> {
    let prefs = nuphus::config::UserPreferences::load();
    Ok(project_dir_payload(&prefs.project_dir))
}

/// 设置 / 切换项目目录 —— 项目中心的唯一入口。
///
/// 设计约束（2026-09-15 定稿）：
/// - **不做提示缓存失效**：L1 是稳定前缀，失效会破坏上游 prompt cache 命中率；
/// - 切换改由 **user 内部消息** 传达（`push_user_internal`：只进 LLM 上下文、
///   前端历史拉取时过滤不显示），模型下一轮即以新工作目录 / 新记忆文件为准；
/// - 覆盖全部活跃槽（leader + workflow），不再只通知 leader 一侧。
#[tauri::command]
pub fn set_project_dir(
    path: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<serde_json::Value, String> {
    let path = path.trim().to_string();
    let mut prefs = nuphus::config::UserPreferences::load();
    let old_path = prefs.project_dir.clone();
    if old_path == path {
        // 幂等：无变化时不重复落盘、不重复注入
        return Ok(project_dir_payload(&path));
    }

    prefs.project_dir = path.clone();
    prefs.save().map_err(|e| e.to_string())?;

    // 项目目录变化 → 写入共享「待注入提示」队列：由下一个轮次边界随 user 消息带出
    // 一次即消费（与执行中追加指令 append_queue 同一条链路），因此项目目录在**执行中**
    // 被切换同样不会丢失提示；也不触碰系统提示前缀缓存，不会每轮重复注入。
    nuphus::state::SignalState::push_notice(&state.signals, project_state_reminder(&path));

    tracing::info!("Project dir changed: {} -> {}", old_path, path);
    Ok(project_dir_payload(&path))
}

/// 读取项目书签（项目中心数据源；与 project_dir 同源落盘 prefs）。
#[tauri::command]
pub fn get_project_bookmarks() -> Result<Vec<nuphus::config::ProjectBookmark>, String> {
    Ok(nuphus::config::UserPreferences::load().project_bookmarks)
}

/// 写入项目书签：整表替换（前端增删后提交），去空、按路径去重、名称兜底取目录名。
///
/// `archived` 不由本命令清除：同路径书签沿用已落盘的归档标记（前端提交 `true` 时亦
/// 沿用）——归档状态只经 [`set_project_folder_archived`] 增删，避免整表替换静默
/// 取消归档。
#[tauri::command]
pub fn set_project_bookmarks(
    bookmarks: Vec<nuphus::config::ProjectBookmark>,
) -> Result<Vec<nuphus::config::ProjectBookmark>, String> {
    let existing = nuphus::config::UserPreferences::load().project_bookmarks;
    let mut normalized: Vec<nuphus::config::ProjectBookmark> = Vec::new();
    for bm in bookmarks {
        let path = bm.path.trim().to_string();
        if path.is_empty() || normalized.iter().any(|x| x.path == path) {
            continue;
        }
        let name = if bm.name.trim().is_empty() {
            project_dir_display_name(&path)
        } else {
            bm.name.trim().to_string()
        };
        let candidate = nuphus::config::ProjectBookmark {
            name,
            path,
            archived: bm.archived,
        };
        let archived = keep_archived(&existing, &candidate);
        normalized.push(nuphus::config::ProjectBookmark {
            name: candidate.name,
            path: candidate.path,
            archived,
        });
    }

    let mut prefs = nuphus::config::UserPreferences::load();
    prefs.project_bookmarks = normalized.clone();
    prefs.save().map_err(|e| e.to_string())?;
    tracing::info!("Project bookmarks saved: {} entries", normalized.len());
    Ok(normalized)
}

/// 归档 / 恢复项目文件夹（归档 = 在会话工作台隐藏，可恢复）。
///
/// - 已有书签 → 只改归档标记，名称与顺序不变；
/// - 未收藏但有会话的自动组 → 补一条 `archived=true` 的书签记录（否则无锚点可恢复）；
/// - 恢复不存在的记录 → 幂等无操作；
/// - 归档只影响会话台分组展示，**不触碰 `project_dir`**，记忆检索的项目过滤不受影响。
#[tauri::command]
pub fn set_project_folder_archived(
    path: String,
    archived: bool,
) -> Result<Vec<nuphus::config::ProjectBookmark>, String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("empty_path".to_string());
    }
    let mut prefs = nuphus::config::UserPreferences::load();
    prefs.project_bookmarks = apply_folder_archived(prefs.project_bookmarks, &path, archived);
    prefs.save().map_err(|e| e.to_string())?;
    tracing::info!("Project folder archived={archived}: {path}");
    Ok(prefs.project_bookmarks)
}

/// 整表替换时的归档保留判定（纯函数，便于测试）：同路径书签沿用已落盘的归档标记，
/// 归档状态只能经 [`set_project_folder_archived`] 变更。
fn keep_archived(
    existing: &[nuphus::config::ProjectBookmark],
    incoming: &nuphus::config::ProjectBookmark,
) -> bool {
    incoming.archived
        || existing
            .iter()
            .any(|x| x.path == incoming.path && x.archived)
}

/// 归档状态应用（纯函数，便于测试）：在既有书签表上设置某目录的归档标记。
fn apply_folder_archived(
    bookmarks: Vec<nuphus::config::ProjectBookmark>,
    path: &str,
    archived: bool,
) -> Vec<nuphus::config::ProjectBookmark> {
    let mut out = bookmarks;
    match out.iter_mut().find(|b| b.path.trim() == path) {
        Some(bm) => bm.archived = archived,
        // 自动组（未收藏）归档：补一条归档记录，作为恢复入口的锚点
        None if archived => out.push(nuphus::config::ProjectBookmark {
            name: project_dir_display_name(path),
            path: path.to_string(),
            archived: true,
        }),
        // 无记录可恢复：幂等无操作
        None => {}
    }
    out
}

/// 设置会话分组折叠上限（全局单值，设置中心入口）。
///
/// 0 无意义（会把每个分组都折叠成空列表）→ 显式拒绝，避免配置静默失效。
#[tauri::command]
pub fn set_session_group_collapsed_limit(limit: u32) -> Result<u32, String> {
    if limit == 0 {
        return Err("invalid_limit".to_string());
    }
    let mut prefs = nuphus::config::UserPreferences::load();
    prefs.session_group_collapsed_limit = limit;
    prefs.save().map_err(|e| e.to_string())?;
    tracing::info!("Session group collapsed limit set to: {limit}");
    Ok(prefs.session_group_limit())
}

/// 设置会话工作台排序偏好（组序维度 + 组内排序键，两个维度互相独立）。
///
/// 非法取值**不报错而是归一**（手改配置 / 旧前端残留不应致命），并把归一后的结果
/// 回给前端——前端据此校准本地状态，避免「界面显示 A、落盘 B」。
///
/// 生效路径：`list_shelf_sessions` 每次返回 `sort_prefs` → 会话工作台 5s 轮询即读到
/// 新值（桌面 SessionRail / 移动端 NavBar 共用同一返回体）。
#[tauri::command]
pub fn set_session_sort_prefs(
    group_order: String,
    sort_key: String,
) -> Result<serde_json::Value, String> {
    let mut prefs = nuphus::config::UserPreferences::load();
    let (group_order, sort_key) = apply_session_sort_prefs(&mut prefs, &group_order, &sort_key);
    prefs.save().map_err(|e| e.to_string())?;
    tracing::info!("Session sort prefs set: group_order={group_order} sort_key={sort_key}");
    Ok(serde_json::json!({
        "group_order": group_order,
        "sort_key": sort_key,
    }))
}

/// 排序偏好归一 + 写回（纯逻辑，便于测试）：返回即落盘值。
///
/// 归一实现收敛在 `nuphus::config::normalize_session_*`（与 `list_shelf_sessions`
/// 下发读数同源），此处只负责写回，不再自造第二套判定。
fn apply_session_sort_prefs(
    prefs: &mut nuphus::config::UserPreferences,
    group_order: &str,
    sort_key: &str,
) -> (String, String) {
    let group_order = nuphus::config::normalize_session_group_order(group_order).to_string();
    let sort_key = nuphus::config::normalize_session_sort_key(sort_key).to_string();
    prefs.session_group_order = group_order.clone();
    prefs.session_sort_key = sort_key.clone();
    (group_order, sort_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与生产书签结构一致（name/path/archived）。
    fn bookmark(name: &str, path: &str, archived: bool) -> nuphus::config::ProjectBookmark {
        nuphus::config::ProjectBookmark {
            name: name.to_string(),
            path: path.to_string(),
            archived,
        }
    }

    /// 归档 / 恢复往返：书签上的标记可逆，名称与顺序不变。
    #[test]
    fn folder_archive_restore_roundtrip() {
        let initial = vec![
            bookmark("A", "E:\\work\\A", false),
            bookmark("B", "E:\\work\\B", false),
        ];

        let archived = apply_folder_archived(initial, "E:\\work\\A", true);
        assert_eq!(archived.len(), 2, "归档不得增删书签");
        assert!(archived[0].archived, "目标书签应被标记归档");
        assert_eq!(archived[0].name, "A");
        assert!(!archived[1].archived, "其它书签不受影响");

        let restored = apply_folder_archived(archived, "E:\\work\\A", false);
        assert_eq!(restored.len(), 2);
        assert!(!restored[0].archived, "恢复后归档标记应被清除");
        assert_eq!(restored[0].path, "E:\\work\\A");
    }

    /// 未收藏的自动组也能归档（补一条 archived=true 记录作为恢复锚点）；
    /// 恢复不存在的记录是幂等无操作。
    #[test]
    fn archive_auto_group_creates_anchor_record() {
        let archived = apply_folder_archived(Vec::new(), "E:\\work\\Auto", true);
        assert_eq!(archived.len(), 1, "自动组归档应补一条记录");
        assert!(archived[0].archived);
        assert_eq!(archived[0].name, "Auto", "名称兜底取目录末段");
        assert_eq!(archived[0].path, "E:\\work\\Auto");

        // 无记录时恢复：不新增、不报错
        assert!(apply_folder_archived(Vec::new(), "E:\\work\\Auto", false).is_empty());
    }

    /// 归档标记不被整表替换清除：同路径书签沿用已落盘的 archived（回归保护）。
    #[test]
    fn bookmarks_replacement_preserves_archived_flag() {
        let existing = vec![
            bookmark("A", "E:\\work\\A", true),
            bookmark("B", "E:\\work\\B", false),
        ];
        // 旧前端整表替换时不提交 archived 字段（serde default → false）
        assert!(
            keep_archived(&existing, &bookmark("A", "E:\\work\\A", false)),
            "已归档书签在整表替换后必须保持归档"
        );
        assert!(
            !keep_archived(&existing, &bookmark("B", "E:\\work\\B", false)),
            "未归档书签不得被误标归档"
        );
        assert!(
            keep_archived(&existing, &bookmark("C", "E:\\work\\C", true)),
            "显式提交归档标记应被接受"
        );
    }

    /// 排序偏好往返：写 → 读回一致；反向写回默认值同样成立。
    /// 只测纯逻辑 `apply_session_sort_prefs`（不 load/save 真实 prefs 文件，
    /// 避免测试污染开发者本机配置）。
    #[test]
    fn set_session_sort_prefs_roundtrip() {
        let mut prefs = nuphus::config::UserPreferences::default();

        let applied = apply_session_sort_prefs(&mut prefs, "recent", "created");
        assert_eq!(applied, ("recent".to_string(), "created".to_string()));
        assert_eq!(prefs.session_group_order(), "recent", "组序维度应写回");
        assert_eq!(prefs.session_sort_key(), "created", "组内键应写回");
        assert_eq!(prefs.session_group_order, "recent", "落盘值不得被归一改写");
        assert_eq!(prefs.session_sort_key, "created");

        let applied = apply_session_sort_prefs(&mut prefs, "bookmark", "updated");
        assert_eq!(applied, ("bookmark".to_string(), "updated".to_string()));
        assert_eq!(prefs.session_group_order(), "bookmark");
        assert_eq!(prefs.session_sort_key(), "updated");
    }

    /// 非法取值归一（不拒绝、不落非法值）：返回的即落盘值，前端据此校准本地状态。
    #[test]
    fn set_session_sort_prefs_normalizes_illegal_values() {
        let mut prefs = nuphus::config::UserPreferences::default();

        let applied = apply_session_sort_prefs(&mut prefs, " RECENT ", "whatever");
        assert_eq!(applied, ("recent".to_string(), "updated".to_string()));
        assert_eq!(prefs.session_group_order, "recent");
        assert_eq!(prefs.session_sort_key, "updated");

        let applied = apply_session_sort_prefs(&mut prefs, "", "");
        assert_eq!(applied, ("bookmark".to_string(), "updated".to_string()));
        assert_eq!(prefs.session_group_order, "bookmark");
        assert_eq!(prefs.session_sort_key, "updated");
    }

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
