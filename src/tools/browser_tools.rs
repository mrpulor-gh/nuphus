//! browser_tools — 浏览器工具的异步执行入口
//!
//! 浏览器工具基于 CDP(Chrome DevTools Protocol)实现,需要异步执行。
//! ToolRegistry::execute() 同步入口会检测 browser_ 前缀并返回提示要求改用 execute_browser_tool。
//!
//! 连接级自愈：每个操作经 `run_browser_op_with_reconnect` 执行——CDP 连接死亡
//! （Chrome 被杀/崩溃，表现为快速连接错误或 Windows 半开 websocket 上的卡死超时）
//! 时自动重置+重连+重试一次，工作流中途浏览器死亡不再变成一连串用户可见错误。

use super::registry::ToolRegistry;
use crate::browser::{BrowserClient, BrowserError};
use crate::ToolResult;
use std::time::Duration;

/// 注册 nuphus-browser 的 cookie 数据源（进程内一次）。
///
/// 浏览器模块抽离为独立 crate 后，`import_cookies` 不再直接引用主 crate 的
/// `crate::cookies::vault()`，改为可插拔 loader——此处把 vault 注册为数据源。
/// 只在执行浏览器工具前惰性注册一次，保证 `browser_import_cookies` 可用。
fn register_cookie_source() {
    static REGISTERED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    REGISTERED.get_or_init(|| {
        let _ = nuphus_browser::cookie_source::set_loader(|domain_filter| {
            crate::cookies::vault()
                .refresh(domain_filter)
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|c| nuphus_browser::cookie_source::CookieData {
                            name: c.name,
                            value: c.value,
                            domain: c.domain,
                            path: c.path,
                            secure: c.secure,
                            http_only: c.http_only,
                            same_site: c.same_site,
                            expires: c.expires,
                        })
                        .collect()
                })
        });
    });
}

