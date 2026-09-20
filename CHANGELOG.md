# Changelog

所有值得注意的变更记录在此文件。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [0.2.17] - 2026-09-21

### Added
- **外链改用系统浏览器打开**：桌面端 WebView 不处理 `target="_blank"`（点击无反应），现在由 App 层捕获阶段统一接管
  （覆盖聊天消息、设置中心各页、GitHub 贡献者页等所有外链），经 `open_external` 命令交系统默认浏览器；后端仅放行
  http/https、拒绝控制字符与超长 URL。
- **GitHub 贡献者页**（设置中心 · 管理组）：按发布轮次列出社区贡献者、贡献内容与各自 GitHub 主页链接；标题下给出
  「欢迎按实际使用情况提交问题与 PR」的说明与仓库入口。
- **网络请求失败诊断可观测性（第一级）**：模型请求失败时，错误文本与日志（行首 `[net]` 前缀，`RUST_LOG=nuphus::net=warn`）附带本轮 DNS 解析事实（IPv4/IPv6 候选地址族分布、样例 IP、解析耗时）、失败分类（`connect` / `timeout` / `first-chunk-timeout`）、错误链与耗时/重试序号；仅在「只解析到一个地址族」或「解析失败」时给出提示。用于定位「仅 IPv6 / IPv6 半通 / 代理」类环境问题——此前只报 `tcp connect error`，无法判断走的是哪个地址族。仅观测，不改连接顺序、重试策略与超时数值。
- **工作流声明式外部输入**：工作流可在 IR 中声明运行前需要填写的参数（`name` / `type`：string·number·boolean·path·json / `required` / `default` / `description` / `sensitive`），运行确认弹窗与画布「运行」（含 `R` 快捷键）按声明渲染输入表单——默认值预填且可改、必填未填时禁用启动并在字段下方给出原因、敏感项用密码控件且不回显；填写值随运行请求直接传给执行层，步骤模板用 `{{inputs.x}}` 或顶层 `{{x}}` 取值，合并优先级为「显式填写 > 声明默认值 > 非必填缺省不注入」。校验新增两条规则：引用未声明的 `{{inputs.x}}` 报错、声明却全程未被引用给出提示；未声明 `inputs` 的旧工作流行为完全不变（字段缺失即空声明，文件形状与执行链路均不变）。已知边界：断点续连时暂停快照在输入注入之后恢复，本次新填的值对快照中已有的同名变量不生效（续跑保持上下文一致），界面暂不提示；敏感值不进日志、不进事件、界面不回显，但仍会随运行记录的变量快照落盘（加密落盘尚未实现）；输入类型只决定表单控件与提示，执行层不做强制类型转换。
- **输入栏 ctx 详情弹窗支持中文**：`cache` / `tok` / `cap` / `step` / `time` / `ttft` / `speed` 七项小标题改走 i18n，
  中文统一为 2 字（缓存 / 令牌 / 容量 / 步数 / 耗时 / 首字 / 速度），标签列固定最小宽度使数值列对齐；弹窗加宽至
  164px；标签字体改用界面字体族——原等宽字体栈（JetBrains Mono / Consolas）不含中文字形，汉字会落到浏览器兜底字体。
- **会话按项目文件夹分组**（#35 后端 / #38 前端）：会话在诞生点快照归属目录，「会话工作台」按项目文件夹分组展示 —— 组间顺序取书签顺序（可切换为按近期项目），组内按更新时间倒序（可切换为创建时间），「未分组」固定末位，每组条数受全局上限约束；组头提供整理（全部展开 / 全部关闭）、排序与「恢复隐藏项目」菜单。归档＝整个分组连同其会话隐藏，恢复入口只在会话栏菜单里。桌面与移动端共用同一份分组实现（`sessionGroups.ts`）；归属在创建时确定，恢复 / 续聊 / 切目录均不改写。
- **新建对话弹窗**：会话列表首位新增创建入口 ——「会话标题」+「归属项目」常驻列表（末位「浏览本地目录…」，未收藏的目录自动建组）。确认后先切换工作目录、再回到欢迎页；标题与归属在会话诞生（首条消息）时一并生效。
- **外部 Agent 契约红线与派发基线审计**（#37 / #39）：契约与 brief 红线新增「不得擅自改动目标仓库的 git 历史」（commit / push / tag / reset / checkout / switch / rebase / stash），代码改动留在工作区由 Leader 审核后统一提交；派发时可声明目标工作区，记录其 HEAD 作为基线，完工（done / blocked）时比对，出现未派发提交就在唤醒消息里提示。未声明工作区 / 非 git 目录一律不审计（宁缺勿错）。
- **设置中心改版**（#32）：改为「左导航 + 右内容」的分区式布局，子页内嵌于面板；画布与模型保留独立整页宿主。
- **内置浏览器特征规避与启动诊断**（#33）：去掉运行期自动化特征、恢复沙箱默认值，并对 Chrome 启动失败给出分类诊断（含 stderr 尾部与退出状态）。
- **界面路径资产**：新增剪映专业版、VS Code 两份实测路径（`plugin/ui-maps/`），记录反直觉步骤与验证锚点，供后续自动化直接复用。

