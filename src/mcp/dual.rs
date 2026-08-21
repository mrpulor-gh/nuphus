//! 双通道（dogfooding）：Nuphus 自身通过 MCP 通道调用 nuphus-mcp server。
//!
//! 设计意图：desktop_*/browser_* 工具调用**优先走 MCP**（若 nuphus-mcp server
//! 已配置或可自动发现），失败回退直连执行器。Nuphus 每次使用桌面/浏览器能力
//! 都在真实验证 MCP 层（dogfooding）。
//!
//! 回退策略（防双重执行，安全优先）：
//! - server 未配置 / 无法连接 / 传输错误 / JSON-RPC error → 回退直连（可用性优先）
//! - MCP 返回语义失败（`isError: true`，即 server 已执行但操作失败）：
//!   - 读工具（`desktop_screen_size` 等）→ 回退直连（直连实现可能成功）
//!   - 写工具（click/type/input 等）→ **不回退**，返回 MCP 结果（避免双重执行副作用）
//!
//! 关闭开关：环境变量 `NUPHUS_MCP_DUAL=off` 可整体禁用双通道（总是直连）。

use super::client::call_tool_with_config;
use super::config::ServerConfig;
use crate::ToolResult;
use serde_json::Value;
use std::collections::HashMap;

/// nuphus-mcp server 在 servers.yaml 中的 key（双通道专用）。
pub const NUPHUS_MCP_SERVER: &str = "nuphus-mcp";

/// 双通道总开关：`NUPHUS_MCP_DUAL=off` 禁用（总是直连）。
fn dual_enabled() -> bool {
    std::env::var("NUPHUS_MCP_DUAL").as_deref() != Ok("off")
}

/// 获取 nuphus-mcp server 配置：servers.yaml 显式配置优先，
/// 否则自动发现同 target 目录下的 nuphus-mcp 二进制（dev/test 场景）。
///
/// 外部浏览器偏好传播：用户在浏览器设置页配置的 CDP 端点通过 env 注入子进程
/// （MCP 通道与直连通道行为一致）。tri-state：None 未配置过 → 不动 yaml 既有
/// env；Some("") 明确回内置 → 剥离该 env；Some(url) → 注入覆盖。
pub fn nuphus_mcp_config() -> Option<ServerConfig> {
    if !dual_enabled() {
        return None;
    }
    // 1. servers.yaml 显式配置
    let mut sc = if let Ok(cfg) = super::config::load_config() {
        cfg.servers.get(NUPHUS_MCP_SERVER).cloned()
    } else {
        None
    };
    // 2. 自动发现
    let mut sc = match sc.take() {
        Some(sc) => sc,
        None => discover_binary()?,
    };
    let prefs = crate::config::UserPreferences::load();
    match prefs.browser_cdp_url {
        Some(url) if !url.is_empty() => {
            sc.env.insert("NUPHUS_MCP_BROWSER_CDP_URL".to_string(), url);
            // Identity envs power the child's attach self-healing. Present with
            // identity → inject; without (legacy URL-only config) → strip stale.
            match prefs.browser_identity {
                Some(id) => {
                    sc.env.insert("NUPHUS_BROWSER_NAME".to_string(), id.name);
                    sc.env
                        .insert("NUPHUS_BROWSER_EXE_PATH".to_string(), id.exe_path);
                    match id.user_data_dir {
                        Some(dir) => {
                            sc.env
                                .insert("NUPHUS_BROWSER_USER_DATA_DIR".to_string(), dir);
                        }
                        None => {
                            sc.env.remove("NUPHUS_BROWSER_USER_DATA_DIR");
                        }
                    }
                }
                None => {
                    sc.env.remove("NUPHUS_BROWSER_NAME");
                    sc.env.remove("NUPHUS_BROWSER_EXE_PATH");
                    sc.env.remove("NUPHUS_BROWSER_USER_DATA_DIR");
                }
            }
        }
        Some(_) => {
            sc.env.remove("NUPHUS_MCP_BROWSER_CDP_URL");
            sc.env.remove("NUPHUS_BROWSER_NAME");
            sc.env.remove("NUPHUS_BROWSER_EXE_PATH");
            sc.env.remove("NUPHUS_BROWSER_USER_DATA_DIR");
        }
        None => {}
    }
    Some(sc)
}

