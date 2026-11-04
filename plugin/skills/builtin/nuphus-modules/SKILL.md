---
title: Nuphus 项目模块知识（自包含摘要 · 32 模块）
id: nuphus-modules
type: skill
tags: [nuphus, 项目知识, 模块, 架构, runtime, agent, llm, memory, tools, workflow, browser, desktop, security, config, tauri, frontend, 索引, 摘要]
builtin: true
---

# Nuphus 项目模块知识

> 自包含的 Nuphus 项目模块摘要库：32 个模块的**结构与作用**，一问即答。
> 数据在本技能 `data/` 目录（与技能同目录，自包含）——**不引用仓库 `docs/`**（该目录不被 git 跟踪、不在发布包内）。

---

## 0. 使用协议（给 AI 执行者）

1. `skill_query` 命中本技能 → 命中条目即答案骨架（定位 / 结构 / 功能作用 / 规则与坑），先答再补；
2. 需要行级细节或「现在怎样」→ 读源码（提示词「代码地图」已给目录：`src/` `src-tauri/` `frontend/src/`）；摘要内已带的关键 `文件:行号` 可直接转引；
3. 答案引用 `源码路径:行号`；摘要为 2026-11-04 提炼，可能与最新代码有漂移，行级断言以源码复核为准；
4. 本技能随版本发布（`plugin/skills/builtin/`，编译期内嵌进二进制、首启落盘到用户数据目录），任何装机环境可用。

---

## 1. 项目骨架速答（高频问题的快速答案）

### 1.1 Nuphus 是什么

Tauri v2 + Rust + React 18 的桌面 AI Agent（协同共生桌面助手），「统一 Runtime + ReactAgent」架构。三层职责边界：

| 层 | 目录 | 职责 |
|---|---|---|
| 前端 | `frontend/` | UI、交互、状态展示、i18n、主题 |
| 壳 | `src-tauri/` | 窗口、IPC 命令注册、AppState、事件转发（**无业务逻辑**） |
| 核心 | `src/` | Runtime、Agent、LLM、Memory、Tools、Security（**全部业务逻辑**） |

IPC 层约 177 个命令（实测 268 个命令属性 / 308 注册项）；核心引擎 32+ 子模块。外部能力：LLM Provider 12+ 家多协议、桌面自动化（Win32/UIA/OCR）、浏览器 CDP（chromiumoxide）、外部 Agent（门铃 HTTP）。

### 1.2 Cargo workspace：5 个 crate

| Crate | 路径 | 角色 |
|---|---|---|
| `nuphus` | `src/` | 核心引擎（lib），32+ 模块 |
| `nuphus-desktop` | `src-tauri/` | Tauri 桌面壳（bin `nuphus`） |
| `desktop-api` | `src-tauri/crates/desktop-api/` | 底层桌面 API（鼠标/键盘/窗口/截图/OCR/输入模拟） |
| `nuphus-index` | `crates/nuphus-index/` | 知识库内存索引引擎（扫描/解析/检索） |
| `nuphus-browser` | `crates/nuphus-browser/` | 浏览器自动化（Chrome DevTools Protocol） |

前端是独立 npm 工程（`frontend/`，pnpm）：React 18.3 + TypeScript 5.8 + Vite 6 + @xyflow/react 12（工作流画布）+ pdfjs-dist + Vitest 4。

### 1.3 一次用户请求的生命周期

```
用户输入（主窗口 ChatInputBar）
  → invoke("send_message_cmd")                       [src-tauri/commands/process.rs]
  → busy 并发锁 + 读取 soul/relation/known_paths
  → 注入记忆上下文（memory → store，FTS5 + 语义双通路）
  → run_leader_with_config（复用或新建 ReactAgent）   [process/leader.rs]
  → Runtime::react_loop() —— ReAct 主循环            [src/runtime/react_loop.rs]
        ├─ llm → transports → Provider（SSE 流式）
        ├─ agent::exec_tool —— 五层安全链
        │     ├─ permissions（ToolCategory 授权）
        │     ├─ security（SecurityGuard / 注入检测 / 审批）
        │     └─ tools（ToolRegistry 分发执行）
        ├─ session（上下文窗口管理）
        ├─ memory → store（双通路检索 + 沉淀）
        ├─ skill（按需知识检索，不占 prompt）
        ├─ workflow / browser / desktop / mcp / handoff
        └─ protection（ProtectionGuard 断路器）
  → 流式事件 → event_seq 推送前端（useEvents hook 消费）
```