### Fixed
- **截图（Ctrl+U）产物带遮罩、重影与框选线**：默认截图模式原先在隐藏浮层后重新抓屏，抓到的是正在消失的遮罩层。现统一从冻结帧裁剪（与 OCR / 取色路径一致），背景由 JPEG 改 PNG，DPR 改为每次读取 —— 跨混合 DPI 显示器不再失真。
- **HTML 交付物预览被「非文本类型」分支截走**：`html` / `htm` 自 `preview://` 沙箱底座上线起就进不了 iframe 分支，点击只能看到「已请求系统默认程序打开」占位。
- **Windows 浅色主题下主窗口黑边**（#34）：无边框窗口阴影导致客户区内缩，关闭阴影后客户区铺满窗口矩形；抓边带宽由 8px 收窄为 4px。
- **提炼后会话无法收尾**：提炼的占用状态与主流程的释放互相覆盖，表现为最终回复后立即提炼、再发消息无响应。

### Changed
- **CI 触发策略**：push main 不再跑全量 CI（改为 tag / PR / 手动触发），另增 main 守卫工作流，只在 push main 时检查 `cargo fmt` / `tsc --noEmit` / `prettier --check`（#36）—— 把「本地少跑一次格式化」的代价拦在合入之前，而不是转嫁给之后的每一个 PR。
- 仓库忽略规则修正：`config.toml` 改为仅忽略根目录，不再误伤 `relay-server/.cargo/config.toml`（musl 交叉编译的链接器配置）。

## [0.2.16] - 2026-09-19

### Added
- **模型刷新改为与官方清单同步**：点「刷新」后该服务商段的模型集合与官方 `/v1/models` 对齐——清单内条目按权威链（provider 限定内置表 → OpenRouter 聚合库）覆写能力元数据，官方清单外的条目被移除。模型条目新增来源标记 `source = auto | manual`（旧配置缺该字段按 `auto` 兼容）；经「添加模型」手动加入的条目永不被刷新移除，并在列表标注「官方清单外」。进入服务商页的静默自动同步只增不删。
- **刷新摘要可见性**：刷新后展示「新增 / 更新 / 移除」计数，并列出能力被覆写的模型代号与被移除的代号，便于知情与重加。
- **中转站切换移入内容页**：左侧导航只保留模块入口；中转站（默认段与各 `custom-xxx` 实例）改在自定义模型内容页顶部以 tab 切换，新建入口改为 tab 栏内的行内输入。
- macOS 系统权限检查与引导、Windows 工作流脚本子进程窗口隐藏（#23）。
- macOS 麦克风权限改为被动查询（#26）。

### Fixed
- **DeepSeek 内置模型清单对齐官方 API**（#31）：官方 `/v1/models` 现仅发布 `deepseek-flash` 与 `deepseek-v4-pro`，内置清单同步对齐，旧名保留为别名（存量配置仍能命中元数据）；修正 `deepseek-flash` 的视觉能力标注（实测支持图像输入）；`list_models` 元数据解析改为 provider 限定查找，消除跨服务商同名模型的元数据串味。
- **会话交互与文件路径识别**（#28）：修复同行混排时 Windows 路径丢失、裸域名误报、路径徽标复制语义、滚轮节流波及鼠标滚轮等问题，并降低 scheme 预检的时间复杂度。

## [0.2.15] - 2026-09-16

