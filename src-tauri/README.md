# Nuphus Tauri 桌面壳（`nuphus-desktop` crate）

> **路径**: `src-tauri/` | **入口**: `src/main.rs` | **Crate**: `nuphus-desktop`

Tauri v2 桌面应用壳，桥接 Rust 核心引擎（`nuphus` crate）与 React 前端。

## 与核心引擎的关系

```
┌────────────────────────────────────┐
│         React 前端 (TypeScript)     │
│   invoke("send_message_cmd", ...)  │
└──────────────┬─────────────────────┘
               │ Tauri IPC
┌──────────────▼─────────────────────┐
│     src-tauri/commands/            │
│     桥接层（无业务逻辑）             │
│       process.rs → send_message_cmd │
└──────────────┬─────────────────────┘
               │ fn call
┌──────────────▼─────────────────────┐
│     nuphus crate (src/)            │
│     Runtime → react_loop()         │
│     核心引擎（全部业务逻辑）         │
└────────────────────────────────────┘
```

**职责边界**：
- **src-tauri**：窗口管理、IPC 命令接收、状态持有、事件转发——纯壳，无业务逻辑
- **src/**：Runtime、Agent、LLM、Memory、Tools、Security——全部业务逻辑

## 入口流程（`src/main.rs`）

1. **浏览器 CDP 环境注入** — 从 `UserPreferences` 读取持久化的外部浏览器 CDP 端点与身份，注入进程环境变量（供 BrowserClient 直连与自愈）
2. **panic hook** — 将 panic 信息持久化到 `~/.nuphus/panic.log`
3. **日志初始化** — 调用 `nuphus::utils::init_logging()`
4. **Tauri 构建** — 注册 2 个插件：
   - `tauri-plugin-dialog`（原生对话框）
   - `tauri-plugin-global-shortcut`（全局快捷键，Ctrl+Q 暂停/恢复、Ctrl+Shift+Q 终止工作流）
5. **setup 回调** — 初始化 AppState + 注册 video 字幕 / PDF render 桥接 + 预创建 capture overlay 窗口
6. **窗口事件** — 主窗口关闭被拦截：**隐藏到托盘而非退出**（`api.prevent_close()` + `window.hide()`）

## 状态管理（`src/state.rs`）

```rust
AppState {
    tools,                    // ToolRegistry
    runtime: Mutex<RuntimeContext>,    // LLM 配置 / 权限 / Agent / 上下文窗口 / refine 阈值
    session: Mutex<SessionState>,      // 会话身份 / 消息去重 / 备份
    execution: Mutex<ExecutionState>,  // 安全审批 / 重试 / 去重队列 / 知识引擎
    llm_config_path,          // 模型注册表路径（providers.toml）
    tool_permissions_path,    // 工具权限持久化路径
    tool_permissions_ref,     // 共享 ToolPermissions（实时策略更新）
    cancel_flag, pause_flag,  // 取消/暂停原子标志
    busy,                     // 执行中锁
    last_process_time, last_completion_time,  // 时间戳
    event_seq,                // 事件序列号
    refine_active,            // refine 进行中标记（StateChecker 跳过 LLM 防竞态）
    workflow_engine,          // WorkflowEngine（RwLock）
    signals,                  // SharedSignals（pause/security/workflow 会话级信号）
    speech,                   // SpeechState（STT 子系统，懒加载）
    mobile_ws_tx, mobile_server_shutdown, mobile_token,  // 移动端服务器状态
    relation_cache,           // 最近一次生效的身份关系配置
}
```

分组子结构：

```rust
RuntimeContext { llm_config, tool_permissions, leader_agent, workflow_agent, model_context_window, refine_threshold }
SessionState   { last_message, last_send_id, session_backup }
ExecutionState { pending_security, pending_retry, completed_send_ids, knowledge_engine }
```

## IPC 命令（`src/commands/`）

### 核心流程：`send_message_cmd`（process.rs:105）

```
并发锁(busy) → 读取 soul/relation/known_paths →
注入记忆上下文 → run_leader_with_config(复用或新建ReactAgent) →
Runtime::react_loop() → 流式事件 → 推送到前端
```

### 命令模块（22 顶层模块 + config/ 子目录 + process/ 子目录）

| 模块 | 说明 | 主要命令 |
|------|------|----------|
| `process.rs` | 消息处理核心 | `send_message_cmd`, `submit_user_message` |
| `process/leader.rs` | Leader 生命周期 | `run_leader_with_config` — Agent 构建/复用 + 记忆注入 + Runtime 启动 |
| `process/session.rs` | 会话管理 | `get_session_info`, `get_chat_history` |
| `process/lifecycle.rs` | 执行生命周期 | `interrupt`, `pause_execution`, `continue_execution`, `append_instruction`, `terminate_execution`, `graceful_stop`, `force_reset`, `is_busy` |
| `process/mode.rs` | 模式切换 | `set_mode` — Leader / Workflow / Custom 模式路由 |
| `process/retry.rs` | 重试机制 | `retry_agent` |
| `process/refine.rs` | 会话提炼 | `execute_session_refine`, `execute_workflow_refine`, `refine_skip` |
| `config/` | 配置与模型 | `configure_llm`, `get_current_config`, `is_llm_configured`, `list_models`, `switch_model`, `get_supported_providers`, `get_context_limit`, `set_capability`, `set_relation`, `update_config_toml` |
| `memory.rs` | 记忆面板 | `list_memories`, `update_memory`, `delete_memory`, `toggle_mark_memory`, `get_memory_stats`, `get_timeline_index_stats`, `get_session_history`, `get_session_detail`, `get_memory_overview`, `submit_execution_rating` |
| `security.rs` | 安全审批 | `approve_once_security`, `approve_session_security`, `reject_security`, `set_tool_permissions` |
| `knowledge.rs` | 知识图谱 | `search_knowledge`, `list_knowledge`, `list_knowledge_tags`, `delete_knowledge`, `get_knowledge_items`, `delete_knowledge_item` |
| `user_input.rs` | 用户输入 | `submit_user_input`, `reject_user_input` |
| `tools.rs` | 工具查询 | `get_tools`, `execute_tool`, `get_desktop_status`, `get_hooks_status` |
| `tenet.rs` | 教导管理 | `get_tenets`, `add_tenet`, `delete_tenet` |
| `skill.rs` | 技能查询 | `skill_install`, `skill_remove`, `skill_list`, `skill_install_git` |
| `workflow.rs` | 工作流面板 | `wf_list`, `wf_delete`, `wf_stop`, `wf_pause`, `wf_resume`, `wf_validate`, `wf_save`, `wf_run`, `wf_tools`, `wf_layout_get`, `wf_layout_save`, `canvas_layout_path` |
| `annotations.rs` | 标注管理 | 关系标注 CRUD |
| `approval.rs` | 审批桥接 | `approve_pending`, `reject_pending`, `get_pending_details` |
| `desktop.rs` | 桌面控制桥接 | `desktop_mouse_position`, `desktop_clipboard_write` |
| `dict_ocr.rs` | OCR 字库 | `dict_ocr_analyze`, `dict_ocr_recognize`, `dict_list`, `dict_load`, `dict_delete`, `dict_ocr_save_char`, `dict_ocr_auto_gaps`, `dict_ocr_list_dicts` |
| `preload.rs` | 模型预载 | `preload_model`, `preload_ocr` |
| `preview.rs` | 文件预览 | `read_file`, `open_path`, `reveal_path` |
| `hud.rs` | HUD 窗口 | `hud_update`, `hud_hide`, `hud_pause`, `hud_resume`, `hud_stop` |
| `mcp.rs` | MCP 管理 | `list_mcp_servers`, `list_mcp_tools` |
| `chat_agent.rs` | ChatAgent 配置 | `chat_agent_list`, `chat_agent_save`, `chat_agent_delete`, `chat_agent_set_active`, `chat_agent_get_active` |
| `custom_agent.rs` | 自定义 Agent | `list_custom_agents`, `save_custom_agent`, `delete_custom_agent`, `get_active_custom_agent`, `set_active_custom_agent` |
| `toolbar.rs` | 工具栏/覆盖层 | `toggle_main_window_topmost`, `finish_startup`, `ensure_overlay`, `overlay_capture_confirm`, `hide_overlay` 等 |
| `export_log.rs` | 日志导出 | `export_error_log` |

另有 `speech/`（STT：stt_start/stt_stop/stt_cancel/stt_status/stt_recognize_file/stt_download_model）、`video/`（视频字幕）、`render/`（PDF 渲染）、`mobile_server`、`relay_client`、`plugin_apps` 等模块在 main.rs invoke_handler 注册（约 177 个命令）。

## Tauri 配置（`tauri.conf.json`）

- 主窗口：1180×768，最小 900×600
- 另有 splash 280×160 + hud 窗口
- 无装饰窗口（`decorations: false`）
- **有 CSP**（default-src/script-src/connect-src/frame-src 等，含 127.0.0.1:18772 白名单）
- 前端 dev URL：`http://localhost:5174`
- 构建产物：`../frontend/dist`

## desktop-api crate

独立 crate 位于 `src-tauri/crates/desktop-api/`，封装底层桌面操作 API（鼠标/键盘/窗口/截图/输入模拟），通过 `send_input` 等方式被核心库调用。与 `src/desktop/` 模块配合实现 Windows 桌面自动化。