### 1.4 全局「坑与经验」速查 Top 10（跨模块，均带 `文件:行号`）

| # | 坑 | 位置与结论 |
|---|---|---|
| 1 | `#[cfg(windows)]` 门控缺失即 CI 死刑 | `src-tauri/src/commands/mod.rs:4-8`——capture 模块不门控，非 Windows 平台 81 处 dead_code 被 `clippy -D warnings` 判死（v0.2.26 实测） |
| 2 | 跨平台依赖误放 cfg 段 | `src/Cargo.toml:86`——Windows 专属段误落跨平台依赖 → Linux 编译 unresolved imports |
| 3 | debug 构建未优化计算 | 根 `Cargo.toml:41-49`——PNG 编码 1884ms vs release 13ms，且必须同时关 overflow-checks |
| 4 | Explorer 图标缓存污染 | `src-tauri/src/main.rs:144-158`——cargo build 覆盖 exe 时 Explorer 读到半写 PE，把白纸图标按路径+时间戳缓存，需 SHChangeNotify 自愈 |
| 5 | 退出持久化异步化 | `src-tauri/src/main.rs:36-45`——序列化快照可达数 MB，主线程同步跑会「点了没反应」；看门狗 8s 兜底 |
| 6 | 单实例插件注册顺序 | `src-tauri/Cargo.toml:113-116`——`tauri-plugin-single-instance` 必须第一个注册，否则第二实例先建窗口/抢热键/抢 18771 门铃端口 |
| 7 | windows-sys vs windows crate | `src-tauri/Cargo.toml:118-120`——混用 windows crate 因 windows_core 多版本导致 Param trait 类型冲突 |
| 8 | objc2 版本双轨 | `src-tauri/Cargo.toml:153-157`——block2 0.4 与 objc2-foundation 0.3.2 一线的 0.6.4 并存，macOS 原生遮罩从未编译通过，整体注释未接入 |
| 9 | tower-http 版本对齐 | `src-tauri/Cargo.toml:44`——axum 0.7 对应 tower-http 0.5 系；锁文件里的 0.6 是 axum 0.8 生态，勿对齐 |
| 10 | 前端设计宪法 | `frontend/docs/DESIGN-RATIONALE.md`——重塑前 167 处硬编码 hex + 99 处 rgba、282 处硬编码 px 字号失控；重塑后全量走语义 Token 层，弹窗归一 CompactModal，accent 铁律 |

---

## 2. 模块摘要索引（`data/` 32 份，与本技能同目录）

### 2.1 Rust 核心库（`src/`）

| 摘要文件 | 覆盖模块 | 一句话 |
|---|---|---|
| core-runtime | `src/runtime/` | 统一运行时：单一 Runtime + 三模式（Leader/Workflow/Custom），ReAct 主循环 + 子任务分发 + 防护断路器 |
| core-agent | `src/agent/` | Agent 基座层：ReactAgent 纯状态容器 + L0/L1/L2 提示词工厂 + 五层安全工具执行链 + 事件协议 + 提炼 |
| core-llm | `src/llm/` `src/transports/` `src/api/` | 多协议 LLM 接入层：Transport 分叉点，三协议归一 StreamEvent，ClientFactory 动态装配 |
| core-memory | `src/session/` `src/memory/` `src/store/` | 数据体系：Session 上下文窗口 + SQLite(WAL+FTS5+HNSW) 双通路检索 + TenetStore 不可变教导 |
| core-tools | `src/tools/` | 全部 Agent 能力唯一供给层：ToolDef/executor 分离 + ToolRegistry 统一分发 + 11 个工具域 |
| core-skill | `src/skill/` | 检索式技能系统：plugin/skills 知识包 + 无状态 SkillRegistry，按需检索不占 prompt |
| core-workflow | `src/workflow/` | 声明式工作流引擎：单遍规划→确定性执行，编译期验证 + 13 种步骤 + Cron 调度 + ChatAgent |
| core-desktop | `src/desktop/` `src/desktop_automation/` | 桌面「眼睛与手」：坐标通道（DesktopClient）+ 语义通道（UIA/AX + 风险门） |
| core-browser | `src/browser/` + `crates/nuphus-browser` | 浏览器自动化：chromiumoxide CDP，定位启动、附着自愈重连、Cookie 导入、overlay 脚本托管 |
| core-security | `src/permissions.rs` `src/security/` `src/filter/` `src/hooks/` `src/automation_gate.rs` | 安全体系：四道闸门（授权→审批→注入防护→互斥锁）+ 输出过滤 + Shell 钩子 |
| core-config | `src/config/` | 模型注册表与凭据保险库：providers.toml 唯一权威源，DPAPI 加密 + 文件锁 + 原子写 |
| core-knowledge | `crates/nuphus-index/` | 知识索引引擎：零依赖纯内存 HashMap，扫描 plugin/knowledge/，加权子串 + 严格 AND 评分 |
| core-crosscut | `src/annotation/` `src/api/` `src/cache/` `src/cookies/` `src/state.rs` `src/utils/` 等 34 文件 | 跨领域基础设施：壳→lib 注入桥、共享状态、横切工具、跨进程边界、编译期内嵌资产 |