### Fixed
- **npm 安装后 macOS / Linux 无法启动**（#21）：发布流程在 Windows 上解压 Release 资产（`publish.ps1` 走系统自带 `tar.exe`），而 NTFS 不表达 Unix 可执行位 —— 子包内文件（含 `Nuphus.app/Contents/MacOS/nuphus`）统一落成 `644`，macOS 上 launchd 直接拒绝 spawn（`Launchd job spawn failed` / `permission denied (126)`）；Windows 不依赖 exec 位，不受影响。现启动器在 spawn 前幂等补齐可执行位（已有 x 位则跳过、不写盘），覆盖 app bundle 主二进制、`Frameworks/*.dylib` 与 Linux 裸二进制。
- **自定义服务商首次配置缺 API 接入点**（#20）：custom 没有预设地址，而首次配置界面没有接入点输入框，空地址被传给配置流程，保存后配置不可用。现仅在选择自定义服务商时显示该输入框，要求非空且为 http/https，去首尾空格后落盘；内置服务商仍用预定义地址。
- **同名模型被路由到错误的服务商**（#20）：视觉模型此前只保存模型 ID，跨服务商同名（两个中转站提供同一模型）时按段序取首个命中，请求会打到错误端点。现同时保存 `vision_provider`，按「服务商 + 模型 ID」精确解析；旧配置缺该字段时沿用原查找逻辑。
- **视觉模型与 Provider 半绑定**：前端此前分两次写入 `vision` / `vision_provider`，第二次失败会留下「新 model + 旧 provider」组合 —— 该组合按精确解析必然落空，用户看到的是「提示保存成功但图片用不了」。现收敛为一次读写：要么都更新，要么都不动。
- **多个自定义中转站无法区分**：自建端点此前共用同一个 `custom` 配置段，同名模型必然串台。现每个中转站是独立配置段（`custom-xxx`，实例名唯一且后端校验格式与重复），实例名贯穿模型列表、连通性探测、主模型与 Agent 绑定、视觉解析；`provider_type` 仍是协议类型 `custom`，官方 Provider 的解析路径不变，旧版 `name = "custom"` 配置继续可用。
- **手动开启的视觉能力被自动探测覆盖**：用户在模型行内手动设定后落盘来源标记（`supports_vision_source = "user"`），自动探测让位，不再出现「今天勾上、下次连接又没了」。

### Added
- 模型行内「视觉输入」开关（与上下文窗口同构的行内元数据编辑）：支持图片输入的模型由此进入「图像理解模型」候选列表。候选恢复按能力严格过滤 —— 未开启能力的模型不再混入，否则要等到用户实际发图才失败，坏得更晚、更难查；已保存值脱离候选集单独解析，开关关闭时仍显示真实配置并给出开启路径。
- 自定义中转站实例管理：可创建多个 `custom-xxx` 实例，各自独立配置接入点、密钥与模型，从根上解决「多中转站被迫挤在同一配置段」的问题。

## [0.2.14] - 2026-09-15

### Fixed
- **显示模式 / 缩放切换后 HUD 跑到屏幕外**（#19，macOS）：窗口的物理坐标不随缩放自动重算，而 HUD 的位置是在旧缩放体系下算好的——实测从 3840×2160「looks like 1920×1080」(scale=2) 切回 1920×1080(scale=1) 后，HUD 停在 2x 算出的物理坐标上，在 1x 工作区里完全在屏幕外；后果不只是看不见：连 `screencapture -l <winid>` 都抓不到屏幕外窗口，暂停 / 终止 / 关闭全部失联，只能重启应用找回。现监听 `WindowEvent::ScaleFactorChanged`：未手动拖过就按当前工作区重新贴右下角；拖过则不擅自搬走，但用 `clamp_into_area` 保证仍完整落在可见区内并回写用户位置。顺带抽出 `clamp_into_area` 与 `hud_bottom_right` 共用（工作区比窗口小的退化情形贴左上角，而不是整个消失）。

## [0.2.13] - 2026-09-15

