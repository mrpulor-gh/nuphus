//! react_loop — ReAct main loop (moved from agent::ReactAgent into Runtime)
//!
//! ReactAgent::run()'s ~630 line loop body is inlined here;
//! Runtime truly owns the execution loop, ReactAgent degrades to a pure state container.

use crate::agent::events::{NuphusEvent, TaskItem};
use crate::agent::prompt;
use crate::agent::reminders::{ReminderCategory, ReminderPriority};
use crate::runtime::protection::{ProtectionAlert, ProtectionGuard};
use crate::workflow::compiler::Compiler;
use crate::{session::ContentBlock, ExecutionStep, StepStatus};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

impl super::Runtime {
    pub(super) async fn react_loop(
        &mut self,
        input: &str,
        images: &Option<Vec<String>>,
        cancel_flag: &AtomicBool,
        resume: bool,
    ) -> crate::Result<crate::AgentOutput> {
        tracing::info!("Agent starting task: {} (resume={})", input, resume);

        let loop_start = std::time::Instant::now();

        // resume（断点续跑）时跳过新回合初始化：
        // - steps 保留（失败回合的进度计入本轮 tools_used / total_calls）
        // - 不触发 session_start hook（不是新会话）
        // - 不 pop 孤儿 user 消息（session 末尾的 user 消息/工具结果就是断点本身）
        if !resume {
            // Clear previous round's steps (already returned via AgentOutput.steps clone)
            self.agent.steps.clear();

            // 0. Session Start Hook
            if let Some(ref hooks) = self.agent.hooks {
                hooks.run_session_start(&self.agent.session.id, input);
            }

            // 0.5 Clean up orphan user messages left from last round's interruption
            self.agent.session.pop_last_user_if_orphan();

            // 0.5b 新任务开始时清空上一任务残留的追加指令队列：
            // 追加指令语义 =「发送时正在执行的任务内生效」，任务结束后残留若不清空，
            // 会在本任务首轮 drain 时被注入——跨任务泄漏（刷新/重连后重复指令的根因之一）。
            crate::mobile_append::clear();
        }

        // 0.6 Reset this round's safety check counter
        self.agent.safety_consecutive_failures = 0;

        // 0.6 Drain leftover pending_append queue
        let _ = crate::agent::pause::drain_pending_append();

        if cancel_flag.load(Ordering::SeqCst) {
            tracing::info!("[INTERRUPT] Cancelled after context retrieval");
            self.agent.emit_exec("// interrupted");
            if let Some(ref emitter) = self.agent.exec_emitter {
                emitter.emit(NuphusEvent::HudUpdate {
                    text: "已中断".into(),
                    phase: "done".into(),
                    step_kind: None,
                });
            }
            return Ok(crate::AgentOutput {
                success: false,
                message: "任务已被用户中断".to_string(),
                steps: vec![],
                retry_session: None,
            });
        }

        // 2. Add user input to session（resume 时跳过：断点已在 session 中，直接续跑）
        if !resume {
            if let Some(imgs) = images {
                if !imgs.is_empty() {
                    let mut blocks: Vec<ContentBlock> = Vec::new();
                    if !input.is_empty() {
                        blocks.push(ContentBlock::Text {
                            text: input.to_string(),
                            reasoning: None,
                        });
                    }
                    for url in imgs {
                        // ════════════════════════════════════════════════════════════
                        // 媒体附件入 session 原则：一旦入 session，字节流必须冻结
                        //
                        // Image: BMP→PNG 在入 session 时转换一次，后续 to_api_messages
                        //   直接用已转换的 PNG data URL，不再重新编码，确保相同图片
                        //   始终产生完全相同的 messages JSON，prompt cache 不断裂。
                        //
                        // Audio/Video（未来扩展）：同理，所有格式转换/重编码必须在
                        //   入 session 时完成。禁止在 to_api_messages 中做任何可能
                        //   产生非确定性输出的转换。文件名用内容 hash 而非时间戳。
                        // ════════════════════════════════════════════════════════════
                        let final_url = if url.starts_with("data:image/bmp") {
                            crate::utils::convert_bmp_data_url_to_png(url).unwrap_or_else(|e| {
                                tracing::warn!("[react_loop] BMP→PNG 转换失败: {e}，使用原始 URL");
                                url.clone()
                            })
                        } else {
                            url.clone()
                        };
                        blocks.push(ContentBlock::Image { url: final_url });
                    }
                    self.agent.session.push_user_blocks(blocks);
                } else {
                    self.agent.session.push_user(input.to_string());
                }
            } else {
                self.agent.session.push_user(input.to_string());
            }
            self.agent.emit_exec(&format!("> {}", input));
        }

        // 2.5 Send session start system message
        self.agent.emit_exec(&format!(
            "Session started · Model: {} · {}",
            self.agent.config.model,
            chrono::Local::now().format("%H:%M:%S")
        ));

        // 4. Build request
        // Custom mode whitelist narrows the prompt-visible tool list.
        let tool_schemas = match &self.agent.custom_tool_whitelist {
            Some(wl) => self.agent.tools.render_tools_for_prompt_filtered(wl),
            None => self.agent.tools.render_tools_for_prompt(),
        };
        let _model_label = format!(
            "{} ({})",
            self.agent.config.model, self.agent.config.provider
        );

        // ── Build merged system prompt (L0+L2+L1) — once per session ──
        let merged_system = self.agent.cached_merged_system_prompt.get_or_insert_with(|| {
            // L0: resolved identity prompt
            let l0_raw = self.agent.cached_base_prompt.get_or_insert_with(|| {
                let stable = prompt::build_leader_base_prompt(&self.agent.leader_ctx);
                tracing::info!("[PromptCache] L0 Kernel built ({} chars), cached for session", stable.len());
                stable
            }).clone();

            let user_label = self.agent.leader_ctx.relation.as_ref()
                .and_then(|r| if r.user_label.is_empty() { None } else { Some(r.user_label.as_str()) })
                .unwrap_or("用户");
            let assistant_name = self.agent.leader_ctx.relation.as_ref()
                .and_then(|r| if r.assistant_name.is_empty() { None } else { Some(r.assistant_name.as_str()) })
                .unwrap_or("Nuphus");
            let resolved_l0 = if user_label != "用户" || assistant_name != "Nuphus" {
                prompt::resolve_placeholders(&l0_raw, user_label, assistant_name)
            } else {
                l0_raw
            };

            // L1: L2 + tools + tenets + cross_session + env + skills
            let l1_items = self.agent.cached_l1_prompt.get_or_insert_with(|| {
                let mut l1_buf: Vec<String> = Vec::new();
                // Custom mode: L2 fully replaced by the active custom card's free text.
                // L0 (above) and the L1 system sections (below) always stay intact.
                if self.config.mode == crate::runtime::Mode::Custom {
                    let custom_l2 = crate::custom_agents::CustomAgentStore::get_active()
                        .map(|c| c.l2_prompt)
                        .filter(|s| !s.trim().is_empty());
                    match custom_l2 {
                        Some(l2) => l1_buf.push(l2),
                        // No active card or empty L2 → fall back to Leader L2 so the
                        // agent never runs without any behavioral guidance.
                        None => l1_buf.push(prompt::build_l2_leader()),
                    }
                } else {
                    l1_buf.push(prompt::build_l2_leader());
                }
                // Custom knowledge 绑定（目录/文件 → 读取注入）。位于 get_or_insert_with
                // 内 → 仅在首次构建时读取，session 内编辑卡片不重读（同 session 不变，
                // 与 save_custom_agent 缓存纪律一致）；换卡 invalidate 后随新卡片重注入。
                if self.config.mode == crate::runtime::Mode::Custom {
                    if let Some(cfg) = crate::custom_agents::CustomAgentStore::get_active() {
                        if !cfg.knowledge.is_empty() {
                            let ks = prompt::custom_knowledge_section(&cfg.knowledge);
                            if !ks.is_empty() {
                                l1_buf.push(ks);
                            }
                        }
                    }
                }
                l1_buf.push(prompt::tool_schemas_section(&tool_schemas));
                let tenets = prompt::tenets_section();
                if !tenets.is_empty() {
                    l1_buf.push(format!("## 用户原则\n{}", tenets));
                }
                // WorkflowAgent 不注入 Leader 记忆（cross_session_context 含 Leader 记忆日志 +
                // 蒸馏标题）：其跨会话记忆由 workflow-memory.md 专属注入承载
                //（workflow_agent::inject_memory_snapshot，已含 session id 与记忆文件导航），
                // 避免 Leader 发布/CI 等记忆混入工作流设计上下文。
                if self.config.mode != crate::runtime::Mode::Workflow {
                    if let Some(md) = crate::agent::ReactAgent::load_cross_session_context(Some(&self.agent.session.id)) {
                        l1_buf.push(format!("## 跨阶段上下文\n{}\n\n> 以上具体记忆内容可通过 memory_search / memory_recent / memory_session_context 工具查询完整记录。\n", md));
                        tracing::debug!("[CROSS_SESSION] Injected cross-session context ({} chars)", md.len());
                    }
                }
l1_buf.push(prompt::env_info_section(&self.agent.config.model, self.agent.config.supports_vision, self.agent.config.vision_model.as_deref(), prompt::EnvAudience::Leader));
                let skill_reg = prompt::skill_registry_section();
                if !skill_reg.is_empty() {
                    l1_buf.push(skill_reg);
                }
                l1_buf.push(prompt::mcp_tools_section());
                tracing::info!("[PromptCache] L1 built ({} items, ~{} total chars), cached for session",
                    l1_buf.len(),
                    l1_buf.iter().map(|s| s.len()).sum::<usize>()
                );
                l1_buf
            }).clone();

            // Merge: L0 + L2 + L1_rest (without L2 duplicate)
            let mut parts = Vec::with_capacity(1 + l1_items.len());
            parts.push(resolved_l0);
            parts.extend(l1_items.iter().cloned());
            let merged = parts.join("\n\n");
            tracing::info!("[PromptCache] Merged system prompt built ({} chars), cached for session", merged.len());
            merged
        }).clone();

        // 5. ReAct main loop
        let mut protection = ProtectionGuard::new();
        let mut leader_should_stop = false;
        let mut has_used_write_file = false;
        let user_wants_file = crate::agent::common::wants_file_output(input);

        // Read in empty-block detection path (line 334) and reset on non-empty path (line 347);
        // compiler may not track the conditional read pattern, so allow unused_assignments.
        #[allow(unused_assignments)]
        let mut consecutive_empty = 0u32;

        // Whether we've already asked the model to produce a formal text reply
        // after it output reasoning-only (thinking without visible text).
        // Prevents infinite follow-up loops; resets each run() call.
        let mut reasoning_followup_done = false;

        for iteration in 0..self.agent.config.max_iterations {
            if iteration > 0 && iteration % 5 == 0 {
                let progress = crate::agent::common::render_ascii_progress(
                    iteration,
                    self.agent.config.max_iterations,
                );
                self.agent.emit_exec(&progress);
            }

            self.agent.session.strip_incomplete_tools();

            if cancel_flag.load(Ordering::SeqCst) {
                tracing::info!(
                    "[INTERRUPT] Agent loop cancelled at iteration {}",
                    iteration
                );
                self.agent.emit_exec("// interrupted");
                return Ok(crate::AgentOutput {
                    success: false,
                    message: "任务已被用户中断".to_string(),
                    steps: self.agent.steps.clone(),
                    retry_session: None,
                });
            }

            // ── Pause check ──
            if let Some(ref pause_flag) = self.agent.pause_flag {
                if pause_flag.load(Ordering::SeqCst) {
                    let action_id =
                        crate::agent::pause::get_pause_action_id(self.agent.tools.signals())
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    // Check if decision already pre-set (continue/append/terminate via frontend).
                    // Skip ExecutionPaused emit to prevent re-popup after user has already dismissed the pause UI.
                    let skip_emit = crate::agent::pause::peek_pause_decision(
                        self.agent.tools.signals(),
                        &action_id,
                    )
                    .is_some();
                    if !skip_emit {
                        if let Some(ref emitter) = self.agent.exec_emitter {
                            emitter.emit(NuphusEvent::ExecutionPaused {
                                action_id: action_id.clone(),
                            });
                        }
                    }
                    let decision = crate::agent::pause::wait_for_pause_decision_global(
                        self.agent.tools.signals(),
                        &action_id,
                        cancel_flag,
                    )
                    .await;
                    pause_flag.store(false, Ordering::SeqCst);
                    match decision {
                        crate::agent::pause::PauseDecision::Continue => {
                            tracing::info!("[PAUSE] User chose to continue");
                        }
                        crate::agent::pause::PauseDecision::Append(instr) => {
                            tracing::info!(
                                "[PAUSE] User appended instruction: {}",
                                instr.chars().take(80).collect::<String>()
                            );
                            // 统一走 format_mobile_append_section：带 [APPEND] 标记，
                            // chat_history 过滤（追加消息不显示在历史）；同时 push_pending_append
                            // 供后续轮次注入语义一致。
                            self.agent.session.push_user_internal(
                                crate::mobile_append::format_mobile_append_section(
                                    std::slice::from_ref(&instr),
                                ),
                            );
                            crate::agent::pause::push_pending_append(instr);
                        }
                        crate::agent::pause::PauseDecision::Terminate => {
                            tracing::info!("[PAUSE] User chose to terminate — graceful stop");
                            leader_should_stop = true;
                            self.agent.session.push_user_internal(
                            "⚠ 用户要求立即停止当前操作。请立即整理已有成果，输出已完成内容和当前状态，不要继续执行任何工具调用。".to_string()
                        );
                            self.agent
                                .emit_exec("// user requested stop — letting LLM wrap up");
                        }
                    }
                }
            }

            tracing::debug!("Iteration {}", iteration + 1);

            // Inject active reminders as user message (doesn't break system prompt cache)
            if let Some(reminder_text) = self.agent.reminders.format_for_prompt() {
                if !reminder_text.is_empty() {
                    self.agent.session.push_user_internal(reminder_text);
                }
            }

            // 门铃事件：与 reminders 同一注入位——轮次边界被动 drain，事件驱动无轮询。
            // 无事件时 drain 返回空 Vec，不注入、不产生日志。
            let handoff_events = crate::handoff::drain_for_injection();
            if !handoff_events.is_empty() {
                self.agent
                    .session
                    .push_user_internal(crate::handoff::format_doorbell_section(&handoff_events));
            }

            // 手机追加指令：与门铃同一注入位——执行中手机发送的消息
            // （busy 锁占用时入队）在轮次边界被动 drain，插入下一迭代。
            let mobile_appends = crate::mobile_append::drain_for_injection();
            if !mobile_appends.is_empty() {
                self.agent.session.push_user_internal(
                    crate::mobile_append::format_mobile_append_section(&mobile_appends),
                );
            }

            let request = self.agent.build_request(&merged_system);

            self.agent.emit_pre_llm_diag(iteration, &request.messages);

            // Provider-declared content tool tags. Hoisted before the stream
            // emitter so the per-chunk TextDelta cleaner strips exactly the set
            // the terminal normalizer uses for session storage.
            let content_tool_tags = crate::config::registry::ProviderRegistry::builtin()
                .get(&self.agent.config.provider)
                .map(|p| p.quirks().content_tool_tags)
                .unwrap_or(&[]);

            // ── LLM call span ──
            let llm_span = tracing::info_span!("llm_call", turn = iteration);
            let _llm_enter = llm_span.enter();

            // Streaming call (with cancel flag + smart retry), TextDelta emitted to frontend in real-time
            let max_llm_retries: u32 = 10;
            let mut llm_retry: u32 = 0;
            let events: Vec<crate::api::AssistantEvent> = loop {
                // State tracking for <think> block depth across streaming chunks.
                // AtomicU32 tracks nesting depth to prevent premature close when
                // LLM discusses `` tags within thinking content.
                let in_think = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
                let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let exec_emitter = self.agent.exec_emitter.clone();
                let collected_clone = collected.clone();
                let think_state = in_think.clone();
                let emitter = Box::new(move |event: crate::api::AssistantEvent| {
                    if let crate::api::AssistantEvent::TextDelta(text) = &event {
                        // Single routing entry: process_text_delta (think split +
                        // tool-XML strip with the provider tag set), then emit
                        // thinking (is_thinking=true) before content
                        // (is_thinking=false) so the frontend order is stable.
                        crate::agent::common::route_stream_text_delta(
                            text,
                            &think_state,
                            content_tool_tags,
                            false,
                            exec_emitter.as_deref(),
                        );
                    }
                    if let crate::api::AssistantEvent::Reasoning(text) = &event {
                        // DeepSeek thinking mode: reasoning_content deltas arrive as
                        // Reasoning events (not TextDelta). Forward in real-time so
                        // thinking appears BEFORE text in the frontend timeline.
                        if let Some(ref emitter) = exec_emitter {
                            emitter.emit(NuphusEvent::LlmTextDelta {
                                text: text.clone(),
                                is_thinking: true,
                                from_task: false,
                            });
                        }
                    }
                    if let crate::api::AssistantEvent::ConnectionStatus(msg) = &event {
                        if let Some(ref emitter) = exec_emitter {
                            emitter.emit(NuphusEvent::Warning {
                                code: "connection_status".to_string(),
                                message: msg.clone(),
                            });
                        }
                    }
                    if let crate::api::AssistantEvent::ImageAttachment { url } = &event {
                        if let Some(ref emitter) = exec_emitter {
                            emitter.emit(NuphusEvent::ImageGenerated { url: url.clone() });
                        }
                    }
                    collected_clone
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(event);
                });
                match self
                    .agent
                    .llm
                    .stream_with_emitter(request.clone(), cancel_flag, emitter)
                    .await
                {
                    Ok(()) => {
                        break std::mem::take(
                            &mut *collected.lock().unwrap_or_else(|e| e.into_inner()),
                        )
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if cancel_flag.load(Ordering::SeqCst) {
                            self.agent.session.strip_incomplete_tools();
                            let session_json =
                                serde_json::to_string(&self.agent.session).unwrap_or_default();
                            return Ok(crate::AgentOutput {
                                message: "任务已被用户中断".to_string(),
                                success: false,
                                steps: self.agent.steps.clone(),
                                retry_session: Some(session_json),
                            });
                        }
                        if !crate::agent::common::is_retryable_llm_error(&err_str) {
                            self.agent.emit_exec(&format!(
                                "// ERROR: LLM error (non-retryable): {}",
                                err_str
                            ));
                            self.agent.session.strip_incomplete_tools();
                            let session_json =
                                serde_json::to_string(&self.agent.session).unwrap_or_default();
                            return Ok(crate::AgentOutput {
                                message: format!(
                                    "LLM请求失败：{}\n该错误不可重试，请检查配置或模型状态",
                                    err_str
                                ),
                                success: false,
                                steps: self.agent.steps.clone(),
                                retry_session: Some(session_json),
                            });
                        }
                        llm_retry += 1;
                        if llm_retry >= max_llm_retries {
                            self.agent.emit_exec(&format!(
                                "// ERROR: LLM error after {} retries: {}",
                                max_llm_retries, err_str
                            ));
                            self.agent.session.strip_incomplete_tools();
                            let session_json =
                                serde_json::to_string(&self.agent.session).unwrap_or_default();
                            return Ok(crate::AgentOutput {
                                message: format!(
                                    "LLM请求失败（已自动重试{}次）\n点击下方按钮重新连接",
                                    max_llm_retries
                                ),
                                success: false,
                                steps: self.agent.steps.clone(),
                                retry_session: Some(session_json),
                            });
                        }
                        let base_wait = 2u64.pow(llm_retry.min(6));
                        let jitter = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                            % 1000;
                        let wait_ms = base_wait * 1000 + jitter;
                        self.agent.emit_exec(&format!(
                            "// LLM 请求失败，{:.1}s 后重试 ({}/{})",
                            wait_ms as f64 / 1000.0,
                            llm_retry,
                            max_llm_retries
                        ));
                        if let Some(ref emitter) = self.agent.exec_emitter {
                            emitter.emit(NuphusEvent::Warning {
                                code: "llm_retry".to_string(),
                                message: format!(
                                    "Network connection timeout, retrying ({}/{})",
                                    llm_retry, max_llm_retries
                                ),
                            });
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                    }
                }
            };

            if cancel_flag.load(Ordering::SeqCst) {
                tracing::info!(
                    "[INTERRUPT] Agent cancelled after LLM call at iteration {}",
                    iteration
                );
                self.agent.emit_exec("// interrupted");
                return Ok(crate::AgentOutput {
                    success: false,
                    message: "任务已被用户中断".to_string(),
                    steps: self.agent.steps.clone(),
                    retry_session: None,
                });
            }

            // Process events — pass provider-specific content tool tags (hoisted
            // above) so XML-embedded tool calls (MiniMax etc.) are parsed as ToolUse
            let processed = crate::agent::common::process_events(events, content_tool_tags);
            if let Some((input, output)) = &processed.usage {
                self.agent.session.update_api_input_tokens(*input as u64);
                tracing::info!(tokens_in = input, tokens_out = output, "LLM call completed");
                if let Some(ref emitter) = self.agent.exec_emitter {
                    // exec source: single call consumption (frontend accumulates incrementally)
                    emitter.emit(NuphusEvent::TokenUsage {
                        input_tokens: *input,
                        output_tokens: *output,
                        cache_hit_tokens: processed.cache_hit_tokens,
                        source: "exec".to_string(),
                    });
                    // main source: Leader accumulated context usage (continuously updates progress bar)
                    let leader_ctx = self.agent.session.api_input_tokens as u32;
                    emitter.emit(NuphusEvent::TokenUsage {
                        input_tokens: leader_ctx,
                        output_tokens: 0,
                        cache_hit_tokens: processed.cache_hit_tokens,
                        source: "main".to_string(),
                    });
                }
            }
            drop(_llm_enter);
            let assistant_blocks = processed.blocks;

            self.agent.emit_post_llm_diag(iteration, &assistant_blocks);
            let text_count = assistant_blocks
                .iter()
                .filter(|b| matches!(b, ContentBlock::Text { .. }))
                .count();
            let tool_count = assistant_blocks
                .iter()
                .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
                .count();
            if tool_count == 0 {
                let text_preview = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(120)
                    .collect::<String>();
                tracing::info!(
                    "[DIAG] iter {}: {} text blocks, 0 tool calls — text preview: {:?}",
                    iteration,
                    text_count,
                    text_preview
                );
            } else {
                tracing::info!(
                    "[DIAG] iter {}: {} text blocks, {} tool calls",
                    iteration,
                    text_count,
                    tool_count
                );
            }

            if assistant_blocks.is_empty() {
                consecutive_empty += 1;
                if consecutive_empty >= 3 {
                    tracing::warn!(
                        "[AGENT] {} consecutive empty responses, breaking",
                        consecutive_empty
                    );
                    return Ok(crate::AgentOutput {
                        success: false,
                        message: String::new(),
                        steps: self.agent.steps.clone(),
                        retry_session: Some(
                            serde_json::to_string(&self.agent.session).unwrap_or_default(),
                        ),
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                continue;
            }
            consecutive_empty = 0;

            // Extract tool calls
            let tool_calls = crate::agent::common::extract_tool_calls(&assistant_blocks);

            // Text and reasoning already emitted in real-time during streaming
            // (TextDelta via process_text_delta, Reasoning via direct forward).
            // Only emit_exec preview for final text block.
            for block in &assistant_blocks {
                if let ContentBlock::Text { text, .. } = block {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        self.agent
                            .emit_exec(&trimmed.chars().take(200).collect::<String>().to_string());
                    }
                }
            }

            // === No tool calls → check if we have a real text response ===
            if tool_calls.is_empty() {
                let text = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                // Empty text + non-empty reasoning → model produced thinking but no formal reply.
                // Ask the model once for a formal text reply so that memory stores the
                // user-facing text (not internal reasoning). Falls back to reasoning
                // display only if the follow-up also yields no text.
                if text.trim().is_empty() {
                    let has_reasoning = assistant_blocks.iter().any(|b| {
                        matches!(
                            b,
                            ContentBlock::Text {
                                reasoning: Some(_),
                                ..
                            }
                        )
                    });
                    if has_reasoning {
                        if !reasoning_followup_done {
                            reasoning_followup_done = true;
                            self.agent.session.push_assistant(assistant_blocks);
                            self.agent.session.push_user_internal(
                                "【系统】你刚才只输出了思考过程，没有给出正式回复。请直接输出面向用户的正式回复内容（不要重复思考过程）。".to_string()
                            );
                            tracing::info!(
                                "Model output reasoning only — requesting formal text reply"
                            );
                            continue;
                        }
                        // Follow-up also produced no text → fall back to reasoning as display output
                        let reasoning_text: String = assistant_blocks
                            .iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text {
                                    reasoning: Some(r), ..
                                } => Some(r.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let display = if reasoning_text.is_empty() {
                            "（模型未产出有效文本回复）".to_string()
                        } else {
                            reasoning_text
                        };
                        self.agent.last_output_text = Some(display.clone());
                        self.agent.all_turn_texts.push(display.clone());
                        self.agent.session.push_assistant(assistant_blocks);
                        self.agent.emit_exec(&display);
                        tracing::info!(
                            "Model output reasoning only after follow-up, using it as final text"
                        );
                        if let Some(ref hooks) = self.agent.hooks {
                            hooks.run_session_end(&self.agent.session.id, true, &display);
                        }
                        // Leader completion: emit HudUpdate done + ExecutionCompleted + LeaderDone
                        if let Some(ref emitter) = self.agent.exec_emitter {
                            let total_duration = loop_start.elapsed().as_millis() as u64;
                            let tool_calls_count = self.agent.steps.len();
                            emitter.emit(NuphusEvent::HudUpdate {
                                text: "任务完成".into(),
                                phase: "done".into(),
                                step_kind: None,
                            });
                            emitter.emit(NuphusEvent::ExecutionCompleted {
                                step_index: 0,
                                output: crate::agent::events::StepOutput {
                                    step_index: 0,
                                    result_message: display.clone(),
                                    artifacts: vec![],
                                    tool_calls_count,
                                },
                                total_duration_ms: total_duration,
                                total_calls: tool_calls_count,
                            });
                            emitter.emit(NuphusEvent::LeaderDone {
                                message: "任务完成".into(),
                            });
                        }
                        return Ok(crate::AgentOutput {
                            message: display,
                            success: true,
                            steps: self.agent.steps.clone(),
                            retry_session: None,
                        });
                    }
                }
                let display = text.trim().to_string();
                self.agent.last_output_text = Some(display.clone());
                self.agent.all_turn_texts.push(display.clone());
                self.agent.session.push_assistant(assistant_blocks);
                self.agent.emit_exec(&display);
                tracing::info!("Agent finished successfully");
                if let Some(ref hooks) = self.agent.hooks {
                    hooks.run_session_end(&self.agent.session.id, true, &display);
                }
                // Leader completion: emit HudUpdate done + ExecutionCompleted + LeaderDone
                if let Some(ref emitter) = self.agent.exec_emitter {
                    let total_duration = loop_start.elapsed().as_millis() as u64;
                    let tool_calls_count = self.agent.steps.len();
                    emitter.emit(NuphusEvent::HudUpdate {
                        text: "任务完成".into(),
                        phase: "done".into(),
                        step_kind: None,
                    });
                    emitter.emit(NuphusEvent::ExecutionCompleted {
                        step_index: 0,
                        output: crate::agent::events::StepOutput {
                            step_index: 0,
                            result_message: display.clone(),
                            artifacts: vec![],
                            tool_calls_count,
                        },
                        total_duration_ms: total_duration,
                        total_calls: tool_calls_count,
                    });
                    emitter.emit(NuphusEvent::LeaderDone {
                        message: "任务完成".into(),
                    });
                }
                return Ok(crate::AgentOutput {
                    message: display,
                    success: true,
                    steps: self.agent.steps.clone(),
                    retry_session: None,
                });
            }

            // === Path where tool_calls exist ===
            let valid_ids: std::collections::HashSet<&str> =
                tool_calls.iter().map(|c| c.id.as_str()).collect();
            let filtered_blocks: Vec<ContentBlock> = assistant_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, .. } if !valid_ids.contains(id.as_str()) => None,
                    _ => Some(b.clone()),
                })
                .collect();
            self.agent.session.push_assistant(filtered_blocks);

            // Accumulate text blocks from this iteration into all_turn_texts
            for block in &assistant_blocks {
                if let ContentBlock::Text { text, .. } = block {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        self.agent.all_turn_texts.push(trimmed);
                    }
                }
            }

            let mut post_tool_warnings: Vec<String> = Vec::new();

            if leader_should_stop {
                let stop_text = assistant_blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let result_msg = if stop_text.trim().is_empty() {
                    "操作已停止".to_string()
                } else {
                    stop_text.trim().to_string()
                };
                if let Some(ref emitter) = self.agent.exec_emitter {
                    if result_msg != "操作已停止" {
                        emitter.emit(NuphusEvent::DirectResponse {
                            message: result_msg.clone(),
                        });
                    }
                    emitter.emit(NuphusEvent::HudUpdate {
                        text: "任务完成".into(),
                        phase: "done".into(),
                        step_kind: None,
                    });
                }
                return Ok(crate::AgentOutput {
                    message: result_msg,
                    success: true,
                    steps: self.agent.steps.clone(),
                    retry_session: None,
                });
            }

            // Extract reasoning from this iteration's assistant_blocks — shared across all tool_calls
            let iteration_reasoning: Option<String> =
                assistant_blocks.iter().find_map(|b| match b {
                    ContentBlock::Text {
                        reasoning: Some(r), ..
                    } => Some(r.clone()),
                    _ => None,
                });

            // Execute tools
            for call in tool_calls {
                let tool_start = std::time::Instant::now();
                if let Some(ref emitter) = self.agent.exec_emitter {
                    emitter.emit(NuphusEvent::ToolCallStart {
                        call_id: call.id.clone(),
                        tool_name: call.tool.clone(),
                        params: call.params.clone(),
                        iteration: iteration as u32,
                        from_task: false,
                    });
                }

                // Tool name validation: intercept non-existent tool names early to prevent system prompt corruption
                if !self.agent.tools.has_tool(&call.tool) {
                    self.agent.session.push_tool_result(
                        call.id.clone(),
                        format!(
                            "未知工具「{}」。请仅使用可用工具列表中的工具名，不要编造工具名。",
                            call.tool
                        ),
                        false,
                    );
                    continue;
                }

                // Protection check — check before tool execution to prevent unnecessary execution
                if let Some(alert) = protection.check_pre_call(&call) {
                    if matches!(alert, ProtectionAlert::SameFileReadOveruse { .. }) {
                        // Push dummy tool_result (doesn't modify system prompt, doesn't create dangling tool_call)
                        self.agent.session.push_tool_result(
                            call.id.clone(),
                            alert.to_session_warning(),
                            false,
                        );
                        continue;
                    }
                    if !matches!(alert, ProtectionAlert::SameFileReadOveruse { .. }) {
                        self.agent.emit_exec(&format!(
                            "// WARN: {}: {}",
                            alert.label(),
                            alert.to_session_warning()
                        ));
                    }
                    if let Some(reminder) = alert.to_force_output_reminder() {
                        if !has_used_write_file && user_wants_file {
                            self.agent.reminders.enqueue(
                                reminder.to_string(),
                                3,
                                ReminderPriority::Critical,
                                ReminderCategory::DeviationCorrect,
                            );
                            self.agent.emit_exec("// forcing LLM to produce output now");
                            // Check if this is a repeat — DeadLoop means we've already warned
                            if matches!(alert, ProtectionAlert::DeadLoop { .. }) {
                                continue;
                            }
                        }
                    }
                    self.agent.session.push_assistant_internal(vec![
                        crate::session::ContentBlock::Text {
                            text: format!("任务完成（共执行 {} 步）", self.agent.steps.len()),
                            reasoning: None,
                        },
                    ]);
                    // Leader completion: emit HudUpdate done + ExecutionCompleted + LeaderDone
                    if let Some(ref emitter) = self.agent.exec_emitter {
                        let total_duration = loop_start.elapsed().as_millis() as u64;
                        let tool_calls_count = self.agent.steps.len();
                        let result_msg = format!("任务完成（共执行 {} 步）", tool_calls_count);
                        emitter.emit(NuphusEvent::HudUpdate {
                            text: "任务完成".into(),
                            phase: "done".into(),
                            step_kind: None,
                        });
                        emitter.emit(NuphusEvent::ExecutionCompleted {
                            step_index: 0,
                            output: crate::agent::events::StepOutput {
                                step_index: 0,
                                result_message: result_msg,
                                artifacts: vec![],
                                tool_calls_count,
                            },
                            total_duration_ms: total_duration,
                            total_calls: tool_calls_count,
                        });
                        emitter.emit(NuphusEvent::LeaderDone {
                            message: "任务完成".into(),
                        });
                    }
                    return Ok(crate::AgentOutput {
                        message: format!("任务完成（共执行 {} 步）", self.agent.steps.len()),
                        success: true,
                        steps: self.agent.steps.clone(),
                        retry_session: None,
                    });
                }

                self.agent.emit_exec(&format!(
                    "{}: {}",
                    call.tool,
                    call.params
                        .as_object()
                        .map(|obj| obj
                            .iter()
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<_>>()
                            .join(", "))
                        .unwrap_or_default()
                ));

                // ── Tool execution span ──
                let tool_span = tracing::info_span!("tool_exec", tool = %call.tool);
                let _tool_enter = tool_span.enter();

                let (result, goal_type) = if call.tool == "task_dispatch" {
                    let (r, gt, exec_steps) = crate::runtime::dispatch::handle_task_dispatch(
                        &mut self.agent,
                        &call,
                        cancel_flag,
                        &mut post_tool_warnings,
                    )
                    .await?;
                    self.agent.steps.extend(exec_steps);
                    (r, Some(gt))
                } else if call.tool == "workflow_validate" {
                    let raw_id = call
                        .params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            crate::NuphusError::agent("workflow_validate: missing id".to_string())
                        })?
                        .to_string();
                    let engine = self.agent.workflow_engine.as_ref().ok_or_else(|| {
                        crate::NuphusError::agent("workflow_engine not injected".to_string())
                    })?;
                    // 热刷新：确保最新工作流被加载
                    {
                        if let Err(e) = engine.read().await.store.load_all().await {
                            tracing::warn!("workflow_validate 热刷新失败: {}", e);
                        }
                    }
                    let engine_r = engine.read().await;
                    let tools_schemas = self.agent.tools.get_schemas();
                    let report = match engine_r.store.get(&raw_id).await {
                        Some(wf) => Compiler::validate_workflow_with_tools(&wf, &tools_schemas),
                        None => {
                            return Err(crate::NuphusError::agent(format!(
                                "Workflow '{}' not found",
                                raw_id
                            )));
                        }
                    };
                    let json = serde_json::json!({
                        "passed": report.passed,
                        "errors": report.errors,
                        "warnings": report.warnings,
                    });
                    (crate::ToolResult::success(json.to_string()), None)
                } else if call.tool == "workflow_run" {
                    let raw_id = call
                        .params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            crate::NuphusError::agent("workflow_run: missing id".to_string())
                        })?
                        .to_string();
                    let inputs: Option<std::collections::HashMap<String, serde_json::Value>> = call
                        .params
                        .get("inputs")
                        .and_then(|v| v.as_object())
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                    let engine = self.agent.workflow_engine.as_ref().ok_or_else(|| {
                        crate::NuphusError::agent("workflow_engine not injected".to_string())
                    })?;
                    // 热刷新：确保最新工作流被加载
                    {
                        if let Err(e) = engine.read().await.store.load_all().await {
                            tracing::warn!("workflow_run 热刷新失败: {}", e);
                        }
                    }
                    // Fuzzy match: exact first, then suffix match
                    let workflow_id = {
                        let engine_r = engine.read().await;
                        let summaries = engine_r.store.list().await;
                        if summaries.iter().any(|s| s.id == raw_id) {
                            raw_id
                        } else {
                            let matched: Vec<_> = summaries
                                .iter()
                                .filter(|s| s.id.contains(&raw_id))
                                .collect();
                            if matched.len() == 1 {
                                matched[0].id.clone()
                            } else if matched.len() > 1 {
                                return Err(crate::NuphusError::agent(format!(
                                    "workflow_run: ambiguous id '{}' matches {} workflows: {}",
                                    raw_id,
                                    matched.len(),
                                    matched
                                        .iter()
                                        .map(|s| s.id.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )));
                            } else {
                                raw_id // let executor report "Workflow not found"
                            }
                        }
                    };
                    // Inject LLM client + ToolRegistry for TalkStep support (write lock briefly)
                    {
                        let mut engine_w = engine.write().await;
                        engine_w.set_llm_client(self.agent.llm.clone());
                        engine_w.set_tools(Arc::new(self.agent.tools.clone()));
                        // chat 步骤 per-step 模型路由（with.model = registry 模型 ID）
                        if let Some(ref factory) = self.agent.client_factory {
                            engine_w.set_client_factory(factory.clone());
                        }
                    }
                    let engine_guard = engine.read().await;
                    let tool_exec = |tool: String, params: serde_json::Value| {
                        let tools = &self.agent.tools;
                        async move {
                            let result = if tool.starts_with("browser_") {
                                tools
                                    .execute_browser_tool(&tool, &params)
                                    .await
                                    .map_err(|e| e.to_string())?
                            } else {
                                tools
                                    .execute(&tool, &params)
                                    .await
                                    .map_err(|e| e.to_string())?
                            };
                            result.into_exec_result()
                        }
                    };
                    let tool_schemas = engine_guard.tools().map(|t| t.get_schemas());
                    let exec_result = engine_guard
                        .execute_workflow(
                            &workflow_id,
                            tool_exec,
                            tool_schemas,
                            self.agent.exec_emitter.as_deref(),
                            inputs,
                            crate::workflow::WorkflowRunSource::Agent {
                                owner: self.config.mode,
                            },
                        )
                        .await;
                    let was_user_cancelled = crate::workflow::hud_control::take_user_cancelled();
                    let (success, mut output, error) = match exec_result {
                        Ok(msg) => (true, Some(msg), None),
                        Err(e) => (false, None, Some(e.to_string())),
                    };
                    if was_user_cancelled {
                        let note = "\n\n[用户已终止此工作流的执行]";
                        output = Some(match output {
                            Some(s) => s + note,
                            None => note.to_string(),
                        });
                        post_tool_warnings.push(
                            "用户已终止工作流的执行。请直接向用户报告结果并结束当前任务，不要重新调用 workflow_run。".to_string(),
                        );
                        if let Some(ref emitter) = self.agent.exec_emitter {
                            emitter.emit(NuphusEvent::HudUpdate {
                                text: "工作流已由用户终止".into(),
                                phase: "warning".into(),
                                step_kind: None,
                            });
                        }
                    }
                    (
                        crate::ToolResult {
                            success,
                            output,
                            error,
                            exit_code: None,
                        },
                        None,
                    )
                } else {
                    // Refresh policy from shared permissions before each tool call
                    if let Ok(fresh) = self.tool_permissions.lock() {
                        self.agent.policy.update_permissions(*fresh);
                    }
                    let r = self
                        .agent
                        .execute_tool_with_permission(&call, &mut post_tool_warnings, cancel_flag)
                        .await?;
                    (r, None)
                };

