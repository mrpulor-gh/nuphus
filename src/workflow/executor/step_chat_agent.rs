//! ChatAgent 智能决策节点执行
use super::*;

impl Executor {
    /// 执行 ChatAgent 步骤 — 智能决策节点，带工具调用循环
    ///
    /// 在 workflow 确定性管道中嵌入观察-思考-操作闭环。
    pub(super) async fn execute_chat_step<F, Fut>(
        &self,
        step: &Step,
        message: &str,
        opts: &ChatOpts,
        variables: &mut HashMap<String, serde_json::Value>,
        llm: Option<&dyn ApiClient>,
        emitter: Option<&dyn EventEmitter>,
        tool_exec: &F,
        workflow_id: &str,
        tool_schemas: Option<&[ToolDefinition]>,
    ) -> crate::Result<String>
    where
        F: Fn(String, serde_json::Value) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = std::result::Result<String, String>> + Send,
    {
        let llm = llm.ok_or_else(|| {
            crate::NuphusError::agent("ChatAgent step requires LLM client".to_string())
        })?;

        // 0. 模型路由：opts.model 为 registry 模型 ID 时经 ClientFactory 装配专属 client
        //    （provider/key/transport quirks 由 factory 封装）；registry 无此 ID 时回退为
        //    裸模型名沿用主 client（向后兼容存量工作流，如 'deepseek-v4-pro'）。
        let mut routed_client: Option<std::sync::Arc<dyn ApiClient>> = None;
        if let Some(ref model_id) = opts.model {
            if let Some(ref factory) = self.client_factory {
                match factory.create_client(model_id) {
                    Ok(client) => {
                        tracing::info!(
                            "[ChatAgent] step '{}': routed to registry model '{}'",
                            step.name,
                            model_id
                        );
                        routed_client = Some(client);
                    }
                    Err(_) => {
                        tracing::warn!(
                            "[ChatAgent] step '{}': model '{}' not in registry, fallback to main client (bare model name)",
                            step.name,
                            model_id
                        );
                    }
                }
            }
        }
        let llm: &dyn ApiClient = routed_client.as_deref().unwrap_or(llm);

        // 1. 解析 message 中的变量
        let resolved = super::variables::resolve_vars_str(message, variables);

        // 2. 加载 ChatAgentConfig（agent_id > active，内联字段覆盖）
        let agent_config = {
            let mut base = if let Some(ref agent_id) = opts.agent_id {
                ChatAgentStore::load_by_id(agent_id).unwrap_or_default()
            } else {
                ChatAgentStore::get_active().unwrap_or_default()
            };
            // 内联字段覆盖
            if let Some(ref persona) = opts.persona {
                base.persona = persona.clone();
            }
            if let Some(ref goal) = opts.goal {
                base.goal = Some(goal.clone());
            }
            if let Some(ref constraints) = opts.constraints {
                base.constraints = constraints.clone();
            }
            if let Some(ref requirements) = opts.requirements {
                base.requirements = requirements.clone();
            }
            if let Some(ref knowledge) = opts.knowledge {
                base.knowledge = knowledge.clone();
            }
            if let Some(max_iterations) = opts.max_iterations {
                base.max_iterations = max_iterations;
            }
            base
        };

        let max_iters = opts
            .max_steps
            .unwrap_or(agent_config.max_iterations)
            .min(30);

        // 3. 构建系统提示：基础框架 + 用户配置 + 知识库（内联 system_prompt 覆盖时完全替换）
        let knowledge_paths = opts.knowledge.as_deref().unwrap_or(&[]);
        let user_config = agent_config.build_chatagent_user_config();
        let system_prompt = if let Some(ref override_prompt) = opts.system_prompt {
            override_prompt.clone()
        } else if knowledge_paths.is_empty() {
            format!("{}\n\n{}", super::BASE_CHAT_AGENT_PROMPT, user_config)
        } else {
            let mut knowledge_text = String::new();
            for path in knowledge_paths {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        knowledge_text
                            .push_str(&format!("\n\n--- Knowledge: {} ---\n{}", path, content));
                    }
                    Err(_) => {
                        tracing::warn!("[ChatAgent] Knowledge file not found: {}", path);
                    }
                }
            }
            format!(
                "{}\n\n{}\n\n{}",
                super::BASE_CHAT_AGENT_PROMPT,
                user_config,
                knowledge_text
            )
        };

        // 4. 构建工具定义（白名单过滤）
        let chat_tools: Vec<ToolDefinition> = tool_schemas
            .map(|schemas| {
                schemas
                    .iter()
                    .filter(|def| CHAT_AGENT_ALLOWED_TOOLS.contains(&def.function.name.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // 5. 构建消息历史（同 step 多次调用时恢复上下文，支持循环对话）
        let session_key = format!("{}:{}", workflow_id, step.id);
        let mut messages: Vec<serde_json::Value> = {
            let sessions = self.chat_sessions.read().await;
            if let Some(history) = sessions.get(&session_key) {
                // 恢复历史：已有内容绝对不变，仅追加本轮用户消息
                let mut msgs = history.clone();
                msgs.push(serde_json::json!({"role": "user", "content": resolved}));
                msgs
            } else {
                vec![
                    serde_json::json!({"role": "system", "content": system_prompt}),
                    serde_json::json!({"role": "user", "content": resolved}),
                ]
            }
        };

        // 6. 工具调用循环
        let mut final_reply;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > max_iters {
                return Err(crate::NuphusError::agent(format!(
                    "ChatAgent '{}': exceeded max iterations ({})",
                    step.name, max_iters
                )));
            }

            // Emit HUD progress
            if let Some(emitter) = emitter {
                emitter.emit(NuphusEvent::HudUpdate {
                    text: format!("{} — 思考中... ({}/{})", step.name, iteration, max_iters),
                    phase: "workflow".into(),
                    step_kind: Some("chat_agent".to_string()),
                });
            }

            // 调用 LLM — 应用 ChatOpts 内联模型参数（有值才覆盖默认）
            let model = opts
                .model
                .clone()
                .unwrap_or_else(|| "deepseek-v4-pro".to_string());
            let mut request = crate::api::MessageRequest::new(model, messages.clone());
            if let Some(max_tokens) = opts.max_tokens {
                request.max_tokens = Some(max_tokens);
            }
            if let Some(temperature) = opts.temperature {
                request.temperature = Some(temperature);
            }
            request.tools = if chat_tools.is_empty() {
                None
            } else {
                Some(chat_tools.clone())
            };
            let events = llm.stream(request).await.map_err(|e| {
                crate::NuphusError::agent(format!(
                    "ChatAgent '{}': LLM call failed: {}",
                    step.name, e
                ))
            })?;

            // 提取文本响应和工具调用
            let mut response = String::new();
            let mut tool_uses: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
            for event in &events {
                match event {
                    crate::api::AssistantEvent::TextDelta(t) => response.push_str(t),
                    crate::api::AssistantEvent::ToolUse { id, name, input } => {
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            let msg: serde_json::Value = if tool_uses.is_empty() {
                serde_json::json!({"role": "assistant", "content": response})
            } else {
                let tc_arr: Vec<serde_json::Value> = tool_uses
                    .iter()
                    .map(|(id, fn_name, args)| {
                        serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": fn_name,
                                "arguments": args
                            }
                        })
                    })
                    .collect();
                let mut msg_obj = serde_json::json!({"role": "assistant", "content": response});
                msg_obj["tool_calls"] = serde_json::Value::Array(tc_arr);
                msg_obj
            };

            let content = msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
            final_reply = content.to_string();

            messages.push(msg.clone());

            // 检查是否有工具调用
            let tool_calls = msg.get("tool_calls").and_then(|v| v.as_array());

            if let Some(tc_arr) = tool_calls {
                if tc_arr.is_empty() {
                    break; // No more tool calls → done
                }

                for tc in tc_arr {
                    let fn_name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let fn_args_str = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}");
                    let fn_args: serde_json::Value =
                        serde_json::from_str(fn_args_str).unwrap_or(serde_json::Value::Null);

                    // HUD: 显示正在调用的工具，让用户在 chat_agent 执行时可见具体动作
                    if let Some(emitter) = emitter {
                        emitter.emit(NuphusEvent::HudUpdate {
                            text: format!("{} — 调用 {}…", step.name, fn_name),
                            phase: "workflow".into(),
                            step_kind: Some("chat_agent".to_string()),
                        });
                    }

                    // Execute tool
                    match tool_exec(fn_name.to_string(), fn_args.clone()).await {
                        Ok(output) => {
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tc.get("id").and_then(|i| i.as_str()).unwrap_or(""),
                                "content": output,
                            }));
                        }
                        Err(e) => {
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tc.get("id").and_then(|i| i.as_str()).unwrap_or(""),
                                "content": format!("Error: {}", e),
                            }));
                        }
                    }
                }
            } else {
                // No tool calls → final response
                break;
            }
        }

        // 10. 持久化会话上下文（下次同 step 被调用时恢复）
        {
            let mut sessions = self.chat_sessions.write().await;
            sessions.insert(session_key, messages);
        }

        // 11. 变量捕获 — 使用干净的 final_reply，不含工具日志污染
        if let Some(ref capture) = step.capture {
            let var_name = super::variables::resolve_vars_str(capture, variables);
            variables.insert(var_name, serde_json::Value::String(final_reply.clone()));
        }

        Ok(final_reply)
    }
}