### Fixed
- **流式响应被总超时误杀**（`chat_completions`）：reqwest 的 `.timeout()` 语义是「开始连接 → 响应体读完」的总时长，慢上游（实测 2.97 tok/s）生成 1000 tokens 需 ~337s，必然撞上 300s 被杀 → 静默重试 → 用户看到「卡住后重来」。现流式路径取消总时长约束，改由两个「无数据」界限兜底：首包 60s（请求发出 → 收到响应头）+ 块间 60s（idle），超时向前端推送可见提示后再进入下一次尝试。
- **代理回落方向错误**：`responses` 通道原先无条件套用系统代理；现改为先直连，仅在连接层错误（`is_connect` / `is_timeout`）时才切代理重试，`detect_proxy_url` 由「每次尝试都探测」改为每次请求一次。
- **本地端点被云端超时口径掐死**（#16）：本地 / 局域网推理服务的耗时由硬件决定（实测 20 万 token 上下文提炼 ≈ 5.5 分钟），而代码写死提炼墙钟 60s(Workflow) / 90s(Leader)、客户端总超时 300s。现新增 `provider::is_local_endpoint`（按 `base_url` 主机判定：loopback / 私网 / mDNS，`10.example.com` 这类域名不会误判）与 `LOCAL_TIMEOUT_FLOOR_SECS = 900` 的**下限**语义（配置更高时以配置为准）；本地端点的首包预算、非流式总超时与提炼墙钟按该口径放大，云端行为不变。
- **LLM 重试上限过高**：10 → 3（连接类错误 2 次），且等待期内可响应取消，不再出现「点了中断还在后台重试」。
- **切换模型不作用于运行中的会话**：切换此前只改配置，运行中的会话仍由旧模型作答；现即时绑定当前会话，并把失败原因暴露到界面。
- **上下文提炼走错模式**：提炼会话未按当前 `current_mode` 路由（Leader / Workflow 两条链路混用），且未校验目标会话是否存在；现按模式路由并前置校验。
- **自定义服务商被回落成占位地址**：`get_supported_providers` 的内置默认 `base_url` 被前端当成「用户地址」下发，而 custom 的内置默认是文档占位示例 `https://your-custom-api.com/v1`，于是模型拉取报 `error sending request for url (...your-custom-api.com/v1/models)`；后端在 base_url 缺省时也只回落内置默认、不读 `config.toml` 已存地址。现占位地址提为单一常量，新增 `resolve_effective_base_url`（显式 → 已存配置 → 内置默认），命中占位符即判「未配置」并给出可读提示，四条路径（拉取 / 连接测试 / 切换模型 / 保存配置）统一。
- **项目目录切换的提示会丢失**：切换改走 `SignalState::pending_notices` 共享队列，由 `react_loop` / `workflow_agent` 在轮次边界 drain 一次即消费——执行中（agent 已被 take 出槽）切换也不丢；不再做「每轮比对上次值」的绕路逻辑，无变化就不注入。
- **项目中心入口与交互**：入口从 Ctrl+K 收敛为输入框常驻 chip（显示当前目录名，支持快捷切换与清除），弹窗复用既有 `CompactModal` 版式并改为「选中 → 应用」两段式；`IPC invoke ... failed: Command not found` 之类原始报错不再直接抛给用户（新增友好化映射）。
- **会话工作台改为抽屉式**：默认只显示收起的色块，点击展开左栏（移除 hover 感应区与「执行完成自动弹出」）；列表收紧为单行标题 + 行尾相对时间（刚刚 / N 分钟 / N 小时 / N 天）与「展开其余 N 个会话」折叠行；执行中色块不可点击并浮出提示气泡。
- **HUD 三处实测问题**（#13，macOS）：① Dock 遮挡 —— 定位改用 `Monitor::work_area()`（macOS 即 `NSScreen.visibleFrame`），坐标全程物理像素不做逻辑/物理混算；② Retina 半尺寸 —— `show()` 按逻辑尺寸 × scale 换算后再 `set_size`；③ 拖不动 —— 补 `data-tauri-drag-region="deep"`，后端以坐标比对区分「程序移动 / 用户拖动」，用户拖过之后 `show()` 尊重其位置。顺带：`hud_pause` / `hud_resume` / `hud_stop` 取不到活动工作流时不再静默失败。
- **工作流步骤面板「关了就再也打不开」**（#14）：面板可见性此前等同 `workflowRunSteps` 非空，而 ✕ 直接清空该数组——误关一次既丢数据、又没有任何入口能找回，面板里的暂停 / 终止 / 紧急停止入口也一起消失（工作流停在 wait 步骤时尤其明显）。现 ✕ 改为只「收起」，面板原位保留「工作流 · N 步」胶囊作为恢复入口，并支持 `Ctrl+Shift+W` 收起 / 展开；「终止」按钮补 `await` + 失败 toast（此前是 unhandled rejection，表现就是「点了终止毫无反应」）。
- **中断 / 终止停不掉工作流**（#15）：工作流是在一次 `workflow_run` 工具调用内部执行的，执行期 agent 循环阻塞在该调用里，`cancel_flag` 要等工具返回才被检查——而输入栏「中断」只置该标志，对正在运行（或停在 wait 步骤）的工作流完全无效，还返回 `Ok("Task interrupted")` 连报错都没有。现 `interrupt()` 同时取消活动工作流（`engine.cancel_workflow` + `mark_user_cancelled`），移动端 `/interrupt` 复用同一命令；无活动工作流时不再静默跳过。
- **wait 步骤提示语不做变量替换**（#18）：`do.wait` 是唯一漏掉 `resolve_vars_str` 的 action（chat / script / tool / mcp 都已替换），HUD 与步骤面板把模板占位符原样显示（实测「等待: 已读取到 {{pending_raw}}…」），用户既看不出在等什么、也看不到该步骤本想汇报的内容。
- 本地服务商（`local`）配置 API Key 后所有请求报 `builder error`：`local.rs` 把 `auth_header()` / `auth_prefix()` 声明为空串（语义是「无内置鉴权」），而注入点直接 `format!("{}{}", prefix, key)` 后交给 `reqwest::RequestBuilder::header`，http crate 把空串判为**非法头名**，最终只抛一句 `builder error`（用户侧表现：`IPC invoke list_provider_models failed: 请求失败: builder error`，极易被误判成网络问题）。现统一走新增的 `config::provider::resolve_auth()`：空 key → 完全不携带鉴权头；未声明鉴权方案 → 按 OpenAI 兼容约定补 `authorization: Bearer <key>`；已声明 header 的 Provider（`x-api-key`、`x-goog-api-key` 等无前缀 scheme）逐字不变。覆盖全部 7 个注入点（list-provider-models / model-meta / vision-probe / chat-completions ×2 / responses / vision_ocr）——其中 `chat_completions` 原先**无条件**注入，本地端点带 key 时聊天同样会挂。
  - 语义依据：实测 llama-swap 只认带 `Bearer ` 前缀的形式（裸 key、空 Bearer、无头均 401）；`voice.rs` / `speech/cloud.rs` 早已用 `("authorization", "Bearer ")` 兜底，「未声明鉴权方案 = 走 OpenAI 兼容约定」是本仓库既有语义，不是新约定。
