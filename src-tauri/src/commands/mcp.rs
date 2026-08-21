//! MCP 管理命令（只读视图）。
//!
//! 仅提供查看能力：已配置 server 列表 + 按需查询工具列表。
//! 增删改仍通过手编 `plugin/mcp/servers.yaml` 完成，本模块不提供编辑入口。

use serde_json::json;

/// 列出所有已配置的 MCP server。
/// 注意：**不返回 env 字段** —— env 可能含 API token，禁止暴露给前端。
#[tauri::command]
pub fn list_mcp_servers() -> Result<serde_json::Value, String> {
    let cfg = nuphus::mcp::config::load_config()?;
    let servers: Vec<serde_json::Value> = cfg
        .servers
        .into_iter()
        .map(|(key, server)| {
            json!({
                "key": key,
                "command": server.command,
                "args": server.args,
                "timeout_ms": server.timeout_ms,
                "auto_start": server.auto_start,
            })
        })
        .collect();
    Ok(json!({ "servers": servers }))
}

/// 查询某 MCP server 的工具列表（JSON-RPC method `tools/list`，不是 tools/call）。
/// 首次调用会懒启动 server 进程（可能涉及 npx 依赖下载），故用 60s 超时兜底。
/// 返回 tools/list 原始响应，前端解析 `.tools` 数组。
#[tauri::command]
pub async fn list_mcp_tools(server: String) -> Result<serde_json::Value, String> {
    let client_arc = nuphus::mcp::client::get_or_create(&server).await?;
    let mut client = client_arc.lock().await;
    let call = async { client.call("tools/list", serde_json::json!({})) };
    tokio::time::timeout(std::time::Duration::from_secs(60), call)
        .await
        .map_err(|_| format!("MCP server '{server}' tools/list timed out after 60s"))?
}
