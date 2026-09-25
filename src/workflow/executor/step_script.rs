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
        use tokio::process::Command;

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
        struct ScriptFile(std::path::PathBuf);
        impl Drop for ScriptFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _script_file = ScriptFile(tmp_path.clone());

        const SCRIPT_TIMEOUT_SECS: u64 = 120;
        let mut cmd = Command::new(interpreter);
        cmd.arg(&tmp_path).kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if let Some(ref dir) = script.cwd {
            cmd.current_dir(super::variables::resolve_vars_str(dir, variables));
        }

        // 带超时等待
        let timeout_secs = step.timeout_secs.unwrap_or(SCRIPT_TIMEOUT_SECS);
        let output_result =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
                .await
                .map_err(|_| {
                    crate::NuphusError::agent(format!("脚本执行超时 ({}s)", timeout_secs))
                });

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
