# Changelog

所有值得注意的变更记录在此文件。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [0.2.11] - 2026-09-12

### Fixed
- 工作流 / 技能 / MCP 配置在发布版全挂：`workspace_root()` 用编译期宏 `env!("CARGO_MANIFEST_DIR")`，CI 在 GitHub Actions 的 Windows runner 上把仓库检出到 `D:\a\nuphus\nuphus`，这个构建机路径被写死进安装包；用户机上该路径不存在（`os error 3`），D 盘为只读介质时更是 `拒绝访问 (os error 5)`，于是 `WorkflowEngine init 失败`、工作流编辑器保存报红、MCP 配置与整个 plugin 目录失效。现改为运行时解析（`NUPHUS_PLUGIN_DIR` / `NUPHUS_WORKSPACE` → 源码检出 → exe 同级 → 用户数据目录），每个候选做真实写探测，只读位置自动降级到可写目录。
- 顺带修掉三处**绕过**该解析器的重复实现（`src/workflow/store.rs`、`src/skill/registry.rs`、`src-tauri/src/commands/process.rs`）——它们各自又写了一遍 `env!("CARGO_MANIFEST_DIR")`，只修 `workspace_root()` 救不了工作流保存。
- 启动日志误导：`WorkflowEngine initialized at startup` 在初始化失败时也照样打印。现成功才打印；失败日志带解析到的 plugin 根路径与覆盖方法，便于定位。

### Added
- 随包只读资产内嵌 + 首启落盘：内置技能、UI 地图示例、MCP 示例配置、经验样例此前**根本没随安装包分发**（`bundle.resources` 为空），安装版里一个内置技能都看不到。现按显式 allowlist 编译期内嵌（21 个文件 / 139 KB），启动时落盘到可写的 plugin 根；`plugin/.assets-version` 记录版本，升级刷新清单内资产、同版本不重复覆盖，用户自建的 workflows / community 等状态永不触碰。
- 画布工作台可指定目标工作流：工作流列表入口带上 id 直接进入编辑，命令面板「画布」不带目标时由工作台自选，关闭时清空目标，避免沿用上次选中项。

### Changed
- `agent-orchestration` 技能去具体化：正文与示例不再写死具体 Agent 名，改为 `{key}` 占位；字段全集以 `AgentFields`（`src-tauri/src/commands/config/team.rs`）为准，实况一律以 `plugin/team.toml` 当次读取为准，避免文档随平台演进失真。

## [0.2.10] - 2026-09-11

### Added
- 发送受理回执（message_accepted）：消息真实入队 / 进入执行循环时立即广播受理事件并携带调用方 send_id，画布类入口据此即时收起遮罩回到对话，不再等整轮执行结束。
- 画布导出落盘（canvas_export）：导出改为写盘并回传绝对路径，取代静默下载；Downloads 目录经 User Shell Folders 解析。
- 面板巡览工作流示例（examples/workflows/nuphus-tour-v4）：含工作流、参数与图文指引。

### Changed
- 模型绑定与上下文窗口按 provider 段精确解析：客户端暴露 providers.toml 段名，上下文窗口改用 (provider, model) 取值，同名模型跨 provider 段不再错配；原 provider-blind 入口降级为兼容保留。
- 绑定解析新增诊断：区分「未设置」与「已设置却未生效」，避免静默回落 leader 且用户无感。
- 发布说明改从 CHANGELOG 当前版本段落提取，Release 页面展示真实改动内容。

### Fixed
- 发送 / 导出重活期间的界面锁：遮罩拦截指针与键盘、总时长兜底、幂等解锁；遮罩期间重复点击直接忽略，消除界面冻结观感与重复投递。
- 强制终止补齐会话清理与中断收敛事件。
- Exec 路径不再把展示标签 `model (provider)` 当作模型 id 喂给解析器（该误用会使上下文窗口恒回落 128K）。

## [0.2.9] - 2026-09-09

### Added
- 模型连接状态指示器（api-health）：对话页展示 provider 连通状态、稳定时长与异常记录，支持全部 / 连接 / 响应 / 影响分类查看与标记已读。
- 终止方式选择弹窗（StopChoiceDialog）：执行中提供「继续执行 / 优雅终止 / 强制终止」三选一，桌面与手机端共用同一数据源；手机端替换原 window.confirm 误触确认。
- 模型管理页补齐 12 个 Provider 官方图标（anthropic / bytedance / deepseek / google / minimax / moonshotai / openai / opencode-go / openrouter / qwen / xai / zhipuai）。
- 发布流水线上传 updater 签名（.sig）并生成 latest.json 元数据，应用内检查更新链路闭环。
- Edit 工具支持可选行号范围，限制搜索作用域并返回实际匹配位置。