/// 自动发现 nuphus-mcp 可执行文件（与 Nuphus 同 workspace target 目录）。
fn discover_binary() -> Option<ServerConfig> {
    let candidates = candidate_dirs();
    let mut seen = std::collections::HashSet::new();
    for dir in candidates {
        for name in ["nuphus-mcp.exe", "nuphus-mcp"] {
            let path = dir.join(name);
            let key = path.to_string_lossy().to_string();
            if seen.insert(key.clone()) && path.is_file() {
                tracing::info!("[dual] Auto-discovered nuphus-mcp at {}", path.display());
                return Some(ServerConfig {
                    command: path.to_string_lossy().to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    timeout_ms: 60_000,
                    auto_start: false,
                });
            }
        }
    }
    None
}

/// 候选目录：当前可执行文件目录 → deps 父目录（测试）→ workspace target。
fn candidate_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            // cargo test 的可执行文件在 target/debug/deps/ 下，二进制在 target/debug/
            if let Some(parent) = dir.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    let root = crate::utils::workspace_root();
    for profile in ["debug", "release"] {
        dirs.push(root.join("target").join(profile));
    }
    dirs
}

/// 工具是否为"写操作"（变更系统/页面状态，双重执行有副作用）。
/// `desktop_mouse` 依 action 区分：position 为读，其余为写。
pub fn is_write_call(name: &str, args: &Value) -> bool {
    if name == "desktop_mouse" {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("position");
        return action != "position";
    }
    matches!(
        name,
        // desktop 写
        "desktop_mouse" | "desktop_mouse_drag" | "desktop_input" | "desktop_window_activate"
            | "desktop_window_screenshot" | "desktop_screenshot" | "desktop_clipboard_write"
            | "desktop_clipboard_clean"
            // browser 写
            | "browser_navigate" | "browser_click" | "browser_type" | "browser_exec"
            | "browser_scroll" | "browser_screenshot" | "browser_close" | "browser_evaluate"
            | "browser_back" | "browser_forward" | "browser_cookies_set"
            | "browser_import_cookies" | "browser_upload_file" | "browser_new_tab"
            | "browser_switch_tab" | "browser_press" | "browser_drag_files"
    )
}

/// 从 MCP tools/call 响应中提取 (文本, isError)。
fn extract_tool_result(response: &Value) -> Result<(String, bool), String> {
    let result = response
        .get("result")
        .ok_or_else(|| "MCP response missing 'result'".to_string())?;
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let text = content
        .iter()
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((text, is_error))
}

/// 双通道路由结果。
pub enum RouteOutcome {
    /// MCP 通道已处理（含写工具语义失败），调用方直接返回该结果。
    Handled(ToolResult),
    /// MCP 不可用（未配置 / 传输错误 / 读工具语义失败），调用方应走直连。
    Fallback(String),
}