/// Apply a browser CDP preference change to every live channel.
///
/// `url`: `Some(non-empty)` = drive the given external browser (e.g. a
/// fingerprint browser started with `--remote-debugging-port`); `None` or
/// `Some("")` = back to the managed Chrome.
/// `identity`: the picked browser's identity (name/exe_path/user_data_dir) —
/// mirrored to env so future `BrowserClient::new()` / MCP spawns can self-heal
/// when the window reopens on a new port; `None` strips the identity envs.
///
/// Three channels are reconciled:
/// 1. Process env — future `BrowserClient::new()` / MCP spawns read it.
/// 2. Direct channel — the shared client's connection is switched immediately
///    (`set_external_cdp_url` closes the current connection; next launch uses
///    the new endpoint. Closing an external attach never kills the user's browser).
/// 3. MCP channel — the pooled `nuphus-mcp` child is dropped so the next call
///    respawns it with the updated env.
pub async fn apply_browser_cdp_url(
    url: Option<String>,
    identity: Option<crate::config::BrowserIdentity>,
) -> Result<(), String> {
    let normalized = url
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());

    match &normalized {
        Some(u) => std::env::set_var("NUPHUS_MCP_BROWSER_CDP_URL", u),
        None => std::env::remove_var("NUPHUS_MCP_BROWSER_CDP_URL"),
    }
    // Identity envs: set when an identity is provided, stripped otherwise
    // (back-to-managed and URL-without-identity both clear stale identity).
    match &identity {
        Some(id) => {
            std::env::set_var("NUPHUS_BROWSER_NAME", &id.name);
            std::env::set_var("NUPHUS_BROWSER_EXE_PATH", &id.exe_path);
            match &id.user_data_dir {
                Some(dir) => std::env::set_var("NUPHUS_BROWSER_USER_DATA_DIR", dir),
                None => std::env::remove_var("NUPHUS_BROWSER_USER_DATA_DIR"),
            }
        }
        None => {
            std::env::remove_var("NUPHUS_BROWSER_NAME");
            std::env::remove_var("NUPHUS_BROWSER_EXE_PATH");
            std::env::remove_var("NUPHUS_BROWSER_USER_DATA_DIR");
        }
    }

    // Direct channel: shared singleton client (ToolRegistry.browser_client wraps it).
    let client_identity = identity.map(|id| crate::browser::ExternalIdentity {
        name: id.name,
        exe_path: id.exe_path,
        user_data_dir: id.user_data_dir,
    });
    {
        let shared = crate::browser::shared_client();
        let mut guard = shared.lock().await;
        if let Some(client) = guard.as_mut() {
            client
                .set_external_cdp_url(normalized.clone(), client_identity)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    // MCP channel: force respawn so the child process picks up the new env.
    crate::mcp::client::drop_server(crate::mcp::dual::NUPHUS_MCP_SERVER).await;

    Ok(())
}

/// Reconcile after a successful browser op: `attach_external` self-healing may
/// have switched the client to a re-resolved endpoint (fingerprint window
/// reopened on a new port). Persist the live endpoint back to env +
/// preferences.json and drop the pooled MCP child so its next respawn picks up
/// the new value — otherwise the next app restart would attach to the stale port.
async fn reconcile_external_cdp_url(client: &BrowserClient) {
    let Some(current) = client.external_cdp_url().map(|s| s.to_string()) else {
        return; // managed-Chrome mode: nothing to reconcile
    };
    let env_url = std::env::var("NUPHUS_MCP_BROWSER_CDP_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    if env_url.as_deref() == Some(current.as_str()) {
        return;
    }
    tracing::info!("[browser] external endpoint self-healed to {current}; persisting");
    std::env::set_var("NUPHUS_MCP_BROWSER_CDP_URL", &current);
    let mut prefs = crate::config::UserPreferences::load();
    prefs.browser_cdp_url = Some(current.clone());
    if let Err(e) = prefs.save() {
        tracing::warn!("[browser] failed to persist self-healed endpoint: {e}");
    }
    crate::mcp::client::drop_server(crate::mcp::dual::NUPHUS_MCP_SERVER).await;
}

impl ToolRegistry {
    /// 设置浏览器客户端（在新线程中安全初始化，避免 Runtime 嵌套）
    pub fn set_browser_client(&mut self, client: BrowserClient) {
        let bc = self.browser_client.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("[set_browser_client] Failed to create runtime: {}", e);
                    return;
                }
            };
            rt.block_on(async {
                let mut guard = bc.lock().await;
                *guard = Some(client);
            });
        });
    }

    /// 执行浏览器工具(异步)
    pub async fn execute_browser_tool(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> std::result::Result<ToolResult, String> {
        register_cookie_source();

        // 双通道（dogfooding）：MCP 优先，失败回退直连
        match crate::mcp::dual::route_tool(tool_name, params).await {
            crate::mcp::dual::RouteOutcome::Handled(result) => return Ok(result),
            crate::mcp::dual::RouteOutcome::Fallback(reason) => {
                tracing::debug!(
                    "[dual] browser '{}' falls back to direct: {}",
                    tool_name,
                    reason
                );
            }
        }

        // 跨进程自动化锁：MCP 通道不可用而回退直连时，与各 nuphus-mcp 实例
        // （其他 Agent）通过同一锁文件互斥。MCP 可用时锁已在 nuphus-mcp 进程内
        // 获取/释放，这里不重复获取（否则会与 MCP 进程的锁自锁）。
        let lock = crate::utils::automation_lock::AutomationLock::new();
        let _lock_guard = match lock.acquire(tool_name) {
            Ok(guard) => guard,
            Err(e) => return Ok(ToolResult::failure(e)),
        };

        let mut client_guard = self.browser_client.lock().await;

        // 如果浏览器客户端未初始化,尝试创建
        if client_guard.is_none() {
            match BrowserClient::new() {
                Ok(client) => {
                    *client_guard = Some(client);
                }
                Err(e) => {
                    return Ok(ToolResult::failure(format!(
                        "Browser automation unavailable: {}. Please install Google Chrome or Microsoft Edge.",
                        e
                    )));
                }
            }
        }

        // 获取可变引用
        let client = client_guard
            .as_mut()
            .ok_or("Browser client not available")?;

        // 确保浏览器已启动(有界面模式,用户可见)
        if let Err(e) = client.launch(false).await {
            return Ok(ToolResult::failure(format!(
                "Failed to launch browser: {}",
                e
            )));
        }

        // 执行具体工具（统一超时保护，防止 CDP 操作卡死）
        let timeout_secs: u64 = match tool_name {
            "browser_navigate" | "browser_back" | "browser_forward" => 30,
            "browser_exec" => 15, // internal 10s eval timeout + buffer
            "browser_wait_for" => {
                params
                    .get("timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5000)
                    / 1000
                    + 5
            }
            _ => 15,
        };

        match run_browser_op_with_reconnect(client, tool_name, params, timeout_secs).await {
            Ok(output) => {
                reconcile_external_cdp_url(client).await;
                Ok(ToolResult::success(output))
            }
            Err(e) => Ok(ToolResult::failure(e)),
        }
    }
}

/// 只自动重试的只读工具白名单：纯读操作、无页面状态副作用。
/// 写工具（click/type/exec/cookies_set/upload/new_tab/switch_tab/close 等）不自动
/// 重试——命令可能已达浏览器但响应丢失，重复下发会重复执行；navigate 也不在列
/// （重新导航可能重提交 POST 表单、摧毁进行中的页面状态）。
/// browser_current_url 目前无对应工具实现，预留在列（纯读语义，未来新增时即为正确分类）。
const READ_ONLY_BROWSER_TOOLS: &[&str] = &[
    "browser_snapshot",
    "browser_extract",
    "browser_list_tabs",
    "browser_cookies_get",
    "browser_current_url",
    "browser_screenshot",
    "browser_list_downloads",
];

fn is_read_only_browser_tool(name: &str) -> bool {
    READ_ONLY_BROWSER_TOOLS.contains(&name)
}

/// 执行单个浏览器操作（连接级自愈编排）。
///
/// - 快速失败且是连接类错误（receiver is gone / channel closed 等）→ 重连；
///   只读工具重试一次，写工具返回"操作可能已执行"错误不重复下发；
/// - 操作超时（Windows 下被杀的 Chrome 不产生快速错误，命令卡在半开 websocket 上）
///   → 活性探测 + 子进程双条件：连接活着（慢页面/事件洪峰）或 Chrome 进程仍在
///   （页面繁忙）→ 原样返回超时错误，绝不误杀慢但健康的浏览器；
///   仅当进程确实消失才重连（重试规则同上）；
/// - 业务错误（元素不存在/导航超时等）→ 原样返回，不重试。
async fn run_browser_op_with_reconnect(
    client: &mut BrowserClient,
    tool_name: &str,
    params: &serde_json::Value,
    timeout_secs: u64,
) -> Result<String, String> {
    let timeout = Duration::from_secs(timeout_secs);

    match tokio::time::timeout(timeout, run_browser_op(client, tool_name, params)).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) if BrowserClient::is_connection_error(&BrowserError::Execution(e.clone())) => {
            tracing::warn!("[browser] CDP connection failed ({}), reconnecting", e);
            client
                .reconnect()
                .await
                .map_err(|e| format!("Browser reconnect failed: {}", e))?;
            if !is_read_only_browser_tool(tool_name) {
                // 写操作可能已在连接断开前执行——浏览器已恢复健康，但重复下发会重复执行。
                return Err(format!(
                    "Browser '{}' failed with a dead CDP connection ({}); the browser was \
                     restarted, but the operation may have already executed before the connection \
                     dropped — 请核实页面状态后再手动重试（写操作不自动重试）.",
                    tool_name, e
                ));
            }
            tokio::time::timeout(timeout, run_browser_op(client, tool_name, params))
                .await
                .map_err(|_| {
                    format!(
                        "Browser '{}' retry timed out after {}s",
                        tool_name, timeout_secs
                    )
                })?
                .map_err(|e| format!("Browser '{}' failed: {}", tool_name, e))
        }
        Ok(Err(e)) => Err(format!("Browser '{}' failed: {}", tool_name, e)),
        Err(_elapsed) => {
            // 操作超时：区分死连接与慢但健康的操作。探测失败不是死亡证明（CDP 事件洪峰
            // 会延迟探测响应，与 launch() 的契约一致）——误杀会摧毁全部标签页，因此需要
            // 第二个信号：Chromium 子进程确实退出。
            if client.is_connection_alive().await {
                return Err(format!(
                    "Browser '{}' timed out after {}s",
                    tool_name, timeout_secs
                ));
            }
            if client.child_process_alive() == Some(true) {
                return Err(format!(
                    "Browser '{}' timed out after {}s and the CDP connection is unresponsive, \
                     but the Chrome process is still alive (页面可能繁忙)；拒绝杀死存活浏览器，请稍后重试.",
                    tool_name, timeout_secs
                ));
            }
            tracing::warn!(
                "[browser] operation timed out after {}s, connection probe failed and the Chrome process is gone; reconnecting",
                timeout_secs
            );
            client
                .reconnect()
                .await
                .map_err(|e| format!("Browser reconnect failed: {}", e))?;
            if !is_read_only_browser_tool(tool_name) {
                return Err(format!(
                    "Browser '{}' timed out after {}s; the browser was restarted, \
                     but the operation may have already executed — 请核实页面状态后再手动重试（写操作不自动重试）.",
                    tool_name, timeout_secs
                ));
            }
            tokio::time::timeout(timeout, run_browser_op(client, tool_name, params))
                .await
                .map_err(|_| {
                    format!(
                        "Browser '{}' retry timed out after {}s",
                        tool_name, timeout_secs
                    )
                })?
                .map_err(|e| format!("Browser '{}' failed: {}", tool_name, e))
        }
    }
}

