# Changelog

所有值得注意的变更记录在此文件。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added
- 开源准备：LICENSE（MIT）、.gitignore、CONTRIBUTING.md、CHANGELOG.md
- Cargo.toml 元数据补全（repository、homepage、documentation）
- 桌面控制基础设施 desktop-api crate（Win32 + xcap）
- PRIVACY.md：数据分类与隐私策略声明
- CI 新增前端 TypeScript 检查（tsc --noEmit）
- Release CI 流水线（三平台构建 + 自动发布）
- README 新增架构概览图（六层架构 + 数据流）

### Fixed
- is_file_tool 缓存函数扩展支持 Read/Write/Edit/Delete/Copy/Rename/Append/Diff
- HTML 提取中的 regex backreference 替换为独立 pattern（兼容 clippy::invalid_regex）
- 	est_executor_with_real_system_shell UUID 不匹配修复
- 6 个测试修复（provider_kind_env、file_cache_mtime、permission categories、injection regex、unknown_tool 消息、executor）
- Clippy warnings：unsafe 函数补充 #Safety 文档、未处理返回值加 let _=、可 derive 的 impl 替换、dead_code 标注
- SessionDivider 移除冗余流式标签（无样式 + 信息增量为零）
- 模型切换弹窗增加按压态 + loading spinner 反馈
- 语言选择页面修复默认选中态不显示问题
- 启动时 LLM 配置未及时加载：`main.rs` 增加 `eager-load` 调用，确保 `send_message_cmd` 启动即可找到 API Key 与 providers.toml（解决"启动后第一次对话报模型未配置"）

### Changed
- 调试文件从 git 追踪中移除（nuphus-debug.log、debug_req_body.json）
- 工作流硬编码路径 C:\Users\Administrator\Desktop\ → ~/Desktop/
- SECURITY.md 联系邮箱更新
- 内部设计文档从 git 追踪中移除（docs/archive/）
- **模型配置统一架构重构**：`AuxiliaryModels`(16 字段) → `Capabilities`(3 字段 vision/stt/tts)；所有 LLM 客户端统一通过 `ClientFactory` + `ModelRegistry` 创建；删除角色级（leader/exec/workflow/chat）模型路由，移除 `src-tauri/src/utils.rs::create_llm_client`
- **Anthropic HTTP 传输层合并**：Anthropic Provider 的 HTTP 调用统一走 `ChatCompletionsTransport`；Anthropic 消息格式 parser 保留为独立模块（`transports/anthropic/`），不再需要独立传输层


## [0.1.0] - 2026-06

### Added
- 核心 Agent 引擎：ReActAgent + SubTaskLoop
- 多 Provider API 支持（DeepSeek、MiniMax、Kimi、Anthropic、OpenAI）
- 工具系统：文件操作（Read/Write/Edit/Delete）、Web 搜索与提取、浏览器自动化（CDP）、桌面控制
- Tauri 桌面应用壳（src-tauri）
- 工作流引擎（Workflow Executor + Compiler）
- 知识库索引（jieba 分词 + Candle Embedding）
- 权限策略系统（ToolPermissions + PermissionPolicy）
- 外部内容注入检测（InjectionDetector）
- CLI 工具（nuphus-cli）：skill 管理