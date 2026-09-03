# Changelog

所有值得注意的变更记录在此文件。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [0.2.5] - 2026-09-03

### Added（内测）
- 录制完成一键「交给 WorkflowAgent 优化」：自动保存画布后关闭并注入 workflow 模式指令（含录制草稿路径 / 工作流 id，由 WorkflowAgent 按设计标准整理为 V2 工作流并覆写跑通）
- 工作流重命名：列表行铅笔入口 + 画布标题双击改名（与当前步骤编辑一并保存）

### Changed
- README 定位句改为「面向个人用户日常编程、办公、自动化工作流的 AI 协作伙伴」（中英同步）

## [0.2.4] - 2026-09-03

### Added（内测）
- 工作流操作录制：桌面点击/滚动/热键/文本/等待/框选动作实时捕获，逐步意图确认后以真实 Action 入画布；浏览器点击走 CDP 捕获生成稳定 CSS 选择器
- 录制完成生成 record-draft JSON（意图/参数/证据），进度持久化支持续录
- 画布增强：连线中点「在此插入」手柄、录制步骤按意图插入、真实步骤编辑

### Changed
- 全局执行闸门 wf_gate_status：任意 Agent / 工作流执行中禁止启动工作流与进入画布；Ui/Schedule/Plugin 统一 WorkflowRunSource
- 输入栏 workflow 扳手菜单：工作流画布（续草稿/新建直达）/ 工作流列表 / 工具箱 Ctrl+U，执行锁态禁用并提示
- 手机连接统一入口与本地网络切换精修：二维码+复制 在左、三段说明在右；header wifi 图标 + 切换确认弹窗
- Agent 模型配置移除 default 档，解析链回 Leader 锚点

### Fixed
- Exec 子任务空交付防护（reasoning-only 索要正文、空响应连续 3 次判失败）；未配置 max_tokens 不再截断长思考（回归源 e22542c）
- 手机「切本地网络一直解析中」：跳转看门狗 6s 自动复位
- 提炼并发锁：refine_active 原子防重 + 双端状态收敛
- 历史加载失败自动重试与 WS 重连补拉顺序（先恢复执行态再对账），断线不再误删本地 pending

## [0.2.3] - 2026-09-02

### Added
- 内置工具页：23 个处理命令（PDF / 图像 / 视频 / 音频 / 语音克隆 / 文档），拖放上传 + 参数表单 + 应用内全屏预览，无需安装外部工具
- 语音克隆（云端 OpenAI 兼容 /audio/speech），音频独立分类、PDF 置末
- Agent 内置工具感知：Leader / WorkflowAgent 提示词声明工具页能力（仅内部机制注册，用户经工具页使用）
- 音效提醒扩展：LLM 执行错误三音下行、重试咚咚中性音（网络重试耗尽不误报失败）

### Changed
- 工具 skill（tools-internal）精简为 Agent 参数手册（23 命令）
- 模型页 custom tab 改名「图像音频配置」，新增语音克隆配置
- README 重构：设计哲学独立成章（Leader / Workflow 两篇），移除「为什么是 Nuphus」整章，开篇改为极简实用主义表述
- cargo fmt 全 workspace 格式规范化

### Fixed
- README 错误信息：版本号、架构图三模式（Free/Plan → Leader/Workflow/Custom）、中继「不转发内容」→「不落盘存储内容」、Rust 构建要求、配置路径（config.toml → providers.toml），含 relay-server/README、relay-usage-policy、config.example.toml 同步

## [0.2.2] - 2026-09-01

### Added
- 交互音效体系：发送 / 会话选中 / 执行完成 / mode·models 切换 / Ctrl+K 面板选择与执行 / 会话台 hover / 点开执行窗口，Web Audio 实时合成零资源
- 安全权限弹窗三音：弹窗出现（审批提示）、批准（确认上行）、拒绝（低沉下行）
- 执行中终止按钮点击提示音（注意/即将中断）
- Ctrl+K 命令面板 hover 事件委托（onMouseOver 覆盖分组与扁平两种渲染），分组模式点击补音效

### Fixed
- 音效音量过低听不见：交互音效峰值提升约 3 倍；AudioContext 首次用户手势预热（避免异步 resume 丢音）
- thinking 指示器显示思考过程而非 agent 正文：只显示 agent 正文输出，无正文显示当前工具调用，不再回退「思考中」
- MobilePage 拓扑状态图形化：两端节点 icon 状态色（绿=就绪/灰=未就绪），连线拆两条独立映射 relay/tunnel 通道，流动/闪烁动画标示阻断段

## [0.2.1] - 2026-08-31

### Added
- Custom Agent 记忆体系补齐：记忆检索放宽（卡片私有 + 项目公共皆可检索，上下文过渡靠记忆承载）、知识库绑定接线（目录/文件读取注入 L1）、提示词缓存同 session 不变（编辑卡片下个 session 生效，换卡 live 刷新）
- 手机端设置视图重构：mode 手风琴直接切换（移除子视图）、模型卡切换胶囊、会话列表直显主视图、网络与连接独立弹窗（header 状态 pill 入口）、新会话弹窗选 mode（set_mode + /new-chat 广播，与桌面统一）
- 桌面会话台执行完成与 HUD 同步弹出（true→false 翻转立即 reveal，清除 10s 渐隐倒计时重新计时）

### Fixed
- 手机端运行时模型显示与桌面输入框不一致（SessionInfo 三个广播点统一改 effective_model，按 mode 解析生效模型）
- 桌面会话台「弹出即隐藏」时序冲突（完成瞬间弹出，不再等鼠标/轮询）

### Changed
- 新会话按钮中性色描边（跟随主题前景色，不抢主色）
- 会话列表标题超长换行、分区间距、滚动条不拦截列表

### Added
- 会话工作台 mode 联动：切换 mode 重载对应会话历史，点击跨 mode 会话自动切换 mode（桌面/手机双端统一）
- 手机端会话列表 mode 铭牌三态：LEADER 蓝 / WORKFLOW 橙 / CUSTOM 紫，与桌面 rail 对齐
- find_image 算法重构：金字塔降采样粗扫 + Top-N 候选 + 原图精扫，多格式模板支持，未命中返回最近候选与诊断

### Fixed
- 会话生命周期解耦：新建对话只回欢迎页消灭空会话（welcome 直发 force_new 创建）；追加判定以后端 busy 为权威，修复追加后执行窗口消失；switch_session 支持跨 mode 原子切换（归档原槽→切 mode→安装目标槽）；启动恢复 current_mode；双端会话台跟随（ShelfUpdated 事件 / 执行中锁定）
- workflow 记忆机制对齐 Leader（append+签名+摘要锚点）并隔离 Leader 记忆注入
- 手机端切换跨 mode 会话报「该会话不属于当前模式」：前端产物过期导致切换请求体缺 mode，重建产物后请求携带会话归属 mode

### Changed
- workflow：L2 方法论精简 + skill 编排核心能力重构，schema 补 DoCall inputs/outputs 文档
- 桌面/浏览器工具描述短句化精简——保留关键参数与硬约束，降低提示词注入开销
- 自动化工作流审批时机明确：构建期确认，运行时不再逐操作弹窗
- title-bar 背景对齐对话区并移除底部分隔线
- cargo fmt 全库格式化

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