### 2.2 Tauri 壳（`src-tauri/`）

| 摘要文件 | 覆盖模块 | 一句话 |
|---|---|---|
| tauri-shell | `src-tauri/src/` 根文件 16 个 | 零业务逻辑外壳：进程入口（task CLI 旁路）、窗口/托盘/热键、AppState 唯一装配、事件双推、自愈守卫 |
| tauri-commands | `src-tauri/src/commands/`（67 文件） | IPC 命令层：前端 invoke() 唯一 Rust 落点，薄壳集合（268 命令属性 / 308 注册） |
| tauri-speech | `src-tauri/src/speech/`（10 文件） | STT 双引擎：本地 sherpa-onnx（sense-voice + silero VAD + cpal）与云端 OpenAI 兼容转写 |
| tauri-video | `src-tauri/src/video/`（5 文件） | 视频字幕三级降级：内嵌字幕流 → 平台字幕 → 本地 SenseVoice ASR |
| tauri-render | `src-tauri/src/render/`（2 文件） | PDF 渲染桥：eval 触发前端 pdf.js + Tauri 命令回传 + mpsc 结算，扫描件 OCR 兜底 |
| tauri-desktop-api | `src-tauri/crates/desktop-api/`（28 文件） | 底层桌面 API crate：截图/OCR/YOLO/键鼠/剪贴板/窗口枚举，core 的执行层（effector） |
| tauri-mobile | `mobile_server.rs` `relay_client.rs` `ext_agent/` `handoff_server.rs` | 四个对外通道：手机第二屏、出站中继双回路、外部 Agent 派发、门铃 HTTP |
| tauri-models | `src-tauri/src/models/`（3 文件） | 模型元数据层：视觉模型自愈下载（bootstrap）+ OpenRouter 目录聚合与能力匹配（aggregator） |

### 2.3 前端（`frontend/`）

