//! 工具步骤执行
use super::*;

impl Executor {
    /// 执行单个工具调用步骤：确定性重试
    pub(super) async fn execute_tool_step<F, Fut>(
        &self,
        step: &Step,
        tool: &str,
        params: &serde_json::Value,
        tool_exec: &F,
        variables: &mut HashMap<String, serde_json::Value>,
        workflow_id: &str,
        events: &EventBus,
        _llm: Option<&dyn ApiClient>,
        emitter: Option<&dyn EventEmitter>,
    ) -> crate::Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        let _ = emitter; // HUD emitted from execute_step before dispatch

        // ── 变量替换 ──
        let resolved_params = Self::resolve_vars(params, variables);

        // ── 根据 on_error 决定重试策略 ──
        let (max_retries, backoff_ms) = match &step.on_error {
            OnError::Retry {
                max, backoff_ms, ..
            } => (*max, *backoff_ms),
            OnError::Skip => (0, 0),
            OnError::Abort => (MAX_RETRIES, 500),
            // AllowCodes: 不重试，单次执行后由退出码决定
            OnError::AllowCodes { .. } => (0, 0),
        };

        let mut last_error = String::new();

        // ── 确定性重试循环 ──
        for attempt in 0..=max_retries {
            // 重试间隔
            if attempt > 0 {
                events.emit(WorkflowEvent::Error {
                    message: format!(
                        "Tool '{}' ({}): attempt {}/{} after error: {}",
                        tool,
                        step.name,
                        attempt,
                        max_retries + 1,
                        last_error
                    ),
                });
                let delay = if backoff_ms > 0 {
                    Duration::from_millis(backoff_ms * 2u64.pow(attempt - 1))
                } else {
                    Duration::from_millis(500)
                };
                tokio::time::sleep(delay).await;

                // 重试前检查取消信号
                self.check_cancel(workflow_id).await?;
            }

            match tool_exec(tool.to_string(), resolved_params.clone()).await {
                Ok(output) => {
                    // ── 变量捕获 ──
                    super::variables::capture_output(&step.capture, &output, variables)?;
                    return Ok(format!("tool_completed:{}", output));
                }
                Err(e) => {
                    last_error = e;

                    // AllowCodes: 检查退出码，白名单码视为成功
                    if let OnError::AllowCodes { codes } = &step.on_error {
                        // 尝试从错误消息中提取退出码
                        if let Some(exit_code) = parse_exit_code_from_tool_error(&last_error) {
                            if codes.contains(&exit_code) {
                                // 白名单退出码：视为成功但捕获为空字符串
                                super::variables::capture_output(&step.capture, "", variables)?;
                                return Ok(format!("tool_allowcode:exit_code={}", exit_code));
                            }
                        }
                        // 非白名单码 → 不重试，直接失败
                        return Err(crate::NuphusError::agent(format!(
                            "Tool '{}' ({}): exit code not allowed: {}",
                            tool, step.name, last_error
                        )));
                    }

                    // 如果是 Skip 策略，直接失败不重试
                    if matches!(&step.on_error, OnError::Skip) {
                        return Err(crate::NuphusError::agent(format!(
                            "Tool '{}' ({}): {} (skipped)",
                            tool, step.name, last_error
                        )));
                    }
                }
            }
        }

        Err(crate::NuphusError::agent(format!(
            "Tool '{}' ({}): exhausted retries ({}), last error: {}",
            tool,
            step.name,
            max_retries + 1,
            last_error
        )))
    }
}

/// Parse exit code from tool error message (common patterns: "exit code: 1", "exit_code=1")
fn parse_exit_code_from_tool_error(err: &str) -> Option<i32> {
    // Try "exit code: N" or "exit_code: N"
    for pattern in &["exit code:", "exit_code:", "exitcode:"] {
        if let Some(pos) = err.to_lowercase().find(pattern) {
            let after = &err[pos + pattern.len()..];
            if let Some(num) = after
                .trim()
                .split(|c: char| !c.is_ascii_digit() && c != '-')
                .next()
            {
                if let Ok(code) = num.parse::<i32>() {
                    return Some(code);
                }
            }
        }
    }
    None
}
