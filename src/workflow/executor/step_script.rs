//! 脚本步骤执行
use super::*;

impl Executor {
    /// 执行脚本步骤（内联代码写入临时文件，调用对应 runtime 执行，带超时保护）
    pub(super) async fn execute_script_step(
        &self,
        step: &Step,
        script: &ScriptDef,
        variables: &mut HashMap<String, serde_json::Value>,
    ) -> crate::Result<String> {
        use std::process::Command;

        let interpreter = match script.runtime.as_str() {
            "python" => "python",
            "node" => "node",
            "ahk" => "AutoHotkey.exe",
            "pwsh" => "pwsh",
            other => {
                return Err(crate::NuphusError::agent(format!(
                    "不支持的 runtime: {}",
                    other
                )))
            }
        };

        let ext = match script.runtime.as_str() {
            "python" => "py",
            "node" => "js",
            "ahk" => "ahk",
            _ => "ps1",
        };

        let tmp_path =
            std::env::temp_dir().join(format!("nuphus_script_{}.{}", uuid::Uuid::new_v4(), ext));
        // 对 code 做 {{var}} 变量替换后再写入临时文件
        let resolved_code = super::variables::resolve_vars_str(&script.code, variables);
        std::fs::write(&tmp_path, &resolved_code)
            .map_err(|e| crate::NuphusError::agent(format!("写入临时脚本失败: {}", e)))?;

        const SCRIPT_TIMEOUT_SECS: u64 = 120;
        let (tx, rx) = std::sync::mpsc::channel();
        let interpreter = interpreter.to_string();
        let tmp_path_clone = tmp_path.clone();
        let cwd = script.cwd.clone();
        std::thread::spawn(move || {
            let mut cmd = Command::new(&interpreter);
            cmd.arg(&tmp_path_clone);
            if let Some(ref dir) = cwd {
                cmd.current_dir(dir);
            }
            let result = cmd.output();
            let _ = tx.send(result);
        });

        // 带超时等待
        let output_result = rx
            .recv_timeout(std::time::Duration::from_secs(SCRIPT_TIMEOUT_SECS))
            .map_err(|_| {
                crate::NuphusError::agent(format!("脚本执行超时 ({}s)", SCRIPT_TIMEOUT_SECS))
            });

        // 清理临时文件
        let _ = std::fs::remove_file(&tmp_path);

        match output_result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                if !output.status.success() {
                    return Err(crate::NuphusError::agent(format!(
                        "脚本失败 (exit {}):\nstdout: {}\nstderr: {}",
                        output.status.code().unwrap_or(-1),
                        stdout,
                        stderr,
                    )));
                }

                let out = if stdout.is_empty() { stderr } else { stdout };
                super::variables::capture_output(&step.capture, &out, variables)?;
                Ok(out)
            }
            Ok(Err(e)) => Err(crate::NuphusError::agent(format!(
                "脚本进程启动失败: {}",
                e
            ))),
            Err(e) => Err(crate::NuphusError::agent(format!("脚本执行异常: {}", e))),
        }
    }
}