### Changed
- 追加消息队列统一到 shared signals：主执行循环与 ExecAgent 共用同一队列状态，跨组件行为一致。
- 移动端补齐 interrupt 接线与终止文案，与桌面端终止语义对齐。
- Edit 指定范围内发现多个候选时拒绝静默选择，避免重复片段误改。
- 保持模糊匹配下的缩进、Tab 与混合缩进策略，并明确单次替换与全量替换边界。

### Fixed
- 恢复移动端网络相关 locale 键，修复移动端网络设置文案缺失。
- 无效 API Key 变体（invalid_api_key / incorrect api key / api_key 等）纳入可重试判定，避免单次误判即中断会话。
- 修复 Edit 仅行尾空白差异时错误重写替换文本缩进的问题。

## [0.2.8] - 2026-09-08

### Added
- 应用内版本更新：Ctrl+K → 管理 → 检查版本，获取官方发布信息、下载、安装并重启（Tauri updater 签名验证）
- 追加消息队列 UI：输入框右上提示 + hover 面板可删除未消费消息，已消费自动隐藏
- 画布统一工作台：工作流编辑器 / UI 原型设计 / 工具三种编辑工具共用全屏宿主与 Header

### Changed
- 模型 Provider 归属显式化：`[last_model]` 持久化 model→provider，同名模型按运行时配置路由，切换不再串卡
- 追加消息链路修复：主执行循环每迭代 drain 追加队列，执行中追加下轮即注入；新任务清残留防跨任务泄漏
- ExecAgent 继承全局 signals：追加消息与暂停决策透传，与 Leader 行为对齐
- 外部 Agent 主题适配：胶囊 / popover / tooltip 改用语义 token，明暗主题统一

### Fixed
- 强制中断后状态一致：中断事件收敛前端 mood，终止按钮随完成事件即时隐藏
- 撤销无效删除：追加消息已被消费时后端拒绝删除并回读真实队列
- 工作区卫生：cargo fmt 全量格式化 + clippy 门禁清理（doc 缩进、derive、field reassign 等 5 项）

## [0.2.7] - 2026-09-05

### Added
- 画布「意图表单」入口：以阶段（大步骤）+ 子步骤纯文本意图描述工作流，一键发送 WorkflowAgent 依意图探索固化（替代操作录制，无参数、低认知负担）
- WorkflowAgent 对话内 `step_form` 补录：需要用户补同一阶段的多个子步骤时，一次表单收齐（阶段名预填可改，提交 `{stage, steps}`）

### Changed
- 工作流运行失败语义修正：Error 后不再「续跑」（写操作失败续跑=死循环），改为「运行」从头 fresh 执行；仅用户主动暂停（Paused）保留续跑（自动跳过已完成步骤）
- 操作录制（recorder）整体退役：画布录制按钮移除、录制组件与接线删除；其步骤增删/编辑/反馈等非工具交互逻辑已融入画布 CRUD

### Fixed
- 画布步骤操作体验：添加/复制后闪光高亮定位；Inspector 空名称字段级内联报错；删除成功给反馈（可撤销）；关闭 Inspector 时未命名提醒；选中节点操作条常显

## [0.2.6] - 2026-09-05

### Added
- 工具页图片批量缩放：多选图片按目标宽高框等比缩放批量输出到目录（保留纵横比）
- PDF 处理增强：压缩模式（智能 / 指定大小 / 质量档）与压缩前后对比、节省统计；文本提取结果预览与一键复制
- 会话栏书签化：历史会话重构为紧凑书签列表（模式首字母标识），「新建」入口常驻置顶

### Changed
- 新建对话改为后端权威转场（归档当前会话 → 当前模式槽置 None → 清 backup/去重/重试现场），杜绝「新建后旧会话复活」；手机端遥控同走后端转场，桌面端仅本地刷视图
- 输入栏上下文显示克制化：主显示收敛为「已用/上限」数值，cache/步数/时长细节移入 hover 弹窗
- 工具调用 XML 解析增强：闭合标签容忍 `>` 前空白，流式输出残余工具标签碎片按扩展标签集清理
- 欢迎页空闲动效重构（气泡浮动画），MoodFace 内嵌动画路径精简
- 命令面板补齐 combobox/listbox 可访问性语义

### Fixed
- refine 提炼摘要展开不再被 max-height 伪上限隐形截断，完整展示（折叠动画降级为透明度过渡）
- ignore 规则修正：`*.py` 限定根目录（scripts/ 正式图标脚本不再被误标忽略），新增 `/preview-*.html` 设计预览稿忽略

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