/// 单个浏览器工具的原子操作（无重连逻辑，由 `run_browser_op_with_reconnect` 编排）。
async fn run_browser_op(
    client: &mut BrowserClient,
    tool_name: &str,
    params: &serde_json::Value,
) -> Result<String, String> {
    match tool_name {
        "browser_navigate" => {
            let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let result = client.navigate(url).await.map_err(|e| e.to_string())?;
            // Auto snapshot after navigation
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(_) => Ok(result),
            }
        }
        "browser_snapshot" => {
            let full = params
                .get("full")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let selector = params.get("selector").and_then(|v| v.as_str());
            client
                .snapshot(full, selector)
                .await
                .map_err(|e| e.to_string())
        }
        "browser_exec" => {
            let script = params.get("script").and_then(|v| v.as_str()).unwrap_or("");
            if script.is_empty() {
                return Err("browser_exec: script parameter is required".to_string());
            }
            client.batch_exec(script).await.map_err(|e| e.to_string())
        }
        "browser_click" => {
            let selector = params
                .get("selector")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("ref").and_then(|v| v.as_str()))
                .unwrap_or("");
            let trusted = params
                .get("trusted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let result = if trusted {
                client
                    .click_trusted(selector)
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                client.click(selector).await.map_err(|e| e.to_string())?
            };
            // Auto snapshot after click
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(_) => Ok(result),
            }
        }
        "browser_type" => {
            let selector = params
                .get("selector")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("ref").and_then(|v| v.as_str()))
                .unwrap_or("");
            let text = params.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let result = client
                .type_text(selector, text)
                .await
                .map_err(|e| e.to_string())?;
            // Auto snapshot after type
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(_) => Ok(result),
            }
        }
        "browser_press" => {
            let key = params
                .get("key")
                .and_then(|v| v.as_str())
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| "browser_press: key parameter is required".to_string())?;
            let result = client.press_key(key).await.map_err(|e| e.to_string())?;
            let include_snapshot = params
                .get("snapshot")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !include_snapshot {
                return Ok(result);
            }
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(e) => Ok(format!(
                    "{}\n\n── Note: post-action snapshot failed; page state unavailable for the next step: {} ──",
                    result, e
                )),
            }
        }
        "browser_scroll" => {
            let direction = params
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("down");
            let amount = params.get("amount").and_then(|v| v.as_i64()).unwrap_or(500) as i32;
            let result = client
                .scroll(direction, amount)
                .await
                .map_err(|e| e.to_string())?;
            // Auto snapshot after scroll (new elements may have come into view)
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(_) => Ok(result),
            }
        }
        "browser_extract" => {
            let max_chars = params
                .get("max_chars")
                .and_then(|v| v.as_i64())
                .unwrap_or(8000) as usize;
            client.extract(max_chars).await.map_err(|e| e.to_string())
        }
        "browser_screenshot" => {
            let path = params.get("path").and_then(|v| v.as_str());
            client.screenshot(path).await.map_err(|e| e.to_string())
        }
        "browser_evaluate" => {
            let script = params.get("script").and_then(|v| v.as_str()).unwrap_or("");
            let value = client.evaluate(script).await.map_err(|e| e.to_string())?;
            // String 类型直接返回原始文本（避免 JSON 双重编码引号污染后续变量捕获）
            // 其他类型保留 JSON 序列化
            match &value {
                serde_json::Value::String(s) => Ok(s.clone()),
                _ => Ok(serde_json::to_string_pretty(&value).unwrap_or_default()),
            }
        }
        "browser_back" => {
            let result = client.back().await.map_err(|e| e.to_string())?;
            // Auto snapshot after navigation
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(_) => Ok(result),
            }
        }
        "browser_forward" => {
            let result = client.forward().await.map_err(|e| e.to_string())?;
            // Auto snapshot after navigation
            match client.snapshot(false, None).await {
                Ok(snap) => Ok(format!("{}\n\n── Page state ──\n{}", result, snap)),
                Err(_) => Ok(result),
            }
        }
        "browser_wait_for" => {
            let selector = params
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let timeout_ms = params
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000);
            let state = params
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("attached");
            client
                .wait_for(selector, timeout_ms, state)
                .await
                .map_err(|e| e.to_string())
        }
        "browser_cookies_get" => {
            let cookies = client.cookies_get().await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_string_pretty(&cookies).unwrap_or_default())
        }
        "browser_cookies_set" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let value = params.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let domain = params.get("domain").and_then(|v| v.as_str());
            let path = params.get("path").and_then(|v| v.as_str());
            client
                .cookies_set(name, value, domain, path)
                .await
                .map_err(|e| e.to_string())
        }
        "browser_import_cookies" => {
            let domain = params.get("domain").and_then(|v| v.as_str());
            client
                .import_cookies(domain)
                .await
                .map_err(|e| e.to_string())
        }
        "browser_upload_file" => {
            let selector = params
                .get("selector")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let file_path = params
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if selector.is_empty() || file_path.is_empty() {
                return Err("browser_upload_file: selector and file_path are required".to_string());
            }
            client
                .upload_file(selector, file_path)
                .await
                .map_err(|e| e.to_string())
        }
        "browser_drag_files" => {
            let file_paths: Vec<String> = params
                .get("file_paths")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if file_paths.is_empty() {
                return Err(
                    "browser_drag_files: file_paths must contain at least one absolute path"
                        .to_string(),
                );
            }
            let selector = params
                .get("selector")
                .and_then(|v| v.as_str())
                .or_else(|| params.get("ref").and_then(|v| v.as_str()))
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| "browser_drag_files: selector or ref is required".to_string())?;
            client
                .drag_files(selector, &file_paths)
                .await
                .map_err(|e| e.to_string())
        }
        "browser_list_downloads" => client.list_downloads().map_err(|e| e.to_string()),
        "browser_new_tab" => {
            let url = params.get("url").and_then(|v| v.as_str());
            client.new_tab(url).await.map_err(|e| e.to_string())
        }
        "browser_list_tabs" => {
            let tabs = client.list_tabs().await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_string_pretty(&tabs).unwrap_or_default())
        }
        "browser_switch_tab" => {
            let index = params.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            client.switch_tab(index).await.map_err(|e| e.to_string())
        }
        "browser_close" => {
            client.close().await.map_err(|e| e.to_string())?;
            Ok("Browser closed".to_string())
        }
        _ => Err(format!("Unknown browser tool: {}", tool_name)),
    }
}