                let tool_duration = tool_start.elapsed().as_millis() as u64;
                tracing::info!(
                    duration_ms = tool_duration,
                    success = result.success,
                    "tool executed"
                );
                drop(_tool_enter);
                let output_str = result
                    .output
                    .as_deref()
                    .or(result.error.as_deref())
                    .unwrap_or("");
                let preview_limit =
                    if call.tool.starts_with("planner_") || call.tool == "task_dispatch" {
                        5000
                    } else {
                        200
                    };
                if let Some(ref emitter) = self.agent.exec_emitter {
                    emitter.emit(NuphusEvent::ToolCallEnd {
                        call_id: call.id.clone(),
                        tool_name: call.tool.clone(),
                        success: result.success,
                        duration_ms: tool_duration,
                        output_preview: output_str.chars().take(preview_limit).collect(),
                        output_full_size: output_str.len(),
                        is_truncated: output_str.chars().count() > preview_limit,
                        error: result.error.clone(),
                        from_task: false,
                    });
                }

                let step = ExecutionStep {
                    tool: call.tool.clone(),
                    params: call.params.clone(),
                    result: Some(result.clone()),
                    status: if result.success {
                        StepStatus::Success
                    } else {
                        StepStatus::Error
                    },
                    timestamp: chrono::Utc::now(),
                    goal_type: goal_type.map(|g| g.id().to_string()),
                    reasoning: iteration_reasoning.clone(),
                };
                self.agent.steps.push(step.clone());

