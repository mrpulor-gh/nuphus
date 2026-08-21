//! 系统工具定义
//!
//! 包含系统信息、环境变量、shell 执行、休眠等 ToolDef 注册方法。

use crate::permissions::ToolCategory;
use crate::tools::registry::{ToolDef, ToolRegistry};
use crate::ToolResult;

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
            description: "Execute a shell command (PowerShell on Windows, sh on Unix). Default timeout 180s.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "PowerShell command on Windows, sh on Unix. On Windows use ; not &&" },
                     "timeout": { "type": "integer", "minimum": 1, "maximum": 600, "default": 180, "description": "Timeout in seconds" },
                    "cwd": { "type": "string", "description": "Working directory for the command" }
                },
                "required": ["command"]
            }),
            category: ToolCategory::SystemAutomation,
            executor: |params, _ctx| {
                let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let timeout_secs = params.get("timeout").and_then(|v| v.as_u64()).unwrap_or(180);
                let cwd = params.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());

                let (tx, rx) = std::sync::mpsc::channel();
                let cmd_owned = command.to_string();
                std::thread::spawn(move || {
                    #[cfg(windows)]
                    let result = {
                        use std::os::windows::process::CommandExt;
                        fn try_shell(exe: &str, cmd: &str, cwd: Option<&str>) -> std::io::Result<std::process::Output> {
                            // PowerShell 5.1 管道输出默认 UTF-16LE，需强制 UTF-8
                            let final_cmd = if exe == "powershell.exe" {
                                format!(
                                    "$OutputEncoding=[Console]::OutputEncoding=[Text.UTF8Encoding]::new();{}",
                                    cmd
                                )
                            } else {
                                cmd.to_string()
                            };
                            let mut proc = std::process::Command::new(exe);
                            proc.args(["-NoProfile", "-NonInteractive", "-Command", &final_cmd])
                                .creation_flags(0x08000000);
                            if let Some(dir) = cwd {
                                proc.current_dir(dir);
                            }
                            proc.output()
                        }
                        // 优先 pwsh (PowerShell 7)，回退到 powershell.exe (Windows 内置 5.1)
                        match try_shell("pwsh", &cmd_owned, cwd.as_deref()) {
                            Ok(output) => Ok(output),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                try_shell("powershell.exe", &cmd_owned, cwd.as_deref())
                            }
                            Err(e) => Err(e),
                        }
                    };

                    #[cfg(not(windows))]
                    let result = {
                        let mut proc = std::process::Command::new("sh");
                        proc.arg("-c").arg(&cmd_owned);
                        if let Some(ref dir) = cwd {
                            proc.current_dir(dir);
                        }
                        proc.output()
                    };

                    let _ = tx.send(result);
                });

                match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
                    Ok(Ok(output)) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let exit_code = output.status.code();
                        if output.status.success() {
                            Ok(ToolResult::success(stdout))
                        } else {
                            Ok(ToolResult {
                                success: false,
                                output: Some(stdout),
                                error: Some(stderr),
                                exit_code,
                            })
                        }
                    }
                    Ok(Err(e)) => Err(format!("shell failed: {}", e)),
                    Err(_timeout) => Ok(ToolResult {
                        success: false,
                        output: None,
                        error: Some(format!("命令超时 ({}s)", timeout_secs)),
                        exit_code: None,
                    }),
                }
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
            executor: |params, _ctx| {
                let seconds = params
                    .get("seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0);
                let capped = seconds.clamp(0.0, 60.0);
                let millis = (capped * 1000.0) as u64;
                std::thread::sleep(std::time::Duration::from_millis(millis));
                Ok(ToolResult::success(format!("Slept for {:.2}s", capped)))
            },
            depends_on: vec![],
        });
    }
}
