//! MCP stdio client — JSON-RPC transport over child process pipes.
//!
//! Manages per-server connections via a global lazy-start pool.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tokio::sync::Mutex;

/// An MCP client managing one server process via stdio JSON-RPC.
pub struct McpClient {
    /// Server key from servers.yaml
    pub server_name: String,
    /// Child process handle
    process: Option<Child>,
    /// Incrementing JSON-RPC request id
    next_id: i64,
    /// Stdin writer (line-buffered)
    stdin_writer: Option<Box<dyn Write + Send>>,
    /// Stdout reader
    stdout_reader: Option<BufReader<std::process::ChildStdout>>,
}

impl McpClient {
    /// Start the server process and perform MCP initialize handshake.
    pub fn start(
        server_name: &str,
        command: &str,
        args: &[String],
        envs: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let mut child = spawn_with_cmd_fallback(command, args, envs).map_err(|e| {
            format!(
                "MCP server '{}' failed to start ({}): {}",
                server_name, command, e
            )
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("MCP server '{}': stdin not available", server_name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("MCP server '{}': stdout not available", server_name))?;

        // 消费 stderr：子进程写满 stderr 管道缓冲会阻塞卡死（如 npx 下载进度、
        // server 调试日志）。起后台线程持续读取，内容降为 debug 日志，不阻塞主流程。
        if let Some(stderr) = child.stderr.take() {
            let srv = server_name.to_string();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let reader = std::io::BufReader::new(stderr);
                for l in reader.lines().map_while(Result::ok) {
                    if !l.trim().is_empty() {
                        tracing::debug!("[mcp:{}] stderr: {}", srv, l);
                    }
                }
            });
        }

        let writer: Box<dyn Write + Send> = Box::new(stdin);
        let reader = BufReader::new(stdout);

        let mut client = McpClient {
            server_name: server_name.to_string(),
            process: Some(child),
            next_id: 1,
            stdin_writer: Some(writer),
            stdout_reader: Some(reader),
        };

        // Initialize handshake
        let init_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "nuphus",
                    "version": "0.1.0"
                }
            }
        });
        let _response = client.send_raw(&init_request)?;
        tracing::info!("[mcp] Server '{}' initialized successfully", server_name);

        // Send initialized notification (no response expected)
        let initialized = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        if let Some(ref mut w) = client.stdin_writer {
            let line = serde_json::to_string(&initialized)
                .map_err(|e| format!("JSON serialize error: {}", e))?;
            writeln!(w, "{}", line).map_err(|e| format!("MCP write error: {}", e))?;
            w.flush().map_err(|e| format!("MCP flush error: {}", e))?;
        }

        Ok(client)
    }

    /// Send a JSON-RPC request and return the parsed response.
    /// `method`: "tools/list", "tools/call", "ping", etc.
    pub fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        self.send_raw(&request)
    }

    /// Send a raw JSON value as request and read the JSON-RPC response.
    fn send_raw(&mut self, request: &serde_json::Value) -> Result<serde_json::Value, String> {
        let line =
            serde_json::to_string(request).map_err(|e| format!("JSON serialize error: {}", e))?;

        // Write request
        let writer = self
            .stdin_writer
            .as_mut()
            .ok_or_else(|| "MCP client: stdin not connected".to_string())?;
        writeln!(writer, "{}", line).map_err(|e| format!("MCP write error: {}", e))?;
        writer
            .flush()
            .map_err(|e| format!("MCP flush error: {}", e))?;

        // Read response (single line JSON)
        let reader = self
            .stdout_reader
            .as_mut()
            .ok_or_else(|| "MCP client: stdout not connected".to_string())?;
        let mut response_line = String::new();
        reader
            .read_line(&mut response_line)
            .map_err(|e| format!("MCP read error: {}", e))?;

        if response_line.trim().is_empty() {
            return Err("MCP server returned empty response".to_string());
        }

        let response: serde_json::Value = serde_json::from_str(&response_line).map_err(|e| {
            format!(
                "MCP JSON parse error: {} (raw: {})",
                e,
                &response_line.chars().take(300).collect::<String>()
            )
        })?;

        // Check for JSON-RPC error
        if let Some(err) = response.get("error") {
            return Err(format!(
                "MCP error: {}",
                serde_json::to_string_pretty(err).unwrap_or_else(|_| "unknown".to_string())
            ));
        }

        Ok(response)
    }

    /// Shut down the server process.
    pub fn shutdown(&mut self) {
        if let Some(ref mut child) = self.process {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.process = None;
        self.stdin_writer = None;
        self.stdout_reader = None;
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 启动 MCP 子进程（面向全体用户的通用兜底，不硬编码任何绝对路径）。
///
/// Windows 下 `Command::new("npx")` 直接 spawn 会报 "program not found"——因为 npx
/// 是 `.cmd` 批处理（无 npx.exe）。此处先直接 spawn，失败且错误为 NotFound 时，
/// 用 `cmd.exe /c` 包装重试（cmd 会用 PATHEXT 解析出 .cmd 并执行）。
/// 这样无论用户的 Node.js 装在哪、命令是 .exe 还是 .cmd，都能正确启动。
fn spawn_with_cmd_fallback(
    command: &str,
    args: &[String],
    envs: &HashMap<String, String>,
) -> std::io::Result<Child> {
    match try_spawn(command, args, envs) {
        Ok(child) => Ok(child),
        Err(e) => {
            #[cfg(windows)]
            {
                // 只有「找不到程序」才值得用 cmd 重试；权限/参数等其他错误直接返回。
                if e.kind() == std::io::ErrorKind::NotFound {
                    let mut cmd_args: Vec<String> = vec!["/c".to_string(), command.to_string()];
                    cmd_args.extend(args.iter().cloned());
                    try_spawn("cmd", &cmd_args, envs)
                } else {
                    Err(e)
                }
            }
            #[cfg(not(windows))]
            {
                Err(e)
            }
        }
    }
}

/// 构造并 spawn 一个 stdio-piped 子进程（stdin/stdout/stderr 均 piped）。
fn try_spawn(
    command: &str,
    args: &[String],
    envs: &HashMap<String, String>,
) -> std::io::Result<Child> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.spawn()
}

// ── Global connection pool ──

/// Server name → McpClient (Arc<Mutex<>> for shared access across executor steps).
type McpPool = HashMap<String, Arc<Mutex<McpClient>>>;

static MCP_POOL: std::sync::LazyLock<Arc<Mutex<McpPool>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

/// Get a reference to the global MCP client pool.
pub fn get_pool() -> Arc<Mutex<McpPool>> {
    MCP_POOL.clone()
}

/// Drop a pooled client, terminating its child process (see `McpClient::drop`).
/// The next call re-spawns the server with the then-current configuration —
/// used after browser CDP preference changes so the MCP child picks up the new
/// `NUPHUS_MCP_BROWSER_CDP_URL` env.
pub async fn drop_server(server_name: &str) {
    let pool = get_pool();
    let mut guard = pool.lock().await;
    if guard.remove(server_name).is_some() {
        tracing::info!(
            "[mcp] dropped pooled server '{}' (respawns on next use)",
            server_name
        );
    }
}

/// Get or create an McpClient for the given server from the global pool.
/// Uses the config from servers.yaml.
pub async fn get_or_create(server_name: &str) -> Result<Arc<Mutex<McpClient>>, String> {
    let cfg = super::config::load_config()?;
    let server_cfg = cfg.servers.get(server_name).cloned().ok_or_else(|| {
        format!(
            "MCP server '{}' not found in servers.yaml. Available: {:?}",
            server_name,
            cfg.servers.keys().collect::<Vec<_>>()
        )
    })?;
    get_or_create_with_config(server_name, server_cfg).await
}

/// Get or create an McpClient using an explicit [`ServerConfig`] (bypasses
/// servers.yaml lookup). Used by the dual-channel (dogfooding) path where the
/// nuphus-mcp binary is auto-discovered rather than pre-configured.
pub async fn get_or_create_with_config(
    server_name: &str,
    server_cfg: super::config::ServerConfig,
) -> Result<Arc<Mutex<McpClient>>, String> {
    let pool = get_pool();
    let mut guard = pool.lock().await;

    // Return existing client if already connected
    if let Some(client) = guard.get(server_name) {
        return Ok(client.clone());
    }

    let client = McpClient::start(
        server_name,
        &server_cfg.command,
        &server_cfg.args,
        &server_cfg.env,
    )?;

    let arc = Arc::new(Mutex::new(client));
    guard.insert(server_name.to_string(), arc.clone());
    tracing::info!("[mcp] Lazy-started server '{}' in pool", server_name);

    Ok(arc)
}

/// Call a tool on an MCP server, with automatic reconnect on failure (1 retry).
pub async fn call_tool(
    server_name: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    timeout_ms: u64,
) -> Result<serde_json::Value, String> {
    call_tool_with_config_opt(server_name, None, tool_name, arguments, timeout_ms).await
}

/// Call a tool with an explicit [`ServerConfig`] (dual-channel dogfooding path).
pub async fn call_tool_with_config(
    server_name: &str,
    server_cfg: super::config::ServerConfig,
    tool_name: &str,
    arguments: serde_json::Value,
    timeout_ms: u64,
) -> Result<serde_json::Value, String> {
    call_tool_with_config_opt(
        server_name,
        Some(server_cfg),
        tool_name,
        arguments,
        timeout_ms,
    )
    .await
}

/// Shared implementation: `server_cfg_opt = None` → servers.yaml lookup.
async fn call_tool_with_config_opt(
    server_name: &str,
    server_cfg_opt: Option<super::config::ServerConfig>,
    tool_name: &str,
    arguments: serde_json::Value,
    timeout_ms: u64,
) -> Result<serde_json::Value, String> {
    let client_arc = match &server_cfg_opt {
        Some(cfg) => get_or_create_with_config(server_name, cfg.clone()).await?,
        None => get_or_create(server_name).await?,
    };

    let call_future = async {
        let mut client = client_arc.lock().await;
        client.call(
            "tools/call",
            serde_json::json!({
                "name": tool_name,
                "arguments": arguments,
            }),
        )
    };

    let result = if timeout_ms > 0 {
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), call_future).await
        {
            Ok(r) => r,
            Err(_) => Err(format!(
                "MCP call '{}::{}' timed out after {}ms",
                server_name, tool_name, timeout_ms
            )),
        }
    } else {
        call_future.await
    };

    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            tracing::warn!(
                "[mcp] Call '{}::{}' failed: {}. Attempting reconnect...",
                server_name,
                tool_name,
                e
            );
            // Attempt reconnect: remove from pool and retry
            {
                let pool = get_pool();
                let mut guard = pool.lock().await;
                if let Some(old) = guard.remove(server_name) {
                    let mut client = old.lock().await;
                    client.shutdown();
                }
            }
            // Retry once
            let retry_arc = match server_cfg_opt {
                Some(ref cfg) => get_or_create_with_config(server_name, cfg.clone()).await?,
                None => get_or_create(server_name).await?,
            };
            let mut retry_client = retry_arc.lock().await;
            retry_client.call(
                "tools/call",
                serde_json::json!({
                    "name": tool_name,
                    "arguments": arguments,
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_initialization() {
        // Verify pool exists and is empty on start
        let pool = get_pool();
        // Can't easily test async in sync context, just verify no panic
        let _ = pool;
    }
}