| 摘要文件 | 覆盖模块 | 一句话 |
|---|---|---|
| fe-overview | `main.tsx` `core/` `ui/` `hooks/` | 前端装配/契约/基础设施层：唯一 IPC 桥、类型契约、横切能力、状态收敛 hooks |
| fe-app-shell | `App.tsx` `layout/` | 主窗口骨架：无 router 状态驱动，15 个 CompactModal 弹窗族 + 浮层编排 + 快捷键路由 |
| fe-chat | `main-window/chat/`（30 源码） | 唯一对话界面：消息渲染管线 + 会话分组 + 轮次导航 + 输入栏 + 执行态可视化（零副本谓词派生） |
| fe-workflow-canvas | `main-window/workflow-canvas/`（97 文件） | 工作流 IR 可视化编辑器：投影 + dagre + xyflow，IrEditOp 管线 + 双层校验 + 草稿存储 |
| fe-workflow-panels | `main-window/workflow/`（21 文件） | 工作流门面：资产管理 + 运行前输入契约 + 参数方案 + 调度历史 + ChatAgent 配置 |
| fe-pages | `main-window/pages/` 等 53 文件 | 设置中心 19 子页 + ModelsPage（4365 行最重单文件）+ 更新机制 + 插件/外部 Agent 页 |
| fe-tools | `main-window/tools/`（8 文件） | 桌面自动化工具前台：悬浮工具条、选区截图、Promise 化捕获 hook、内置文件工具页 |
| fe-mobile | `frontend/src/mobile/`（32 文件） | 手机共生端 PWA：零依赖，REST+WS 共享同一对话与执行态，lan/wan/null 三通道 |
| fe-hud-market-capture | `hud/` `market/` `capture-overlay/` | 三个独立前端单元：HUD 置顶进度浮窗、可摘除插件市场、全屏截图覆盖层 |
| fe-i18n-styles | `locales/` `styles/` 主题链路 | 语言层 + 皮肤层：中英双语、tokens.css 设计 Token 单一来源、三主题/自定义主题/皮肤背景 |
| fe-testing | `src/**/*.test.*`（159 文件） | 测试体系：Vitest+jsdom 五范式（纯函数/RTL/CSS 断言/SSR 快照/回归复现钉） |

---

## 3. 高频问题直接命中示例

| 用户问 | 先读（data/ 文件） |
|---|---|
| Nuphus 整体架构 / 分几层 / 几个 crate | 本技能 §1（项目骨架速答） |
| ReAct 主循环怎么跑 / 死循环怎么防 / 三模式 | core-runtime |
| 提示词分几层 / L0 L1 L2 是什么 / 提炼 | core-agent |
| 模型 Provider 怎么配 / 凭据存哪 / OAuth | core-config |
| 记忆怎么存 / 怎么检索 / 怎么压缩 | core-memory |
| 有哪些内置工具 / 工具怎么注册 / 超时取消 | core-tools |
| 工作流引擎怎么设计 / 有几种步骤 / 调度 | core-workflow |
| 某个 Tauri 命令在哪 / IPC 全貌 / 薄壳纪律 | tauri-commands |
| 窗口/托盘/热键/单实例/启动流程 | tauri-shell |
| 语音识别 / 视频字幕 / PDF 渲染 | tauri-speech · tauri-video · tauri-render |
| 手机端 / 中继 / 外部 Agent / 门铃 | tauri-mobile（前端侧补 fe-mobile） |
| 安全审批 / 注入防护 / 输出过滤 / 互斥锁 | core-security |
| 浏览器自动化 / CDP / 标注 overlay | core-browser |
| 桌面点击 / OCR / UIA / 字库找字 | core-desktop（底层 API 补 tauri-desktop-api） |
| 技能系统本身怎么实现 | core-skill |
| 知识库检索怎么实现 | core-knowledge |
| 前端某页面/弹窗/画布在哪 | fe-pages · fe-app-shell · fe-workflow-canvas |
| 前端怎么跑测试 / mock Tauri / 回归钉 | fe-testing |
| 主题 / i18n / 设计 Token | fe-i18n-styles |

---

## 4. 维护备注

- **提炼来源**：2026-11-04 由本地模块梳理档案（33 份：总览 + core 13 + tauri 8 + frontend 11，快照日期 2026-10-27）蒸馏而成；该档案不被 git 跟踪、不在发布包内，故本技能改为**自包含摘要**，不引用其路径。
- 源档案或代码演进后，需同步更新 `data/` 对应摘要；「结构」列的文件名/行数为快照值，行级断言以源码复核为准。
- 本技能位于 `plugin/skills/builtin/`（git 跟踪，**随版本发布**：经 `src/build.rs` 的 ASSET_SOURCES 编译期内嵌，`src-tauri/src/main.rs` setup 期 `seed_plugin_assets` 落盘到发布版 plugin 根）；改动后需重新编译才会进入新安装包。
- 提示词「已安装技能」段由 `SkillRegistry` 实时扫描生成（`src/agent/prompt.rs:61` `skill_registry_section()`；伙伴面板注入点 `src-tauri/src/commands/side_chat.rs:150`）——新建 skill 目录后下个会话自动出现在该段，无需手改提示词。
