//! Nuphus Tauri application entry

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Shelf 退出持久化经 commands::process::shelf::persist_and_mirror（元数据行+镜像一并落盘）
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

mod commands;
mod emitter;
mod handoff_server;
mod mobile_server;
mod models;
mod plugin_apps;
mod relay_client;
mod render;
mod shortcut;
mod speech;
mod splash;
mod state;
mod utils;
mod video;

fn main() {
    // Inject the persisted external-browser CDP endpoint into the process env so
    // future BrowserClient::new() (direct channel) picks it up; the MCP channel
    // gets it via dual::nuphus_mcp_config() at spawn time.
    let prefs = nuphus::config::UserPreferences::load();
    if let Some(url) = &prefs.browser_cdp_url {
        if !url.is_empty() {
            std::env::set_var("NUPHUS_MCP_BROWSER_CDP_URL", url);
            // Identity envs power attach self-healing in BrowserClient::new()
            // (fingerprint windows reopen on a new random debug port).
            if let Some(id) = &prefs.browser_identity {
                std::env::set_var("NUPHUS_BROWSER_NAME", &id.name);
                std::env::set_var("NUPHUS_BROWSER_EXE_PATH", &id.exe_path);
                if let Some(dir) = &id.user_data_dir {
                    std::env::set_var("NUPHUS_BROWSER_USER_DATA_DIR", dir);
                }
            }
        }
    }

    // Install panic hook to persist panic info to file (preserved even if terminal closes)
    let panic_path = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".nuphus")
        .join("panic.log");
    std::panic::set_hook(Box::new(move |info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let msg = format!(
            "[{}] PANIC at {}\n  payload: {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            location,
            payload
        );
        let _ = std::fs::create_dir_all(panic_path.parent().unwrap_or(std::path::Path::new(".")));
        let _ = std::fs::write(&panic_path, msg);
    }));

    // Initialize logging (tracing + file output)
    nuphus::utils::init_logging();

    let app = tauri::Builder::default()
        .manage(state::AppState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        let state = app.state::<crate::state::AppState>();
                        let engine = state.workflow_engine.clone();
                        let signals = state.signals.clone();
                        let key = shortcut.to_string();
                        tauri::async_runtime::spawn(async move {
                            let active_id = nuphus::workflow::hud_control::active_id(&signals);
                            if let Some(id) = active_id {
                                let engine = engine.read().await;
                                // Ctrl+Q = 暂停/继续切换
                                if key.contains('Q') && !key.contains("Shift") {
                                    if engine.is_paused(&id).await {
                                        engine.resume_workflow(&id).await;
                                        tracing::info!("[Hotkey] Ctrl+Q 恢复: {}", id);
                                    } else {
                                        engine.pause_workflow(&id).await;
                                        tracing::info!("[Hotkey] Ctrl+Q 暂停: {}", id);
                                    }
                                }
                                // Ctrl+Shift+Q = 终止
                                else if key.contains('Q') && key.contains("Shift") {
                                    engine.cancel_workflow(&id).await;
                                    nuphus::workflow::hud_control::mark_user_cancelled();
                                    tracing::info!("[Hotkey] Ctrl+Shift+Q 终止: {}", id);
                                }
                            }
                        });
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::list_memories,
            commands::update_memory,
            commands::delete_memory,
            commands::toggle_mark_memory,
            commands::configure_llm,
            commands::clear_provider_api_key,
            commands::switch_model,
            commands::set_relation,
            commands::get_agent_models,
            commands::get_effective_model,
            commands::set_agent_model,
            commands::send_message_cmd,
            commands::preload_model,
            commands::preload_ocr,
            models::bootstrap::vision_models_status,
            commands::approve_once_security,
            commands::approve_session_security,
            commands::reject_security,
            commands::get_tools,
            commands::execute_tool,
            commands::get_memory_stats,
            commands::get_timeline_index_stats,
            commands::get_memory_overview,
            commands::get_session_history,
            commands::get_session_detail,
            commands::get_desktop_status,
            commands::get_hooks_status,
            commands::get_knowledge_items,
            commands::delete_knowledge_item,
            commands::search_knowledge,
            commands::list_knowledge,
            commands::list_knowledge_tags,
            commands::delete_knowledge,
            commands::get_session_info,
            commands::get_chat_history,
            commands::get_current_config,
            commands::is_llm_configured,
            commands::get_tool_permissions,
            commands::get_browser_cdp_url,
            commands::set_browser_cdp_url,
            commands::get_browser_connection,
            commands::test_browser_cdp_url,
            commands::detect_cdp_browsers,
            commands::interrupt,
            commands::pause_execution,
            commands::continue_execution,
            commands::append_instruction,
            commands::terminate_execution,
            commands::graceful_stop,
            commands::force_reset,
            commands::set_mode,
            commands::is_busy,
            commands::list_custom_agents,
            commands::save_custom_agent,
            commands::delete_custom_agent,
            commands::get_active_custom_agent,
            commands::set_active_custom_agent,
            commands::agent_init,
            commands::handoff_ensure,
            commands::agent_status,
            commands::list_agent_statuses,
            commands::list_agent_deliverables,
            commands::list_shelf_sessions,
            commands::switch_session,
            commands::new_chat_session_cmd,
            commands::rename_session_cmd,
            commands::has_resume_candidate,
            commands::resume_latest_session,
            commands::list_external_agents,
            commands::upsert_external_agent,
            commands::delete_external_agent,
            commands::extract_agent_icon,
            commands::set_tool_permissions,
            commands::retry_agent,
            commands::list_models,
            commands::get_default_model,
            commands::test_llm_connection,
            commands::list_provider_models,
            commands::refresh_provider_models,
            commands::get_supported_providers,
            commands::get_capabilities,
            commands::set_capability,
            commands::get_context_limit,
            commands::get_reasoning_effort,
            commands::set_reasoning_effort,
            commands::get_language,
            commands::set_language,
            commands::set_project_dir,
            commands::set_project_bookmarks,
            commands::execute_session_refine,
            commands::refine_skip,
            // -- 移动端局域网 server（默认关闭，设置页开关）--
            mobile_server::mobile_server_start,
            mobile_server::mobile_server_stop,
            mobile_server::mobile_server_status,
            mobile_server::mobile_server_ensure,
            mobile_server::mobile_token_regenerate,
            mobile_server::mobile_password_set,
            relay_client::relay_client_status,
            relay_client::relay_client_set_enabled,
            relay_client::relay_caller_token_rotate,
            commands::get_session_refine_config,
            commands::set_session_refine_config,
            commands::submit_execution_rating,
            commands::approve_pending,
            commands::reject_pending,
            commands::get_pending_details,
            commands::submit_user_input,
            commands::reject_user_input,
            commands::get_tenets,
            commands::add_tenet,
            commands::delete_tenet,
            commands::skill_install,
            commands::skill_install_git,
            commands::skill_remove,
            commands::skill_list,
            // -- MCP 管理（只读） --
            commands::list_mcp_servers,
            commands::list_mcp_tools,
            // -- App Plugin（应用插件体系：安装器 + KV + 主题快照）--
            plugin_apps::plugin_app_install,
            #[cfg(feature = "market")]
            plugin_apps::plugin_market_install,
            plugin_apps::plugin_app_list,
            plugin_apps::plugin_app_uninstall,
            plugin_apps::plugin_app_set_enabled,
            plugin_apps::plugin_app_pack,
            plugin_apps::plugin_kv_get,
            plugin_apps::plugin_kv_set,
            plugin_apps::plugin_kv_delete,
            plugin_apps::plugin_kv_keys,
            plugin_apps::plugin_agent_chat,
            plugin_apps::plugin_workflow_list,
            plugin_apps::plugin_workflow_run,
            plugin_apps::plugin_export_sample,
            plugin_apps::theme_snapshot_save,
            // -- Workflow --
            commands::wf_list,
            commands::wf_delete,
            commands::wf_stop,
            commands::wf_pause,
            commands::wf_resume,
            commands::wf_validate,
            commands::wf_save,
            commands::wf_run,
            commands::wf_tools,
            commands::wf_layout_get,
            commands::wf_layout_save,
            // -- Annotation --
            commands::get_annotations,
            commands::add_annotation,
            commands::update_annotation,
            commands::remove_annotation,
            // -- Chat Agent --
            commands::chat_agent_list,
            commands::chat_agent_save,
            commands::chat_agent_delete,
            commands::chat_agent_set_active,
            commands::chat_agent_get_active,
            commands::chat_agent_list_inline,
            commands::chat_agent_update_inline,
            // -- Desktop 工具直通命令 --
            commands::desktop::desktop_mouse_position,
            commands::desktop::desktop_clipboard_write,
            // -- 字典 OCR 命令 --
            commands::dict_ocr::dict_ocr_analyze,
            commands::dict_ocr::dict_ocr_binarize_preview,
            commands::dict_ocr::dict_ocr_extract,
            commands::dict_ocr::dict_ocr_recognize,
            commands::dict_ocr::dict_ocr_save_char,
            commands::dict_ocr::dict_ocr_auto_gaps,
            commands::dict_ocr::dict_ocr_auto_match,
            commands::dict_ocr::dict_ocr_list_dicts,
            commands::dict_ocr::dict_ocr_identify_segments,
            commands::dict_ocr::dict_remove_char,
            commands::dict_ocr::save_temp_image,
            commands::dict_ocr::read_image_base64,
            commands::dict_ocr::dict_list,
            commands::dict_ocr::dict_load,
            commands::dict_ocr::dict_delete,
            commands::toggle_main_window_topmost,
            commands::finish_startup,
            commands::splash_status_update,
            commands::splash_skip_download,
            // -- 全屏遮罩覆盖窗截图 --
            commands::start_overlay_mask,
            commands::overlay_magnifier_region,
            commands::overlay_capture_confirm,
            commands::overlay_capture_done,
            commands::overlay_capture_cancel,
            commands::overlay_pick_color,
            commands::take_capture_result,
            // -- HUD overlay --
            commands::hud::hud_update,
            commands::hud::hud_hide,
            commands::hud::hud_pause,
            commands::hud::hud_resume,
            commands::hud::hud_stop,
            commands::export_error_log,
            // -- Speech-to-text --
            speech::commands::stt_start,
            speech::commands::stt_stop,
            speech::commands::stt_cancel,
            speech::commands::stt_status,
            speech::commands::stt_recognize_file,
            speech::download::stt_download_model,
            // -- Video subtitle extraction --
            video::commands::video_extract_subtitles,
            // -- 文件预览（AI 回复路径点击） --
            commands::read_file,
            commands::read_file_base64,
            commands::open_path,
            commands::reveal_path,
            // -- Document render service (pdf.js in main webview) --
            render::commands::pdf_render_done,
            render::commands::pdf_render_error,
        ])
        .setup(|app| {
            // ── 便携模式桌面快捷方式自建 ──
            // npm 一键安装 / 手工拷贝的便携 exe 不经安装器 → 无桌面图标，用户找不到。
            // 仅便携模式且 .lnk 不存在时创建一次；NSIS/Program Files 安装自动跳过。
            crate::shortcut::ensure_portable_desktop_shortcut();

            // ── wry 拖放注册修复 ──
            // main 窗口必须以 visible=true 创建，否则 WebView2 内部子窗口未就绪，
            // wry 的 RegisterDragDrop 失败 → 文件拖放失效（禁止光标）。
            // 这里创建后立即隐藏，保持 splash→main 启动流程不变
            // （见 tauri issue #14643 / wry issue #1639）。
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.hide();
            }

            // Register video subtitle pipeline into the nuphus lib tool bridge
            // (single process, fn-pointer injection — no IPC).
            crate::video::commands::init_bridge(app.handle());

            // Register the PDF render service (main-webview pdf.js) into the
            // nuphus lib render bridge — same injection pattern as video.
            crate::render::commands::init_bridge(app.handle());

            // Pre-create capture overlay window (hidden) to eliminate white flash on first use
            if let Err(e) = commands::toolbar::ensure_overlay(app.handle()) {
                tracing::warn!("Failed to pre-create overlay window: {e}");
            }

            // HUD 窗口由 tauri.conf.json 声明，无需手动创建

            // ── 注册工作流全局快捷键（不依赖鼠标）──
            {
                let gs = app.global_shortcut();
                for key in ["Ctrl+Q", "Ctrl+Shift+Q"] {
                    gs.register(key).unwrap_or_else(|e| {
                        tracing::warn!("Failed to register global shortcut {}: {}", key, e);
                    });
                }
                tracing::info!("Global shortcuts: Ctrl+Q(pause/resume) Ctrl+Shift+Q(stop)");
            }

            // Load workflows at startup
            let wf_event_rx = {
                let state = app.state::<crate::state::AppState>();
                tauri::async_runtime::block_on(async {
                    let engine = state.workflow_engine.write().await;
                    if let Err(e) = engine.init().await {
                        tracing::error!("WorkflowEngine init 失败: {}", e);
                    }
                    // Get event receiver (for forwarding to frontend)
                    engine.event_bus().subscribe()
                })
            };
            tracing::info!("WorkflowEngine initialized at startup");
            // Update splash status (事件推送；旧 eval+内联 setStatus 被 CSP 拦从未生效)
            crate::splash::emit_splash_progress(app.handle(), None, "正在启动引擎…");

            // ── 一次性迁移：回填 conversation 条目空 intent/summary（历史 bug 导致
            // 对话全文已存但 FTS/检索命中不到）。幂等，启动时执行一次。──
            match nuphus::store::memory::backfill_conversation_index_fields() {
                Ok(n) if n > 0 => {
                    tracing::info!("conversation 索引字段回填完成: {} 条", n)
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("conversation 索引字段回填失败（降级跳过）: {}", e),
            }

            // ── Eager-load LLM config from disk ──
            // This ensures runtime has the API key in memory and
            // providers.toml is synced for send_message_cmd to find.
            {
                let state = app.state::<crate::state::AppState>();
                commands::config::load_llm_config_from_disk(&state);
            }

            // Update splash status
            crate::splash::emit_splash_progress(app.handle(), None, "准备模型…");

            // ── Inject LLM client into WorkflowEngine (required for ChatAgent Talk steps) ──
            {
                let state = app.state::<crate::state::AppState>();
                let llm_config = state.runtime.lock()
                    .ok()
                    .and_then(|g| g.llm_config.clone())
                    .filter(|c| !c.model.is_empty() && !c.api_key.is_empty());

                if let Some(cfg) = llm_config {
                    let registry = nuphus::config::ModelRegistry::from_single(
                        cfg.model.clone(),
                        cfg.provider.clone(),
                        cfg.api_key.clone(),
                        cfg.base_url.clone(),
                        cfg.reasoning_effort.clone(),
                    );
                    let factory = nuphus::llm::ClientFactory::new(registry);
                    match factory.create_main_client() {
                        Ok(client) => {
                            tauri::async_runtime::block_on(async {
                                let mut engine = state.workflow_engine.write().await;
                                engine.set_llm_client(client);
                                engine.set_tools(std::sync::Arc::new(state.tools.clone()));
                                // 完整 registry 工厂：chat 步骤 with.model 按模型 ID 路由专属 provider
                                if let Ok(full_registry) = nuphus::config::load_registry() {
                                    engine.set_client_factory(nuphus::llm::ClientFactory::new(full_registry));
                                }
                            });
                            tracing::info!("[STARTUP] LLM client + ToolRegistry injected into WorkflowEngine for ChatAgent");
                        }
                        Err(e) => {
                            tracing::warn!("[STARTUP] Failed to create WorkflowEngine LLM client: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("[STARTUP] No LLM config, ChatAgent Talk steps will fail");
                }
            }

            // Wire schedule execution callback (cron → execute_workflow)
            {
                let state = app.state::<crate::state::AppState>();
                let app_handle = app.handle().clone();

                let exec_cb: nuphus::workflow::ScheduleExecCallback = std::sync::Arc::new(move |workflow_id: String| {
                    let app_handle = app_handle.clone();
                    Box::pin(async move {
                        let state = app_handle.state::<crate::state::AppState>();
                        let tools = state.tools.clone();

                        let tool_exec = move |tool: String, params: serde_json::Value| {
                            let tools = tools.clone();
                            async move {
                                // browser_ 工具需走异步入口（ToolRegistry::execute 会拒绝）
                                let result = if tool.starts_with("browser_") {
                                    tools.execute_browser_tool(&tool, &params).await
                                } else {
                                    tools.execute(&tool, &params).await
                                }
                                .map_err(|e| e.to_string())?;
                                result.into_exec_result()
                            }
                        };

                        let engine = state.workflow_engine.read().await;
                        // For scheduled execution, tool schemas are not available (no Tauri state access)
                        // Pass empty vec — ChatAgent steps will work but without tool definitions
                        if let Err(e) = engine.execute_workflow(&workflow_id, tool_exec, Some(vec![]), None, None).await {
                            tracing::error!("[Scheduler] Cron-triggered workflow {} failed: {}", workflow_id, e);
                        }
                    })
                });

                let engine = state.workflow_engine.blocking_write();
                engine.set_schedule_exec_callback(exec_cb);
                drop(engine);

                // 恢复持久化的调度任务（在 tokio runtime 上异步执行）
                let wf_engine = state.workflow_engine.clone();
                tauri::async_runtime::spawn(async move {
                    let engine = wf_engine.read().await;
                    engine.restore_schedules().await;
                });
            }
            tracing::info!("Schedule execution callback wired");

            // ── 外部 Agent 交接门铃（HTTP server，仅 127.0.0.1）──
            // 事件驱动：POST 到达即入 HandoffStore，轮次边界由 react_loop 被动 drain，无轮询。
            // 启动失败内部优雅降级（warn 日志），不阻塞应用启动。
            crate::handoff_server::spawn();

            // ── Session Shelf 预热：磁盘镜像装回内存展示台，rail 列表立即可用 ──
            {
                let state = app.state::<crate::state::AppState>();
                let shelf_locked = state.shelf.lock();
                if let Ok(mut shelf) = shelf_locked {
                    crate::commands::process::shelf::warm_from_disk(&mut shelf);
                    let n = shelf.len();
                    tracing::info!("[Shelf] 预热完成，装载 {n} 个镜像会话");
                }
            }

            // ── 外部 Agent 状态清零：上一轮生命周期的 status.json 一律作废，
            //    状态栏仅显示本轮真实启动且经门铃上报验证过的 agent ──
            crate::commands::config::handoff::reset_all_statuses_at_startup();

            // ── 中继客户端：enabled + 配置完整即启动双回路（外部网络控制桌面）──
            // 出站 WS 连中继服务器，收到任务走 submit_user_message 共享入口（source="relay"）。
            // 断线指数退避重连。（2026-08 起 Pro 体系移除，远程访问对所有配对设备免费）
            crate::relay_client::spawn_relay_loops(app.handle().clone());

            // ── 移动端局域网 server：默认关闭，仅当持久化配置 enabled=true 时自动恢复 ──
            // （上次退出前处于开启状态 → 重启后继续提供服务；token/端口随配置恢复）
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let cfg = crate::mobile_server::load_config();
                    if cfg.enabled {
                        let state = app_handle.state::<crate::state::AppState>();
                        match crate::mobile_server::start_server(&app_handle, &state, cfg.port).await {
                            Ok(status) => tracing::info!("[Mobile] 配置 enabled=true，已自动恢复启动（端口 {}）", status.port),
                            Err(e) => tracing::warn!("[Mobile] 自动恢复启动失败（优雅降级，不影响应用）: {e}"),
                        }
                    }
                });
            }

            // ── 后台预热：embed 模型（Candle）→ STT 识别器 ──
            // 合并为一条独立 OS 线程顺序执行（先 embed 后 STT），避免启动时 CPU/IO 争抢。
            // 不用 tauri::async_runtime::spawn：Embedder::get() 与 recognizer 加载都是
            // 重阻塞调用，跑在 tokio worker 上会阻塞异步调度；且 embed.rs 注释指出模型
            // 未下载场景下 debug 构建会在 reqwest 内部 wait::enter() panic。
            {
                // 预热目标必须是 state.speech.cache 同一实例：先 clone Arc 再 move 进线程
                let stt_cache = std::sync::Arc::clone(
                    &app.state::<crate::state::AppState>().speech.cache,
                );
                let spawn_result = std::thread::Builder::new()
                    .name("preload".to_string())
                    .spawn(move || {
                        // 注意：嵌入模型（bge-small-zh）与视觉模型（OCR/YOLO）不在后台
                        // 线程预热——改由前端 preload_model / preload_ocr 命令阻塞触发，
                        // 以便 splash 展示真实下载进度（后台预热会吞掉进度回调）。

                        // STT 预热仅覆盖本地引擎场景：云端路由（capabilities.stt 可解析）
                        // 不需要本地 recognizer；模型文件未下载时静默跳过，绝不触发下载。
                        // 失败仅 warn 回退懒加载，不影响启动。
                        if crate::speech::cloud::resolve_cloud_config().is_some() {
                            tracing::info!("[Preload] STT preload skipped (cloud route, local recognizer not needed)");
                        } else if let Err(e) = crate::speech::engine::resolve_stt_paths() {
                            tracing::info!("[Preload] STT preload skipped (model not downloaded): {}", e);
                        } else {
                            match stt_cache.get_or_load(crate::speech::commands::USE_ITN) {
                                Ok(_) => tracing::info!("[Preload] STT recognizer loaded"),
                                Err(e) => tracing::warn!("[Preload] STT recognizer preload failed (will lazy-init): {}", e),
                            }
                        }
                    });
                if let Err(e) = spawn_result {
                    // 线程创建失败仅降级为懒加载，不得影响启动
                    tracing::warn!("[Preload] Failed to spawn preload thread (will lazy-init): {}", e);
                }
            }

            // Forward Workflow events to frontend in background
            let app_handle_clone = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = wf_event_rx;
                tracing::info!("Workflow event forwarding task started");
                // Send test event to verify channel
                let test_ok = rx.try_recv().is_err(); // channel is empty -> expect TryRecvError::Empty
                tracing::info!("Workflow event channel ready (rx empty: {})", test_ok);
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            tracing::info!("Forwarding workflow event: {:?}", event);
                            // Try AppHandle::emit
                            let result = app_handle_clone.emit("workflow-event", &event);
                            if let Err(e) = result {
                                // Fallback: try sending via window
                                tracing::warn!("AppHandle emit failed: {}, trying window emit", e);
                                if let Some(window) = app_handle_clone.get_webview_window("main") {
                                    let _ = window.emit("workflow-event", &event);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("Workflow event channel lagged by {} messages", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::error!("Workflow event channel closed");
                            break;
                        }
                    }
                }
                tracing::error!("Workflow event forwarding task ended (channel closed)");
            });
            // Create tray icon
            let icon = app.default_window_icon()
                .cloned()
                .unwrap_or_else(|| {
                    // Fallback: create a 1x1 transparent RGBA icon
                    tauri::image::Image::new_owned(vec![0, 0, 0, 0], 1, 1)
                });
            // Create right-click menu
            let menu = tauri::menu::MenuBuilder::new(app)
                .text("show", "显示")
                .separator()
                .text("quit", "退出")
                .build()?;

            tauri::tray::TrayIconBuilder::new()
                .icon(icon)
                .tooltip("Nuphus - 协同共生桌面助手")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            // 退出前保存当前 session（元数据行 + Shelf 磁盘镜像）
                            if let Some(state) = app.try_state::<crate::state::AppState>() {
                                if let Ok(guard) = state.runtime.lock() {
                                    if let Some(rt) = guard.leader_agent.as_ref() {
                                        crate::commands::process::shelf::persist_and_mirror("leader", rt.session());
                                    }
                                    if let Some(wa) = guard.workflow_agent.as_ref() {
                                        crate::commands::process::shelf::persist_and_mirror("workflow", wa.session());
                                    }
                                }
                            }
                            // 销毁窗口并退出
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.destroy();
                            }
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::{TrayIconEvent, MouseButton, MouseButtonState};
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.unminimize();
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        if let tauri::RunEvent::WindowEvent {
            label,
            event: win_event,
            ..
        } = event
        {
            if label == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = win_event {
                    // 拦截关闭事件：隐藏到托盘而非退出
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            }
        }
    });
}