- 本地/自定义端点在无鉴权时无法检测与刷新模型：`fetch_provider_models` / `refresh_provider_models` 一律要求非空 key，Ollama / llama.cpp 这类默认无鉴权的端点必须瞎填一个 key 才能连接。现按 Provider 元数据放行空 key（判据为 `auth_header()` 为空串，另加用户自建的 `custom` 端点），且仍不携带任何鉴权头。
- 模型管理弹窗显示陈旧数据：`ChatPanel` 的 `allModels` 只在 `mode` / `modelName` 变化时重载，打开弹窗并不触发刷新；「同 id 换 provider」（如 custom → local 使用同一个模型 id）时两个依赖都没变，卡片便读旧快照（表现为 Local 卡片显示 0 个模型，实际后端数据正常）。现打开弹窗时强制重拉模型与配置。
- 模型管理弹窗空态把 i18n key 当文案渲染：`ChatPanel.tsx` 引用了并不存在的 `modelManager.noModels`，界面直接显示原始 key 字面量。改用已存在的 `models.noModels`。
- 模型页无法拖动窗口：模型页是全屏覆盖层，盖住了 `TitleBar` 的 `data-tauri-drag-region`，而页头自身没挂拖动区，停留在该页面时窗口拖不动。现于页头补一条占满剩余空白的拖动区。