/// MCP 优先路由：desktop_*/browser_* 工具经 nuphus-mcp 执行。
/// 返回 `Handled` 表示已由 MCP 通道处理；`Fallback(reason)` 表示应回退直连。
pub async fn route_tool(tool_name: &str, args: &Value) -> RouteOutcome {
    let cfg = match nuphus_mcp_config() {
        Some(cfg) => cfg,
        None => {
            return RouteOutcome::Fallback(
                "nuphus-mcp not configured/auto-discovered (direct channel)".to_string(),
            );
        }
    };

    let is_write = is_write_call(tool_name, args);

    match call_tool_with_config(NUPHUS_MCP_SERVER, cfg, tool_name, args.clone(), 60_000).await {
        Ok(response) => match extract_tool_result(&response) {
            Ok((text, false)) => {
                tracing::info!("[dual] MCP channel OK: {} (len={})", tool_name, text.len());
                RouteOutcome::Handled(ToolResult::success(text))
            }
            Ok((text, true)) => {
                if is_write {
                    // 写工具语义失败：server 已执行，不回退（防双重执行）
                    tracing::warn!(
                        "[dual] MCP write tool '{}' semantic error, NOT falling back: {}",
                        tool_name,
                        text
                    );
                    RouteOutcome::Handled(ToolResult::failure(text))
                } else {
                    tracing::warn!(
                        "[dual] MCP read tool '{}' semantic error, fallback direct: {}",
                        tool_name,
                        text
                    );
                    RouteOutcome::Fallback(format!("MCP semantic error: {}", text))
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[dual] MCP response parse failed for '{}': {}, fallback direct",
                    tool_name,
                    e
                );
                RouteOutcome::Fallback(e)
            }
        },
        Err(e) => {
            tracing::warn!(
                "[dual] MCP call '{}' failed ({}), fallback direct",
                tool_name,
                e
            );
            RouteOutcome::Fallback(format!("MCP transport error: {}", e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_classifier_detects_write_tools() {
        assert!(is_write_call(
            "desktop_input",
            &json!({"mode":"type","hwnd":1})
        ));
        assert!(is_write_call(
            "desktop_mouse",
            &json!({"action":"click","x":1,"y":2})
        ));
        assert!(!is_write_call(
            "desktop_mouse",
            &json!({"action":"position"})
        ));
        assert!(is_write_call("browser_click", &json!({"selector":"#a"})));
        assert!(!is_write_call("browser_snapshot", &json!({})));
        assert!(!is_write_call("desktop_screen_size", &json!({})));
        assert!(!is_write_call("desktop_windows_list", &json!({})));
        assert!(!is_write_call("browser_list_tabs", &json!({})));
        assert!(!is_write_call("desktop_mouse", &json!({}))); // 缺 action → position（读）
    }

    #[test]
    fn extract_result_parses_text_and_is_error() {
        let ok = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "content": [{"type":"text","text":"hello"}], "isError": false }
        });
        let (text, is_err) = extract_tool_result(&ok).expect("parse ok");
        assert_eq!(text, "hello");
        assert!(!is_err);

        let err = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": { "content": [{"type":"text","text":"boom"}], "isError": true }
        });
        let (text, is_err) = extract_tool_result(&err).expect("parse err");
        assert_eq!(text, "boom");
        assert!(is_err);
    }

    #[test]
    fn discovery_finds_binary_in_workspace_target() {
        // 当前环境（workspace root target/debug）应能发现 nuphus-mcp 二进制。
        // 该测试在 nuphus-mcp 构建完成后才有意义；构建缺失时返回 None 不算失败。
        let cfg = nuphus_mcp_config();
        if std::env::var("NUPHUS_MCP_DUAL").as_deref() == Ok("off") {
            assert!(cfg.is_none());
        } else {
            // 不强制存在（CI 可能未构建），仅验证候选目录逻辑不 panic
            let dirs = candidate_dirs();
            assert!(!dirs.is_empty());
        }
    }

    /// 真实双通道链路（需先 `cargo build -p nuphus-mcp`）：
    /// 运行：`cargo build -p nuphus-mcp && cargo test -p nuphus --lib mcp::dual::tests::e2e_dogfood_screen_size -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "requires nuphus-mcp binary built"]
    async fn e2e_dogfood_screen_size() {
        let cfg = nuphus_mcp_config()
            .expect("nuphus-mcp binary must be built (cargo build -p nuphus-mcp)");
        tracing::info!("[dual-e2e] using server: {}", cfg.command);

        let resp = call_tool_with_config(
            NUPHUS_MCP_SERVER,
            cfg,
            "desktop_screen_size",
            json!({}),
            30_000,
        )
        .await
        .expect("MCP call must succeed");

        let (text, is_err) = extract_tool_result(&resp).expect("parse result");
        assert!(!is_err, "screen_size must not be an error: {}", text);
        let parsed: Value = serde_json::from_str(&text).expect("screen_size returns JSON");
        let w = parsed["width"].as_u64().expect("width");
        let h = parsed["height"].as_u64().expect("height");
        assert!(w > 0 && h > 0, "screen size must be positive: {}x{}", w, h);
        tracing::info!("[dual-e2e] desktop_screen_size via MCP = {}x{}", w, h);
    }
}
