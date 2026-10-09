//! 系统工具定义
//!
//! 包含系统信息、环境变量、shell 执行、休眠等 ToolDef 注册方法。

use crate::permissions::ToolCategory;
use crate::tools::background_tasks as bg;
use crate::tools::registry::{ToolCtx, ToolDef, ToolRegistry};
use crate::ToolResult;
use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

impl ToolRegistry {
    pub(crate) fn register_system_info(&mut self) {
        self.register(ToolDef {
            name: "system_info".to_string(),
            description: "Get OS, CPU, memory, and disk info. Windows: pwsh.exe required for full details.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            category: ToolCategory::Core,
            executor: |_params, _ctx| {
                let mut info = serde_json::json!({
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                });

                #[cfg(target_os = "windows")]
                {

                    use std::os::windows::process::CommandExt;

                    // 用 PowerShell + Get-CimInstance(替代已废弃的 wmic)
                    fn ps(script: &str) -> Option<String> {
                        let wrapped = format!(
                            "$OutputEncoding = [Console]::OutputEncoding = \
                             [Text.UTF8Encoding]::new(); {}",
                            script
                        );
                        let output = std::process::Command::new("pwsh")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &wrapped])
                            .creation_flags(0x08000000)
                            .output()
                            .ok()?;
                        if output.status.success() {
                            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                        } else {
                            None
                        }
                    }

                    // CPU 名称
                    if let Some(cpu) = ps("(Get-CimInstance Win32_Processor).Name") {
                        if !cpu.is_empty() {
                            info["cpu"] = serde_json::Value::String(cpu);
                        }
                    }

                    // 内存
                    if let Some(mem_json) = ps("$os = Get-CimInstance Win32_OperatingSystem; [Math]::Round($os.TotalVisibleMemorySize / 1MB, 2).ToString() + ',' + [Math]::Round($os.FreePhysicalMemory / 1MB, 2).ToString()") {
                        let parts: Vec<&str> = mem_json.split(',').collect();
                        if parts.len() == 2 {
                            let total = parts[0].parse::<f64>().unwrap_or(0.0);
                            let free = parts[1].parse::<f64>().unwrap_or(0.0);
                            info["memory"] = serde_json::json!({
                                "total_gb": total,
                                "available_gb": free,
                                "used_gb": (total - free).max(0.0),
                            });
                        }
                    }

                    // 磁盘
                    if let Some(disk_text) = ps(
                        "Get-CimInstance Win32_LogicalDisk -Filter \"DriveType=3\" | ForEach-Object { \"$($_.DeviceID),$($_.FreeSpace),$($_.Size)\" }"
                    ) {
                        let mut disks = Vec::new();
                        for line in disk_text.lines() {
                            let parts: Vec<&str> = line.split(',').collect();
                            if parts.len() == 3 {
                                let drive = parts[0];
                                let free_bytes = parts[1].parse::<u64>().unwrap_or(0);
                                let total_bytes = parts[2].parse::<u64>().unwrap_or(0);
                                if total_bytes > 0 {
                                    disks.push(serde_json::json!({
                                        "drive": drive,
                                        "free_gb": (free_bytes as f64 / 1024.0 / 1024.0 / 1024.0 * 100.0).round() / 100.0,
                                        "total_gb": (total_bytes as f64 / 1024.0 / 1024.0 / 1024.0 * 100.0).round() / 100.0,
                                    }));
                                }
                            }
                        }
                        info["disks"] = serde_json::Value::Array(disks);
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    use std::process::Command;

                    fn sh_cmd(cmd: &str) -> Option<String> {
                        Command::new("sh").args(["-c", cmd]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
                    }

                    // CPU: Linux /proc/cpuinfo, macOS sysctl
                    let cpu = sh_cmd("cat /proc/cpuinfo 2>/dev/null | grep 'model name' | head -1 | cut -d: -f2")
                        .or_else(|| sh_cmd("sysctl -n machdep.cpu.brand_string 2>/dev/null"));
                    if let Some(c) = cpu { info["cpu"] = serde_json::Value::String(c); }

                    // Memory: Linux free, macOS vm_stat + sysctl
                    let mem = sh_cmd("free -m 2>/dev/null | awk '/^Mem:/ {print $2\",\"$7}'")
                        .or_else(|| {
                            let total = sh_cmd("sysctl -n hw.memsize 2>/dev/null")?.parse::<u64>().ok()?;
                            let pages = sh_cmd("vm_stat 2>/dev/null | awk '/Pages free:/ {print $3}'")?.trim_end_matches('.').parse::<u64>().ok()?;
                            let page_size = sh_cmd("sysctl -n hw.pagesize 2>/dev/null")?.parse::<u64>().ok()?;
                            let avail_mb = (pages * page_size) / (1024 * 1024);
                            let total_mb = total / (1024 * 1024);
                            Some(format!("{},{}", total_mb, avail_mb))
                        });
                    if let Some(m) = mem {
                        let parts: Vec<&str> = m.split(',').collect();
                        if parts.len() == 2 {
                            let total_mb = parts[0].parse::<u64>().unwrap_or(0);
                            let avail_mb = parts[1].parse::<u64>().unwrap_or(0);
                            info["memory"] = serde_json::json!({
                                "total_gb": (total_mb as f64 / 1024.0 * 100.0).round() / 100.0,
                                "available_gb": (avail_mb as f64 / 1024.0 * 100.0).round() / 100.0,
                                "used_gb": ((total_mb.saturating_sub(avail_mb)) as f64 / 1024.0 * 100.0).round() / 100.0,
                            });
                        }
                    }

                    // Disk: df works on both Linux and macOS
                    if let Some(d) = sh_cmd("df -B1 / 2>/dev/null | awk 'NR==2 {print $1\",\"$4\",\"$2}'") {
                        let parts: Vec<&str> = d.split(',').collect();
                        if parts.len() == 3 {
                            let free = parts[1].parse::<u64>().unwrap_or(0);
                            let total = parts[2].parse::<u64>().unwrap_or(0);
                            info["disks"] = serde_json::json!([{
                                "drive": parts[0],
                                "free_gb": (free as f64 / 1024.0 / 1024.0 / 1024.0 * 100.0).round() / 100.0,
                                "total_gb": (total as f64 / 1024.0 / 1024.0 / 1024.0 * 100.0).round() / 100.0,
                            }]);
                        }
                    }
                }

                Ok(ToolResult::success(
                    serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string())
                ))
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_system_env_get(&mut self) {
        self.register(ToolDef {
            name: "system_env_get".to_string(),
            description: "Read an environment variable".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Environment variable name" }
                },
                "required": ["key"]
            }),
            category: ToolCategory::Core,
            executor: |params, _ctx| {
                let key = params.get("key").and_then(|v| v.as_str()).unwrap_or("");
                if key.is_empty() {
                    return Ok(ToolResult::failure("key is required"));
                }
                match std::env::var(key) {
                    Ok(val) => Ok(ToolResult::success(format!("{}={}", key, val))),
                    Err(_) => Ok(ToolResult::success(format!("{} is not set", key))),
                }
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_execute_shell(&mut self) {
        self.register(ToolDef {
            name: "system_shell".to_string(),
            description: "Execute a shell command (PowerShell on Windows, sh on Unix). Default timeout 180s. Foreground commands (default) are killed with their process tree when the user cancels.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "PowerShell command on Windows, sh on Unix. On Windows use ; not &&" },
                    "timeout": { "type": "integer", "minimum": 1, "maximum": 1800, "default": 180, "description": "Timeout in seconds. On timeout the process is NOT killed — it keeps running, is registered as a retained background task (pid + output file are reported back); read its output file instead of re-running." },
                    "background": { "type": "boolean", "default": false, "description": "true = the whole process tree survives a user cancel (use only for long jobs whose progress must not be thrown away); false = default foreground, cancelled by killing the process tree." },
                    "cwd": { "type": "string", "description": "Working directory for the command" }
                },
                "required": ["command"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, ctx| {
                let request = ShellRequest::from_params(params);
                run_system_shell(&request, ctx)
            },
            depends_on: vec![],
        });
    }

    pub(crate) fn register_sleep(&mut self) {
        self.register(ToolDef {
            name: "system_sleep".to_string(),
            description: "Pause execution for N seconds (default 1, max 60)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "seconds": {
                        "type": "number",
                        "default": 1,
                        "minimum": 0,
                        "maximum": 60,
                        "description": "Seconds to wait (decimals OK, max 60)"
                    }
                }
            }),
            category: ToolCategory::Core,
            executor: |params, ctx| {
                let seconds = params
                    .get("seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                run_system_sleep(seconds, ctx)
            },
            depends_on: vec![],
        });
    }
}

// ============================================================================
// system_shell / system_sleep 的可中断执行体
// ============================================================================
//
// 为什么执行体要从 `ToolDef.executor` 闭包里搬出来：原实现把「起进程 + 阻塞等
// 结束」整坨塞在闭包里，child / pgid 从不外泄，于是**没有人能杀它**——cancel_flag
// 置位后 agent 只能干等（用户侧表现：点了中断毫无反应）。搬成自由函数是因为取消
// 路径需要三样东西在同一作用域内可见：子进程句柄、注册表条目、取消标志句柄。
//
// 取消语义（契约勿改）：
// - **先解除等待**：主循环每 150ms 查一次取消标志，命中即刻返回，agent 立刻脱身。
// - **再处置进程**：background=false（默认前台）→ 按 pid 杀进程树；
//   background=true（调用方显式声明保留）→ 保留并标记 retain，只解除等待。
// - **超时不杀**（既有对外契约不动）：进程保留 ⇒ 登记为 retain，并把 pid 与输出
//   文件位置一并告诉 Agent，禁止诱导重跑。
// - **取消必须返回结构化事实**（`__CANCELLED__` 前缀 + 明文），不能只回
//   “cancelled”——那会让 Agent 以为命令没跑过而重跑长任务。

/// 子进程状态轮询周期（与 registry 等待段的 `CANCEL_POLL_INTERVAL` 同档）
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// system_sleep 的可中断等待切片
const SLEEP_SLICE: Duration = Duration::from_millis(150);

/// CREATE_NO_WINDOW —— 不弹黑框（沿用既有 shell 启动写法）
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// shell 的入参。独立成 struct：默认值与分支都必须能被单测直接断言，
/// 不能只藏在闭包里靠人眼读。
#[derive(Debug, Clone, PartialEq)]
pub struct ShellRequest {
    pub command: String,
    pub timeout_secs: u64,
    /// 调用方**显式**要求保留在后台。默认 false = 前台，取消时杀进程树。
    pub background: bool,
    pub cwd: Option<String>,
}

impl ShellRequest {
    pub fn from_params(params: &serde_json::Value) -> Self {
        Self {
            command: params
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            timeout_secs: params
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(180),
            background: params
                .get("background")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            cwd: params
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }
    }
}

/// 主循环的三种结局
enum ShellOutcome {
    /// 进程自然结束（带退出码）
    Exited,
    /// 到达 timeout 上限（**什么都不杀**）
    Timeout,
    /// 取消被置位
    Cancelled,
}

/// 一个管道的排空线程：读到底，同时把已读内容 tee 进输出日志。
///
/// 为什么必须有它：子进程 stdout/stderr 是管道，**不排空就会写满缓冲区后死锁**。
/// 原实现靠 `Command::output()` 内部排空，改成手工 `spawn()` 后必须自己负责。
struct StreamDrain {
    buf: Arc<Mutex<Vec<u8>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StreamDrain {
    fn spawn<R>(stream: R, sink: Option<Arc<Mutex<std::fs::File>>>) -> Self
    where
        R: Read + Send + 'static,
    {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let out = buf.clone();
        let handle = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stream);
            let mut chunk = [0u8; 8192];
            let mut local: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        local.extend_from_slice(&chunk[..n]);
                        if let Some(sink) = &sink {
                            if let Ok(mut f) = sink.lock() {
                                let _ = f.write_all(&chunk[..n]);
                                let _ = f.flush();
                            }
                        }
                    }
                }
            }
            // 只在 EOF 时锁一次：读者线程之间无竞争，也不会和主循环抢锁
            if let Ok(mut b) = out.lock() {
                b.extend_from_slice(&local);
            }
        });
        Self {
            buf,
            handle: Some(handle),
        }
    }

    /// 等读取线程收尾并取回完整内容。
    ///
    /// 进程被保留（超时 / 后台）时**不调用** —— 那些线程会随进程自然结束而退出，
    /// 此时等待是白等。
    fn join(mut self) -> String {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        String::from_utf8_lossy(&self.locked_bytes()).to_string()
    }

    /// 不等待读取线程，只取「此刻已经读到」的内容（取消/超时后立即返回用）。
    fn snapshot(&self) -> String {
        String::from_utf8_lossy(&self.locked_bytes()).to_string()
    }

    fn locked_bytes(&self) -> Vec<u8> {
        self.buf.lock().map(|b| b.clone()).unwrap_or_default()
    }
}