                // planner_create success → push TaskList
                if call.tool == "planner_create" && result.success {
                    if let Some(ref output) = result.output {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) {
                            if let Some(plan) = parsed.get("plan") {
                                if let Some(tasks) = plan.get("tasks").and_then(|t| t.as_array()) {
                                    let task_items: Vec<TaskItem> = tasks
                                        .iter()
                                        .enumerate()
                                        .filter_map(|(i, t)| {
                                            Some(TaskItem {
                                                id: i + 1,
                                                name: t.get("name")?.as_str()?.to_string(),
                                                status: "pending".to_string(),
                                            })
                                        })
                                        .collect();
                                    if !task_items.is_empty() {
                                        if let Some(ref emitter) = self.agent.exec_emitter {
                                            emitter.emit(NuphusEvent::TaskList {
                                                plan_path: parsed
                                                    .get("plan_path")
                                                    .and_then(|p| p.as_str())
                                                    .unwrap_or("")
                                                    .to_string(),
                                                tasks: task_items,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if result.success {
                    let output_str = result.output.as_deref().unwrap_or("");
                    let display = if call.tool == "Read" {
                        let first_line = output_str.lines().next().unwrap_or("");
                        let line_count = output_str.lines().count();
                        format!("{} ({} lines)", first_line, line_count.saturating_sub(1))
                    } else {
                        output_str.to_string()
                    };
                    self.agent.emit_exec(&display);
                } else {
                    let error_str = result.error.as_deref().unwrap_or("unknown error");
                    self.agent
                        .emit_exec(&format!("// ERROR: {}: {}", call.tool, error_str));
                }

                let is_write = call.tool == "Write";
                if !result.success {
                    if let Some(alert) = protection.check_post_call(&call) {
                        post_tool_warnings.push(alert.to_session_warning());
                    }
                } else {
                    protection.reset_consecutive_errors();
                    if is_write {
                        has_used_write_file = true;
                    }
                }

                if cancel_flag.load(Ordering::SeqCst) {
                    tracing::info!(
                        "[INTERRUPT] Agent cancelled during tool execution, skipping session push"
                    );
                    self.agent.emit_exec("// interrupted");
                    return Ok(crate::AgentOutput {
                        success: false,
                        message: "任务已被用户中断".to_string(),
                        steps: self.agent.steps.clone(),
                        retry_session: None,
                    });
                }

                let filtered_output = result
                    .output
                    .as_ref()
                    .map(|o| {
                        let filtered = crate::filter::ToolOutputFilter::apply(&call.tool, o);
                        // External content: sanitize + injection scan + untrusted boundary (unified entry)
                        crate::security::injection::process_external_output(
                            &call.tool,
                            Some(&call.params),
                            &filtered,
                        )
                    })
                    .unwrap_or_default();

                let filtered_result = crate::ToolResult {
                    success: result.success,
                    output: Some(filtered_output),
                    error: result.error.clone(),
                    exit_code: result.exit_code,
                };

                let result_str = serde_json::to_string(&filtered_result).unwrap_or_else(|e| {
                    tracing::warn!(
                        "Failed to serialize tool result for session: {}, tool={}",
                        e,
                        call.tool
                    );
                    String::new()
                });
                let truncated = crate::utils::truncate_tool_output(&result_str, 8000, &call.tool);

                self.agent
                    .session
                    .push_tool_result(call.id.clone(), truncated, !result.success);
            }
            for w in post_tool_warnings.drain(..) {
                self.agent.session.push_user_internal(w);
            }

            // -- Context hints (large-window models only, > 200K context) --
            let ctx_window = crate::agent::goal_types::get_context_window(&self.agent.config.model);
            if ctx_window > 200_000 {
                let ctx_tokens = self.agent.session.api_input_tokens;

                // Attention dilution warning — once per refine cycle
                if !self.agent.context_hint_shown && ctx_tokens > 200_000 {
                    self.agent.session.push_user_internal(
                        format!(
                            "[系统提示词] 当前上下文实际用量 {}K / 模型窗口 {}K tokens。\n\n注意力权重优先级：系统提示词 > 当前目标 > 推理链路 > 工具输出 > 历史对话。长上下文极易导致系统提示词（Constitution/身份/规则）被稀释遗忘，需主动回顾。\n\n建议立即执行 leader_memory_update 持久化当前阶段、关键决策、阻塞项。此后可继续，复杂子任务请用 task_dispatch 隔离。",
                            ctx_tokens / 1_000,
                            ctx_window / 1_000,
                        )
                    );
                    self.agent.context_hint_shown = true;
                    self.agent.last_memory_hint_at = ctx_tokens;
                }

                // Periodic memory save reminder — every ~100K tokens
                let next_milestone = ((self.agent.last_memory_hint_at / 100_000) + 1) * 100_000;
                if ctx_tokens >= next_milestone {
                    self.agent.session.push_user_internal(
                        format!(
                            "[系统提示词] 当前上下文实际用量 {}K / 模型窗口 {}K tokens。建议执行 leader_memory_update 保存当前阶段的关键决策和进度，避免注意力稀释后丢失上下文。",
                            ctx_tokens / 1_000,
                            ctx_window / 1_000,
                        )
                    );
                    self.agent.last_memory_hint_at = next_milestone;
                }
            }
        }

        tracing::warn!("Max iterations reached");

        if let Some(ref hooks) = self.agent.hooks {
            hooks.run_session_end(&self.agent.session.id, false, "Max iterations reached");
        }
        self.agent
            .session
            .push_assistant_internal(vec![crate::session::ContentBlock::Text {
                text: "达到最大迭代次数".to_string(),
                reasoning: None,
            }]);

        // Emit completion events so frontend can finalize (mood → success → idle)
        if let Some(ref emitter) = self.agent.exec_emitter {
            let total_duration = loop_start.elapsed().as_millis() as u64;
            let tool_calls_count = self.agent.steps.len();
            emitter.emit(NuphusEvent::HudUpdate {
                text: "达到最大迭代次数".into(),
                phase: "done".into(),
                step_kind: None,
            });
            emitter.emit(NuphusEvent::ExecutionCompleted {
                step_index: 0,
                output: crate::agent::events::StepOutput {
                    step_index: 0,
                    result_message: "达到最大迭代次数".into(),
                    artifacts: vec![],
                    tool_calls_count,
                },
                total_duration_ms: total_duration,
                total_calls: tool_calls_count,
            });
            emitter.emit(NuphusEvent::LeaderDone {
                message: "达到最大迭代次数".into(),
            });
        }

        Ok(crate::AgentOutput {
            message: "达到最大迭代次数".to_string(),
            success: false,
            steps: self.agent.steps.clone(),
            retry_session: None,
        })
    }
}

// detect_frustration removed — replaced by LLM thinking-based emotion detection.
// Trigger: src/agent/prompt.rs (build_l2_leader) instructs LLM to read prompts/emotion_guide.md when needed.
