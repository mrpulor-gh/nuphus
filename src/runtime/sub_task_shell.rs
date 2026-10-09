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

use crate::tools::background_tasks as bg;
use std::io::BufRead;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 长任务阈值（秒）。
///
/// `timeout_secs` 超过此值的调用被视为长任务（Release 构建 / 全量测试 /
/// 大下载）：超时后**不终止**进程，因为杀掉会让已完成的编译产物与测试
/// 进度全部作废，下次从零重跑。
///
/// 未超过的按原行为杀掉止损 —— 卡死的短命令继续跑只会占资源。
///
/// 取 300s（5 分钟）的依据：普通命令的合理上限远低于此；而 `tool_timeout`
/// 给 system_shell 的桶是 1800s，真正需要"不杀"的长任务必然远超 300s。
const LONG_TASK_THRESHOLD_SECS: u64 = 300;

/// Execute system_shell with line-by-line streaming, pushing stdout/stderr to frontend in real-time
pub fn stream_shell_blocking(
    command: &str,
    timeout_secs: u64,
    call_id: &str,
    emitter: &Arc<dyn EventEmitter>,
    // 调用方显式要求保留在后台（取消时只解除等待，不杀进程树）
    background: bool,
    // 共享取消标志句柄（与宿主持有的是同一个实例）
    cancel_flag: Option<Arc<AtomicBool>>,
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

    // ── 登记进进程级后台任务清单：启动即登记。登记之前那一小段窗口里，谁都杀不了它 ──
    let registry = bg::global();
    let task_id = registry.next_task_id();
    let log_path = bg::open_log_file(&task_id);
    let task = bg::BackgroundTask {
        id: task_id.clone(),
        tool: "system_shell".to_string(),
        command: bg::truncate_command(command),
        pid: child.id(),
        started_at_ms: bg::now_ms(),
        retain: background,
        output_path: log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
    };
    registry.register(task.clone());

    // stdout / stderr 各一个**独立追加句柄**指向同一日志文件（File 不实现 Clone，
    // 共用一个句柄会让两个读者线程互抢文件偏移）
    let stdout_sink = log_path
        .as_deref()
        .and_then(|p| std::fs::File::options().append(true).open(p).ok())
        .map(|f| std::sync::Arc::new(std::sync::Mutex::new(f)));
    let stderr_sink = log_path
        .as_deref()
        .and_then(|p| std::fs::File::options().append(true).open(p).ok())
        .map(|f| std::sync::Arc::new(std::sync::Mutex::new(f)));

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
                if let Some(sink) = &stdout_sink {
                    if let Ok(mut f) = sink.lock() {
                        let _ = writeln!(f, "{}", line);
                    }
                }
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
                if let Some(sink) = &stderr_sink {
                    if let Ok(mut f) = sink.lock() {
                        let _ = writeln!(f, "{}", line);
                    }
                }
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
    // 超过这个时长的调用方意图是长任务（构建 / 全量测试 / 大下载），
    // 超时后不终止进程；短命令仍按原行为杀掉，避免卡死的命令泄漏。
    // 判据用调用方自己传的 timeout_secs：传 180（默认）是普通命令，
    // 传 600+ 说明调用方明知这是长任务。
    let is_long_task = timeout_secs > LONG_TASK_THRESHOLD_SECS;

    loop {
        let now = std::time::Instant::now();

        // 取消：**先解除等待**。本循环每轮最多阻塞 50ms，故取消响应上限就是 50ms。
        // 让子进程真正停下来的是下面的杀进程树，不是这里。
        if crate::tools::definitions::is_cancelled(&cancel_flag) {
            flush_output_lines(emitter, call_id, &mut line_buf);
            if background {
                // 显式保留：只解除等待，进程照跑
                registry.set_retain(&task.id, true);
                let notice = bg::CancellationNotice::retained(
                    command,
                    Some(task.pid),
                    task.output_path.clone(),
                );
                tracing::info!("[BG] stream shell 取消（后台保留）: pid={}", task.pid);
                return bg::cancelled_result(full_stdout, &notice);
            }
            let killed = bg::kill_process_tree(task.pid);
            registry.unregister(&task.id);
            bg::discard_log(&task);
            let notice = match &killed {
                Ok(()) => bg::CancellationNotice::terminated(command, Some(task.pid)),
                Err(e) => {
                    tracing::error!("[BG] 取消时终止进程树失败 pid={}: {e}", task.pid);
                    bg::CancellationNotice::wait_released_without_kill("system_shell")
                }
            };
            tracing::info!(
                "[BG] stream shell 取消（前台，已终止）: pid={} ok={}",
                task.pid,
                killed.is_ok()
            );
            return bg::cancelled_result(full_stdout, &notice);
        }

        if now >= deadline {
            flush_output_lines(emitter, call_id, &mut line_buf);
            if !is_long_task {
                // 短命令：杀掉止损。卡死的命令继续跑只会占资源。
                let _ = child.kill();
                registry.unregister(&task.id);
                bg::discard_log(&task);
                return ToolResult {
                    success: false,
                    output: Some(full_stdout),
                    error: Some(format!(
                        "命令超时 ({}s)，已终止。可增加 timeout 参数重试。",
                        timeout_secs
                    )),
                    exit_code: None,
                };
            }
            // 长任务：**不杀**。杀掉会让已完成的编译产物/测试进度全部作废，
            // 下次从零重跑 —— 一次 5-6 分钟的活被切成无数段，每段都从零开始。
            //
            // 安全性：`child` 是局部变量，函数返回时 drop，但 std Child 的 drop
            // **不会**终止进程；stdout/stderr 已 `take()` 移交给 reader 线程，
            // 管道不会关闭，reader 继续读到进程自然结束，无句柄泄漏、无僵尸。
            //
            // 登记为保留项：进程必须可被查询/结束，并且必须告诉 Agent「别重跑」。
            registry.set_retain(&task.id, true);
            return ToolResult {
                success: false,
                output: Some(full_stdout),
                error: Some(format!(
                    "命令已超过等待上限 ({}s)，**仍在后台继续运行**（未终止）。\n\
                     进程 PID {}（任务 {}）已登记为保留的后台任务。\n\
                     输出持续追加到：{}\n\
                     勿重复执行同一命令 —— 重复触发会让长任务从零重跑。",
                    timeout_secs,
                    task.pid,
                    task.id,
                    task.output_path.as_deref().unwrap_or("(未开启输出文件)")
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
                registry.unregister(&task.id);
                bg::discard_log(&task);
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
    cancel_flag: Option<Arc<AtomicBool>>,
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
    // background 缺省 false（前台，取消杀进程树）；保留必须显式声明
    let background = call
        .params
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let call_id = call.id.clone();
    let emitter = emitter.clone();

    tracing::info!(
        command = %command.chars().take(100).collect::<String>(),
        timeout = timeout_secs,
        background = background,
        "shell execution start"
    );
    let shell_start = std::time::Instant::now();

    match tokio::task::spawn_blocking(move || {
        stream_shell_blocking(
            &command,
            timeout_secs,
            &call_id,
            &emitter,
            background,
            cancel_flag,
        )
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
