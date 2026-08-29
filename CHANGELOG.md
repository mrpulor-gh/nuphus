# Changelog

所有值得注意的变更记录在此文件。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [0.1.10] - 2026-08-29

### Added
- 记忆系统闭环重构：md=工作记忆（leader `memory/{tag}.md` append cap 32K / workflow 快照 overwrite）、SQLite=恒久历史，去双写；L1 注入三段式（md tail + distill 标题 + 记忆导航），L2 注入去重（唯一 system 注入）；session_meta 项目绑定，记忆检索默认当前项目过滤、all_projects=true 逃生
- 会话工作台 hover 预览：agent 最终回复与派生标题互补，预览并入气泡统一整体，指向箭头固定不随高度漂移
- preview:// 沙箱文件运行底座：HTML 游戏/交互 demo 可玩（内联脚本/CDN/同目录资源）+ 跨平台路径识别，主 CSP 不动双层沙箱隔离
- 后台下载引导闭环：阶段收尾信号（pct=null）根治完成仍挂按钮 + 品牌黑白反色

### Fixed
- 传输层 UTF-8 严格校验：`from_utf8_lossy` 静默放行改 `String::from_utf8` 严格校验，失败走既有 retry 显式重试（显式失败优于假成功）
- 紧凑 Markdown（标题与正文间无空行）正文静默丢失：桌面/移动双端标题分支补剩余行递归渲染 + renderToString 回归钉 4 例
- 回复块标题后内容静默丢失：BlockRenderer 标题分支补剩余行递归 MarkdownText 再分块
- 主聊天 max_tokens 兜底 8192：修复长回复末尾被服务端默认值截断 + 缺字段 400
- 启动期模型弹窗切换竞态：providers 加载门控 + provider 初始值去硬编码 + 消灭静默失败
- refine：busy 字段 Arc 化，修复 RefineGuard 借用与 workflow 分支 move 冲突（E0505）
- 仅剩提炼摘要的会话 hover 预览缺失：补 refine 态回退分支
- 移除误提交的 tsc-out.txt 工作产物并加入 .gitignore

### Changed
- 输出纪律：汇报精简（结论/路径/待办/风险前置防截断）+ 路径表述跨平台化
- 隐私声明：阐明 macOS/Linux 明文存储是有意取舍——无跨平台 OS enclave API，诚实降级优于虚构防护

## [0.1.9] - 2026-08-27

### Added
- 开源准备：LICENSE（Apache-2.0）、.gitignore、CONTRIBUTING.md、CHANGELOG.md
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
- 启动时 LLM 配置未及时加载：`main.rs` 增加 `eager-load` 调用，确保 `send_message_cmd` 启动即可找到 API Key 与 providers.toml（解决"启动后第一次对话��模型未配置"）
- refine：提炼期间置位 busy——根除提炼前后对话窗口强刷与会话切换竞态
- HUD：agent_dispatch 投递完成后步数指示不再永远转动（编排结束发终态事件）
- 输入框 mode 锁切后端权威源：界面刷新/热更新后执行中不再误解锁
- 外部 Agent 头像单一实现：状态栏/弹窗/设置 chips/编辑区四处渲染不一致与尺寸偏小一并修正
- shelf：快照保护名单防误杀，重启后可恢复会话不再锐减
- 终止确认弹窗 portal 到 body，修复带 transform 祖先下错位

### Changed
- 调试文件从 git 追踪中移除（nuphus-debug.log、debug_req_body.json）
- 工作流硬编码路径 C:\Users\Administrator\Desktop\ → ~/Desktop/
- SECURITY.md 联系邮箱更新
- 内部设计文档从 git 追踪中移除（docs/archive/）
- **模型配置统一架构重构**：`AuxiliaryModels`(16 字段) → `Capabilities`(3 字段 vision/stt/tts)；所有 LLM 客户端统一通过 `ClientFactory` + `ModelRegistry` 创建；删除角色级（leader/exec/workflow/chat）模型路由，移除 `src-tauri/src/utils.rs::create_llm_client`
- **Anthropic HTTP 传输层合并**：Anthropic Provider 的 HTTP 调用统一走 `ChatCompletionsTransport`；Anthropic 消息格式 parser 保留为独立模块（`transports/anthropic/`），不再需要独立传输层
- license 变更为 Apache-2.0

### Security
- 移除 CI 注释中的 VPS IP（基础设施地址不入仓库）

### Added（0.1.9 功能）
- 外部 Agent 通路：agent_dispatch 单次派发编排（上板 → 窗口捕获 → 确定性投递 → 门铃异步回传）与 nuphus-task 完工上报 CLI
- 外部 Agent 交付物管理：弹窗查看与删除（范围校验 + canonicalize 前缀断言双防线）
- 模型能力 OpenRouter 聚合权威源：上下文窗口/定价贯通桌面/手机/工作流三端


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