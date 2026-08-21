//! Shell Hooks 模块
//!
//! Shell 生命周期钩子，允许外部脚本在 Agent 运行时被调用。
//!
//! ## 设计参考
//! 外部脚本可以挂载到生命周期事件上，无需写 Python 插件。
//! Nuphus 实现 `pre_tool_call`、`post_tool_call`、`on_session_start`、`on_session_end` 等钩子。
//!
//! ## 支持的钩子类型
//! - `pre_tool_call`: 工具执行前调用，可返回 Veto 阻止执行
//! - `post_tool_call`: 工具执行后调用，可检查结果
//! - `on_session_start`: 会话开始时调用
//! - `on_session_end`: 会话结束时调用
//!
//! ## 使用方式
//! ```rust
//! use nuphus::hooks::{HookConfig, HookRunner};
//!
//! let config = HookConfig {
//!     pre_tool_call: Some("/path/to/pre_hook.sh".into()),
//!     post_tool_call: Some("/path/to/post_hook.sh".into()),
//!     ..Default::default()
//! };
//! let runner = HookRunner::new(config);
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

/// 钩子配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfig {
    /// 工具执行前调用脚本路径
    /// 脚本接收参数: tool_name json_params
    /// 返回 0 = 允许执行，返回非 0 = Veto（阻止执行）
    pub pre_tool_call: Option<PathBuf>,

    /// 工具执行后调用脚本路径
    /// 脚本接收参数: tool_name json_params json_result
    /// 用于日志、监控、通知等
    pub post_tool_call: Option<PathBuf>,

    /// 会话开始时调用脚本路径
    /// 脚本接收参数: session_id user_input
    pub on_session_start: Option<PathBuf>,

    /// 会话结束时调用脚本路径
    /// 脚本接收参数: session_id success output
    pub on_session_end: Option<PathBuf>,
}

impl HookConfig {
    /// 检查是否配置了任何钩子
    pub fn has_any_hook(&self) -> bool {
        self.pre_tool_call.is_some()
            || self.post_tool_call.is_some()
            || self.on_session_start.is_some()
            || self.on_session_end.is_some()
    }
}

/// 钩子运行时
#[derive(Debug, Clone)]
pub struct HookRunner {
    config: HookConfig,
}

impl HookRunner {
    pub fn new(config: HookConfig) -> Self {
        Self { config }
    }

    /// 执行 pre_tool_call 钩子
    /// 返回 true = 允许执行，false = Veto
    pub fn run_pre_tool_call(&self, tool: &str, params: &serde_json::Value) -> bool {
        let Some(ref script) = self.config.pre_tool_call else {
            return true;
        };

        let params_json = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());

        tracing::debug!(
            "Running pre_tool_call hook: {} {} {}",
            script.display(),
            tool,
            params_json
        );

        match Self::run_script(script, &[tool, &params_json]) {
            Ok(exit_code) => {
                if exit_code != 0 {
                    tracing::warn!("pre_tool_call hook vetoed {} (exit {})", tool, exit_code);
                    false
                } else {
                    tracing::debug!("pre_tool_call hook allowed {}", tool);
                    true
                }
            }
            Err(e) => {
                tracing::error!("pre_tool_call hook failed: {}", e);
                true // 失败时默认允许
            }
        }
    }

    /// 执行 post_tool_call 钩子
    pub fn run_post_tool_call(
        &self,
        tool: &str,
        params: &serde_json::Value,
        result: &crate::ToolResult,
    ) {
        let Some(ref script) = self.config.post_tool_call else {
            return;
        };

        let params_json = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
        let result_json = serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string());

        tracing::debug!(
            "Running post_tool_call hook: {} {} -> {}",
            script.display(),
            tool,
            result.success
        );

        // 异步执行，不阻塞主流程
        let script = script.clone();
        let tool = tool.to_string();
        std::thread::spawn(move || {
            if let Err(e) = Self::run_script_sync(&script, &[&tool, &params_json, &result_json]) {
                tracing::error!("post_tool_call hook failed: {}", e);
            }
        });
    }

    /// 执行 on_session_start 钩子
    pub fn run_session_start(&self, session_id: &str, input: &str) {
        let Some(ref script) = self.config.on_session_start else {
            return;
        };

        tracing::debug!("Running session_start hook: {}", script.display());

        let script = script.clone();
        let session_id = session_id.to_string();
        let input = input.to_string();
        std::thread::spawn(move || {
            if let Err(e) = Self::run_script_sync(&script, &[&session_id, &input]) {
                tracing::error!("session_start hook failed: {}", e);
            }
        });
    }

    /// 执行 on_session_end 钩子
    pub fn run_session_end(&self, session_id: &str, success: bool, output: &str) {
        let Some(ref script) = self.config.on_session_end else {
            return;
        };

        tracing::debug!("Running session_end hook: {}", script.display());

        let script = script.clone();
        let session_id = session_id.to_string();
        let output = output.to_string();
        let success_str = if success { "true" } else { "false" };
        std::thread::spawn(move || {
            if let Err(e) = Self::run_script_sync(&script, &[&session_id, success_str, &output]) {
                tracing::error!("session_end hook failed: {}", e);
            }
        });
    }

    /// 同步执行脚本（用于 pre_tool_call，需要等待结果）
    fn run_script(script: &PathBuf, args: &[&str]) -> std::io::Result<i32> {
        Self::run_script_sync(script, args)
    }

    /// 同步执行脚本的内部实现
    fn run_script_sync(script: &PathBuf, args: &[&str]) -> std::io::Result<i32> {
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;

        // 根据平台选择脚本解释器
        let ext = script
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        let (interpreter, extra_arg): (String, Option<String>) = if cfg!(windows) && ext == "ps1" {
            ("powershell".into(), Some("-File".into()))
        } else if !cfg!(windows) && ext == "sh" {
            ("bash".into(), Some(script.to_string_lossy().into()))
        } else {
            (script.to_string_lossy().into(), None)
        };
        let mut cmd = Command::new(&interpreter);
        // .ps1 on Windows: powershell -File script.ps1 args
        // .sh on Unix: bash script.sh args
        // otherwise: direct executable
        if let Some(ref extra) = extra_arg {
            cmd.arg(extra);
        }
        cmd.arg(script);
        cmd.args(args);

        // 设置环境变量
        cmd.env("NUPHUS_VERSION", env!("CARGO_PKG_VERSION"));
        cmd.env("NUPHUS_HOOKS_VERSION", "1");

        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let output = cmd.output()?;

        Ok(output.status.code().unwrap_or(-1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_runner_default() {
        let runner = HookRunner::new(HookConfig::default());
        // 默认配置所有钩子都是 None，应该直接返回
        assert!(runner.run_pre_tool_call("test", &serde_json::json!({})));
    }

    #[test]
    fn test_hook_config_serialization() {
        let config = HookConfig {
            pre_tool_call: Some(PathBuf::from("/bin/test")),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(yaml.contains("pre_tool_call"));
    }
}