/// 启动 shell 子进程（stdout/stderr 走管道）。
/// 打开一个「追加写」句柄用于 tee。文件不存在/打不开时返回 None ——
/// 只是少一个落点，不影响命令执行。
fn open_append_sink(path: Option<&std::path::Path>) -> Option<Arc<Mutex<std::fs::File>>> {
    let path = path?;
    std::fs::File::options()
        .append(true)
        .open(path)
        .ok()
        .map(|f| Arc::new(Mutex::new(f)))
}

///
/// Windows 优先 pwsh（PowerShell 7），NotFound 时回退 powershell.exe（内置 5.1）——
/// 5.1 的管道输出默认 UTF-16LE，必须显式压成 UTF-8，否则中文输出全是乱码。
fn spawn_shell_child(request: &ShellRequest) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        fn build(exe: &str, cmd: &str, cwd: Option<&str>) -> std::io::Result<Child> {
            let final_cmd = if exe == "powershell.exe" {
                format!(
                    "$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new();{}",
                    cmd
                )
            } else {
                cmd.to_string()
            };
            let mut proc = Command::new(exe);
            proc.args(["-NoProfile", "-NonInteractive", "-Command", &final_cmd])
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(dir) = cwd {
                proc.current_dir(dir);
            }
            proc.spawn()
        }

        match build("pwsh", &request.command, request.cwd.as_deref()) {
            Ok(child) => Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                build("powershell.exe", &request.command, request.cwd.as_deref())
            }
            Err(e) => Err(e),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut proc = Command::new("sh");
        proc.arg("-c")
            .arg(&request.command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(ref dir) = request.cwd {
            proc.current_dir(dir);
        }
        proc.spawn()
    }
}