### Added
- 剪贴板支持粘贴文件与截图：从资源管理器复制的文件、截图工具复制的图片可直接粘贴进输入栏（此前仅支持纯文本）。
- 输入栏常驻「项目目录」chip：显示当前目录名，支持一键快捷切换与清除；项目书签统一落 `preferences.project_bookmarks`（收敛此前两套互不相通的键）。
- 模型页密钥栏对本地服务商开放：可留空直接连接；服务启用鉴权时（llama-swap、带 key 的网关或反向代理）在此填写后再连接。
- 模型页新增「仅保存密钥」按钮：key 此前只在「点击可用模型」那一步经 `configureLlm` 落盘，服务端鉴权失败、可用模型列表为空时，用户填了 key 却找不到任何保存入口。现以该服务商当前模型（否则已配置的首个模型）作为落盘目标，不依赖「连接」是否成功。
- ctx 状态区展示解码速度与首 token 延迟：`TokenUsage` 事件新增可选字段 `gen_tps` / `ttft_ms`（`serde(default, skip_serializing_if)`，字段缺失与旧端反序列化均兼容）。速度按「输出 tokens ÷ 首 token→结束耗时」计算，把网络、排队与 prefill 从解码速度中剥离，与 llama.cpp 报告的 decode 速度同口径；TTFT 单独展示。速度与 TTFT 不占常驻状态栏位，明细在 ctx 悬停弹窗（`ttft` / `speed` 两行）。目前仅 Leader 的流式调用（`react_loop`）产出数据，sub-agent 与 workflow 路径固定为 `None`（后续按需接入）。

### Build
- macOS 开发机上的两处构建缺陷（#17）：① `sync_sherpa_libs()` 只按「体积不同」判陈旧，而 `install_name_tool` 是原地改 load command（体积不变），解耦结果永远刷不到 `target/` 下那两份副本 → `cargo test` / `cargo run` 报 `Library not loaded: @rpath/libonnxruntime.1.27.0.dylib`，判据补「源 mtime 更新」；② arm64 上未签名的 dylib 会被 dyld 直接 SIGKILL（零错误输出，极易误判为测试或代码本身的问题），新增 `ensure_codesign_adhoc()`（先 `codesign --verify` 再决定是否补签，避免无条件重签使 mtime 判据恒真、每次构建白拷 68MB）。

## [0.2.12] - 2026-09-12

### Fixed
- 本地模型后端（llama.cpp / llama-server 等）在上下文提炼后请求全挂：会话内 System 消息（提炼摘要、`push_system` 安全警告、`insert_refine_marker` 分裂锚点、ExecPool 压缩）经 `to_api_messages` 原样以 `system` 角色下发，叠加传输层前置的系统提示词，报文成为 `[system(主提示), system(摘要), user…]`；llama.cpp 的 Jinja 模板硬检查 `System message must be at the beginning` → 确定性 HTTP 500（每次 <50ms 即时失败）。云端各家的模板会静默合并多条 system，所以这个客户端 bug 长期只在本地后端暴露。现改为在序列化层以 `user` 角色下发——会话内 System 是**内容**而非指令，这也正是仓库既有的做法（提炼提示词早就以 internal user 注入）。单点覆盖全部注入路径，不依赖逐个调用点。
- 顺带修掉 Anthropic 侧的一处静默内容丢失：适配器把会话内 system 消息的内容替换成占位串 `[system prompt — see top-level system field]`，导致提炼摘要根本没送到模型（不报错，但上下文实际断裂）。
- 提炼结果未落盘导致的「提炼 → 非干净退出 → 恢复全量 → 再次强制提炼」死循环：提炼成功后只写记忆、不写快照（仅退出钩子/归档时落盘）。现 leader 与 workflow 两条路径提炼后立即持久化元数据行与镜像。
- 提炼后未清零 `api_input_tokens`：强制提炼判据（上下文窗口 80% 阈值）直接读取该字段，旧峰值残留使下一轮必然再次越过阈值——即使请求能发出去也会反复空烧。现两个提炼入口（替换式 / 累积式）均清零。
- LLM 重试日志缺少关键详情：重试分支只输出「请求失败，X 后重试」，不含 HTTP 状态码与错误体（`LLMError::ApiError` 本就携带 `API error {status}: {body}`），确定性的参数/模板错误在日志与界面上完全不可见；且提示文案写死 `Network connection timeout`，真实原因是模板异常也会被报成超时、把人往网络方向带偏。现重试日志、警告事件与失败时的用户可见提示均带上截断后的真实错误。

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