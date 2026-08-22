//! SubTask shell streaming execution — independent module, no SubTaskRunner state
//!
//! Extracted from sub_task.rs, keeping original method signatures (all free functions/static methods).

use crate::{
    agent::events::{EventEmitter, NuphusEvent},
    tools::ToolRegistry,
    ToolResult,
};

/// Flush buffered lines as ToolOutputLine events
pub fn flush_output_lines(
    emitter: &Arc<dyn EventEmitter>,
    call_id: &str,
    buf: &mut Vec<(String, bool)>,
) {
    if buf.is_empty() {
        return;
    }
    for (line, is_stderr) in buf.drain(..) {
        emitter.emit(NuphusEvent::ToolOutputLine {
            call_id: call_id.to_string(),
            line,
            is_stderr,
        });
    }
}

use std::io::BufRead;
use std::sync::Arc;

/// Execute system_shell with line-by-line streaming, pushing stdout/stderr to frontend in real-time
pub fn stream_shell_blocking(
    command: &str,
    timeout_secs: u64,
    call_id: &str,
    emitter: &Arc<dyn EventEmitter>,
) -> ToolResult {
    // ── Create piped process ──
    #[cfg(windows)]
    let child = {
        use std::os::windows::process::CommandExt;
        fn spawn_shell(cmd: &str) -> std::io::Result<std::process::Child> {
            let builder = |exe: &str| {
                // PowerShell 5.1 pipe output defaults to UTF-16LE, force UTF-8
                let final_cmd = if exe == "powershell.exe" {
                    format!(
                        "$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new();{}",
                        cmd
                    )
                } else {
                    cmd.to_string()
                };
                std::process::Command::new(exe)
                    .args(["-NoProfile", "-NonInteractive", "-Command", &final_cmd])
                    .creation_flags(0x08000000) // CREATE_NO_WINDOW
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
            };
            // Prefer pwsh (PowerShell 7), fallback to powershell.exe (Windows built-in 5.1)
            builder("pwsh").or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    builder("powershell.exe")
                } else {
                    Err(e)
                }
            })
        }
        spawn_shell(command)
    };

    #[cfg(not(windows))]
    let child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => return ToolResult::failure(format!("Failed to spawn shell: {}", e)),
    };

    // ── Channel: reader threads → main thread ──
    let (tx, rx) = std::sync::mpsc::channel::<(String, bool)>();

    // stdout reader thread
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let mut buf = Vec::new();
            while let Ok(n) = reader.read_until(b'\n', &mut buf) {
                if n == 0 {
                    break;
                }
                if buf.ends_with(b"\n") {
                    buf.pop();
                }
                if buf.ends_with(b"\r") {
                    buf.pop();
                }
                let line = String::from_utf8_lossy(&buf).to_string();
                if tx.send((line, false)).is_err() {
                    break;
                }
                buf.clear();
            }
        });
    }

    // stderr reader thread
    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stderr);
            let mut buf = Vec::new();
            while let Ok(n) = reader.read_until(b'\n', &mut buf) {
                if n == 0 {
                    break;
                }
                if buf.ends_with(b"\n") {
                    buf.pop();
                }
                if buf.ends_with(b"\r") {
                    buf.pop();
                }
                let line = String::from_utf8_lossy(&buf).to_string();
                if tx.send((line, true)).is_err() {
                    break;
                }
                buf.clear();
            }
        });
    }

    // ── Collect output (main thread) ──
    let mut full_stdout = String::new();
    let mut full_stderr = String::new();
    let mut line_buf: Vec<(String, bool)> = Vec::new();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            let _ = child.kill();
            flush_output_lines(emitter, call_id, &mut line_buf);
            return ToolResult {
                success: false,
                output: Some(full_stdout),
                error: Some(format!(
                    "命令超时 ({}s)，请增加 timeout 参数重试",
                    timeout_secs
                )),
                exit_code: None,
            };
        }
        let remaining = deadline - now;
        let poll_timeout = std::cmp::min(remaining, std::time::Duration::from_millis(50));

        match rx.recv_timeout(poll_timeout) {
            Ok((line, is_stderr)) => {
                if is_stderr {
                    full_stderr.push_str(&line);
                    full_stderr.push('\n');
                } else {
                    full_stdout.push_str(&line);
                    full_stdout.push('\n');
                }
                line_buf.push((line, is_stderr));

                if line_buf.len() >= 20 {
                    flush_output_lines(emitter, call_id, &mut line_buf);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !line_buf.is_empty() {
                    flush_output_lines(emitter, call_id, &mut line_buf);
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                flush_output_lines(emitter, call_id, &mut line_buf);
                let status = child.wait().ok();
                let success = status.map(|s| s.success()).unwrap_or(false);
                return if success {
                    ToolResult::success(full_stdout)
                } else {
                    ToolResult {
                        success: false,
                        output: Some(full_stdout),
                        error: Some(full_stderr),
                        exit_code: status.and_then(|s| s.code()),
                    }
                };
            }
        }
    }
}

/// Execute system_shell with line-by-line streaming push (async wrapper)
pub async fn execute_shell_streaming(
    call: &crate::ToolCall,
    emitter: &Arc<dyn EventEmitter>,
) -> ToolResult {
    let command = call
        .params
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timeout_secs = call
        .params
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(180);
    let call_id = call.id.clone();
    let emitter = emitter.clone();

    tracing::info!(
        command = %command.chars().take(100).collect::<String>(),
        timeout = timeout_secs,
        "shell execution start"
    );
    let shell_start = std::time::Instant::now();

    match tokio::task::spawn_blocking(move || {
        stream_shell_blocking(&command, timeout_secs, &call_id, &emitter)
    })
    .await
    {
        Ok(result) => {
            let elapsed = shell_start.elapsed().as_millis() as u64;
            tracing::info!(
                duration_ms = elapsed,
                success = result.success,
                "shell execution end"
            );
            result
        }
        Err(join_err) => {
            tracing::error!(error = %join_err, "shell execution panicked");
            ToolResult::failure(format!("shell streaming panicked: {}", join_err))
        }
    }
}

/// Execute tool only (no security check) — delegates to exec_tool.rs
pub async fn execute_tool_only(
    tools: &ToolRegistry,
    call: &crate::ToolCall,
    emitter: Option<&dyn crate::agent::events::EventEmitter>,
) -> ToolResult {
    crate::agent::exec_tool::execute_tool_only(tools, &call.tool, &call.params, None, emitter).await
}