/// 取消标志是否已置位。
///
/// 无取消面（`None`：纯库 / CLI 等未注入 registry 的上下文）时返回 false —— 退化
/// 为「不可取消」，**不得**当成已取消。
pub fn is_cancelled(flag: &Option<Arc<AtomicBool>>) -> bool {
    flag.as_ref()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
}

/// system_shell 执行体
pub fn run_system_shell(request: &ShellRequest, ctx: &ToolCtx) -> Result<ToolResult, String> {
    let registry = bg::global();
    let task_id = registry.next_task_id();

    // 输出日志先开好再起进程：进程一旦存活就必须有落点，否则「超时后去看输出」
    // 这条既有对外承诺拿不到落点（工具描述里本就写着 read its output file）。
    // 打不开日志不阻断执行 —— 命令能跑比有输出位置重要。
    let log_path = bg::open_log_file(&task_id);
    let output_path = log_path.as_ref().map(|p| p.to_string_lossy().to_string());
    // stdout / stderr 各开一个**独立追加句柄**指向同一文件：两个读者线程并发写，
    // 共享句柄会互抢文件偏移（std::fs::File 不实现 Clone，只能各开各的）。
    let stdout_sink = open_append_sink(log_path.as_deref());
    let stderr_sink = open_append_sink(log_path.as_deref());

    let mut child = match spawn_shell_child(request) {
        Ok(c) => c,
        Err(e) => {
            // 起进程失败 ⇒ 没有任何进程会往这个日志里写，空文件纯属垃圾
            // （正常/超时/取消三条路径都有归属，只有这条早期失败会漏）
            if let Some(ref p) = log_path {
                let _ = std::fs::remove_file(p);
            }
            return Err(format!("shell failed: {}", e));
        }
    };
    let pid = child.id();

    let task = bg::BackgroundTask {
        id: task_id.clone(),
        tool: "system_shell".to_string(),
        command: bg::truncate_command(&request.command),
        pid,
        started_at_ms: bg::now_ms(),
        retain: request.background,
        output_path: output_path.clone(),
    };
    // 启动即登记：登记之前的那一小段窗口里，谁都杀不了它
    registry.register(task.clone());

    let stdout = child
        .stdout
        .take()
        .map(|s| StreamDrain::spawn(s, stdout_sink));
    let stderr = child
        .stderr
        .take()
        .map(|s| StreamDrain::spawn(s, stderr_sink));

    let deadline = Instant::now() + Duration::from_secs(request.timeout_secs);
    let mut exit: Option<ExitStatus> = None;

    let outcome = loop {
        // 先看进程状态：已经结束就别再谈取消/超时（结束就是结束）
        match child.try_wait() {
            Ok(Some(status)) => {
                exit = Some(status);
                break ShellOutcome::Exited;
            }
            Ok(None) => {}
            Err(e) => {
                registry.unregister(&task.id);
                bg::discard_log(&task);
                return Err(format!("shell failed: {}", e));
            }
        }

        if is_cancelled(&ctx.cancel_flag) {
            break ShellOutcome::Cancelled;
        }
        if Instant::now() >= deadline {
            break ShellOutcome::Timeout;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    match outcome {
        ShellOutcome::Exited => {
            let status = exit.expect("Exited 分支必然带退出状态");
            let stdout_text = stdout.map(|d| d.join()).unwrap_or_default();
            let stderr_text = stderr.map(|d| d.join()).unwrap_or_default();
            registry.unregister(&task.id);
            bg::discard_log(&task);
            if status.success() {
                Ok(ToolResult::success(stdout_text))
            } else {
                Ok(ToolResult {
                    success: false,
                    output: Some(stdout_text),
                    error: Some(stderr_text),
                    exit_code: status.code(),
                })
            }
        }

        // 超时：**不杀**。进程继续跑 ⇒ 转为保留项，把 pid + 输出位置交给 Agent。
        ShellOutcome::Timeout => {
            registry.set_retain(&task.id, true);
            let produced = stderr.map(|d| d.snapshot()).unwrap_or_default();
            let produced = stdout.map(|d| d.snapshot()).unwrap_or(produced);
            let message = format!(
                "命令已超过等待上限 ({}s)，**仍在后台继续运行**（未终止）。\n\
                 进程 PID {}（任务 {}）已登记为保留的后台任务。\n\
                 输出持续追加到：{}\n\
                 **勿重复执行同一命令** —— 重复触发会让长任务从零重跑。",
                request.timeout_secs,
                pid,
                task.id,
                output_path.as_deref().unwrap_or("(未开启输出文件)")
            );
            tracing::info!("[BG] system_shell 超时转后台: pid={pid} id={}", task.id);
            Ok(ToolResult {
                success: false,
                output: if produced.trim().is_empty() {
                    None
                } else {
                    Some(produced)
                },
                error: Some(message),
                exit_code: None,
            })
        }

        ShellOutcome::Cancelled => {
            let produced = stderr.map(|d| d.snapshot()).unwrap_or_default();
            let produced = stdout.map(|d| d.snapshot()).unwrap_or(produced);
            if request.background {
                // 显式保留：只解除等待，进程照跑 —— 标记 retain 后留在清单里
                registry.set_retain(&task.id, true);
                let notice =
                    bg::CancellationNotice::retained(&request.command, Some(pid), output_path);
                tracing::info!(
                    "[BG] system_shell 取消（后台保留）: pid={pid} id={}",
                    task.id
                );
                return Ok(bg::cancelled_result(produced, &notice));
            }

            // 默认前台：杀进程树。这一步才是「真正生效」——只解除等待而不杀，
            // 那只是把卡住换成孤儿继续跑。
            let killed = bg::kill_process_tree(pid);
            registry.unregister(&task.id);
            bg::discard_log(&task);
            let notice = match &killed {
                Ok(()) => bg::CancellationNotice::terminated(&request.command, Some(pid)),
                Err(e) => {
                    tracing::error!("[BG] 取消时终止进程树失败 pid={pid}: {e}");
                    bg::CancellationNotice::wait_released_without_kill("system_shell")
                }
            };
            tracing::info!(
                "[BG] system_shell 取消（前台，已终止）: pid={pid} id={} ok={}",
                task.id,
                killed.is_ok()
            );
            Ok(bg::cancelled_result(produced, &notice))
        }
    }
}

/// system_sleep 的入参夹取：钳到 [0, 60]。
///
/// 独立成函数的原因：上界 60 秒**没法用真实等待来验证**（那会让单测多跑一分钟），
/// 把它变成纯函数后既能直接断言，`run_system_sleep` 也只留执行逻辑。
pub fn clamped_seconds(seconds: f64) -> f64 {
    seconds.clamp(0.0, 60.0)
}

/// system_sleep 执行体：切成小片等待，每片查一次取消标志。
///
/// 与 system_shell 共用同一套取消事实编码，让 Agent 在两类长等待上看到同一种口径。
pub fn run_system_sleep(seconds: f64, ctx: &ToolCtx) -> Result<ToolResult, String> {
    let capped = clamped_seconds(seconds);
    let millis = (capped * 1000.0) as u64;
    let deadline = Instant::now() + Duration::from_millis(millis);

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if is_cancelled(&ctx.cancel_flag) {
            let notice = bg::CancellationNotice::no_process(&format!("system_sleep {capped}s"));
            tracing::info!("[BG] system_sleep 被取消（等待 {capped}s，中止）");
            return Ok(bg::cancelled_result(String::new(), &notice));
        }
        std::thread::sleep(std::cmp::min(deadline - now, SLEEP_SLICE));
    }

    Ok(ToolResult::success(format!("Slept for {:.2}s", capped)))
}

// ============================================================================
// 单测
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn ctx_with(cancel: Option<Arc<AtomicBool>>) -> ToolCtx {
        ToolCtx {
            cancel_flag: cancel,
            ..ToolCtx::default()
        }
    }

    // ── 入参解析：默认值与分支 ──────────────────────────────────────────

    #[test]
    fn shell_request_defaults_to_foreground_and_180s() {
        let r = ShellRequest::from_params(&serde_json::json!({"command": "echo hi"}));
        assert_eq!(r.command, "echo hi");
        assert_eq!(r.timeout_secs, 180);
        assert!(
            !r.background,
            "background 默认必须是 false（前台）——保留必须显式声明"
        );
        assert_eq!(r.cwd, None);
    }

    #[test]
    fn shell_request_reads_background_true() {
        let r = ShellRequest::from_params(
            &serde_json::json!({"command": "yarn build", "background": true}),
        );
        assert!(r.background, "显式 background=true 必须被采纳");
    }

    #[test]
    fn shell_request_treats_non_boolean_background_as_false() {
        // 模型偶尔会传 "true" 字符串 / 1；只认真布尔，避免误留孤儿进程
        let r =
            ShellRequest::from_params(&serde_json::json!({"command": "x", "background": "true"}));
        assert!(!r.background);
    }

    #[test]
    fn shell_request_reads_timeout_and_cwd() {
        let r = ShellRequest::from_params(
            &serde_json::json!({"command": "x", "timeout": 600, "cwd": "C:/tmp"}),
        );
        assert_eq!(r.timeout_secs, 600);
        assert_eq!(r.cwd.as_deref(), Some("C:/tmp"));
    }

    #[test]
    fn shell_request_missing_command_is_empty_not_panic() {
        let r = ShellRequest::from_params(&serde_json::json!({}));
        assert_eq!(r.command, "");
        assert_eq!(r.timeout_secs, 180);
    }

    #[test]
    fn is_cancelled_false_without_cancel_surface() {
        assert!(!is_cancelled(&None), "无取消面 ≠ 已取消");
    }

    #[test]
    fn is_cancelled_reflects_shared_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        assert!(!is_cancelled(&Some(flag.clone())));
        flag.store(true, Ordering::SeqCst);
        assert!(is_cancelled(&Some(flag)));
    }

    // ── system_sleep 的取消分支 ─────────────────────────────────────────

    #[test]
    fn system_sleep_returns_immediately_when_already_cancelled() {
        let flag = Arc::new(AtomicBool::new(true));
        let started = Instant::now();
        let r = run_system_sleep(60.0, &ctx_with(Some(flag))).expect("sleep 不应报错");
        assert!(!r.success, "被取消必须以失败形态上抛");
        let err = r.error.expect("取消事实必须落在 error 字段");
        let notice = bg::decode_cancellation(&err).expect("必须是可解码的取消事实");
        assert!(!notice.still_running());
        assert_eq!(notice.pid, None, "sleep 不启动任何子进程");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "已置位取消时必须立即返回，不能睡满 60s"
        );
    }

    #[test]
    fn system_sleep_normal_path_still_reports_duration() {
        let r = run_system_sleep(0.2, &ctx_with(None)).expect("sleep 不应报错");
        assert!(r.success);
        assert_eq!(r.output.as_deref(), Some("Slept for 0.20s"));
    }

    #[test]
    fn system_sleep_clamps_out_of_range_input() {
        // 上界用纯函数断言：真去睡 60s 会白白拖慢整个测试套件一分钟
        assert_eq!(clamped_seconds(999.0), 60.0);
        assert_eq!(clamped_seconds(-5.0), 0.0);
        assert_eq!(clamped_seconds(0.2), 0.2);

        // 端到端只验下界（秒级以内）
        let lo = run_system_sleep(-5.0, &ctx_with(None)).expect("不应报错");
        assert_eq!(lo.output.as_deref(), Some("Slept for 0.00s"));
    }

    // ── system_shell：三种结局 ──────────────────────────────────────────

    /// 跨平台的「会睡很久」命令
    fn long_sleep_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 6".to_string()
        } else {
            "sleep 6".to_string()
        }
    }

    fn request(command: &str, background: bool, timeout_secs: u64) -> ShellRequest {
        ShellRequest {
            command: command.to_string(),
            timeout_secs,
            background,
            cwd: None,
        }
    }

    /// 在 delay 之后置位取消标志
    fn cancel_after(delay: Duration, flag: Arc<AtomicBool>) {
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            flag.store(true, Ordering::SeqCst);
        });
    }

    #[test]
    fn system_shell_normal_completion_returns_stdout() {
        let r = run_system_shell(&request("echo nuphus-ok", false, 60), &ctx_with(None))
            .expect("正常命令不应报错");
        assert!(r.success, "正常命令必须成功: {:?}", r.error);
        assert!(
            r.output.as_deref().unwrap_or("").contains("nuphus-ok"),
            "必须回传 stdout: {:?}",
            r.output
        );
    }

    /// 结局一：前台命令被取消 ⇒ **进程树真的被杀**，且事实里必须带 pid。
    ///
    /// 这是本批的核心语义：只解除等待而不杀进程，等于把「卡住」换成「孤儿继续跑」。
    #[test]
    fn system_shell_foreground_kills_process_tree_on_cancel() {
        let flag = Arc::new(AtomicBool::new(false));
        cancel_after(Duration::from_millis(800), flag.clone());
        let started = Instant::now();

        let r = run_system_shell(
            &request(&long_sleep_command(), false, 120),
            &ctx_with(Some(flag)),
        )
        .expect("取消不应报错");

        assert!(
            !r.success,
            "取消必须以失败形态上抛，否则模型会以为命令跑完了"
        );
        let notice = bg::decode_cancellation(r.error.as_deref().expect("取消事实"))
            .expect("必须是本约定的取消事实");
        assert!(
            !notice.still_running(),
            "前台命令取消后不得谎称仍在后台跑: {}",
            notice.message
        );
        let pid = notice.pid.expect("前台终止必须给出 pid");
        assert!(
            !bg::is_process_alive(pid),
            "PID {pid} 必须真的消失（taskkill /T /F），不能只是解除等待"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "取消必须立即脱身，实测 {:?}",
            started.elapsed()
        );
    }

    /// 结局二：`background=true` ⇒ 只解除等待，进程照跑，并如实标记为保留。
    #[test]
    fn system_shell_background_survives_cancel_and_is_marked_retained() {
        let flag = Arc::new(AtomicBool::new(false));
        cancel_after(Duration::from_millis(800), flag.clone());

        let r = run_system_shell(
            &request(&long_sleep_command(), true, 120),
            &ctx_with(Some(flag)),
        )
        .expect("取消不应报错");

        let notice = bg::decode_cancellation(r.error.as_deref().expect("取消事实"))
            .expect("必须是本约定的取消事实");
        assert!(
            notice.still_running(),
            "显式后台保留必须如实报告「仍在跑」: {}",
            notice.message
        );
        assert!(
            notice.message.contains("勿重复执行"),
            "必须劝阻重跑，否则长任务会被跑两遍: {}",
            notice.message
        );
        let pid = notice.pid.expect("保留项必须给出 pid");
        assert!(bg::is_process_alive(pid), "保留项进程必须还活着");

        let entry = bg::global()
            .list()
            .into_iter()
            .find(|t| t.pid == pid)
            .expect("保留项必须留在后台清单里");
        assert!(entry.retain, "取消后条目必须标记为 retain（不许被自动杀）");

        // 收尾：显式结束它，别把子进程留在测试机上
        let _ = bg::global().kill_one(&entry.id);
    }

    /// 结局三：超时**不杀**（既有对外契约），但必须登记为保留项并交出 pid/输出位置。
    #[test]
    fn system_shell_timeout_keeps_process_and_reports_pid_and_output() {
        let r = run_system_shell(&request(&long_sleep_command(), false, 1), &ctx_with(None))
            .expect("超时不报错");

        assert!(!r.success);
        let err = r.error.clone().expect("超时必须有说明");
        assert!(
            err.contains("仍在后台继续运行"),
            "超时不杀是既有对外契约，文案不得改口: {err}"
        );
        assert!(err.contains("PID"), "必须交出 pid: {err}");
        assert!(err.contains("bg-"), "必须交出任务 id: {err}");
        assert!(err.contains("勿重复执行"), "必须劝阻重跑: {err}");

        // 该进程被保留 ⇒ 必须在清单里且标记 retain（命令 6s 后自退，届时自动清理）
        let entry = bg::global()
            .list()
            .into_iter()
            .filter(|t| t.command.contains("Start-Sleep") || t.command.contains("sleep"))
            .max_by_key(|t| t.started_at_ms);
        if let Some(entry) = entry {
            assert!(entry.retain, "超时保留的条目必须标记为 retain");
        }
    }
}
