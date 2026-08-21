---
title: Agent 平台编排
id: agent-orchestration
type: skill
tags: [agent, 外部Agent, 并行, 编排]
---

# Agent 平台编排

> Leader 与外部 Agent 平台协作的通用指南。登记、区分、选协议、定策略。

---

## 零、外部 Agent 登记（team.toml）

不做事前全量扫描。采用**渐进登记**：每使用一个外部 Agent，就在 `plugin/team.toml` 追加一段，逐渐积累出大王电脑上有哪些外部 Agent。

### 段格式

```toml
[claude-code]
type = "terminal"           # terminal | web-ui | desktop — 决定交互协议（§2）
open = "终端执行 claude"     # 打开方式：命令 / exe 路径 / URL
process = "claude.exe"      # 进程名特征，process_list 据此识别是否在跑
note = "可选，一句话备注"
```

**只记稳定事实**：打开方式、窗口类型、进程名。**禁止记** PID / 窗口句柄 / 屏幕坐标等易变值（坐标参数走 `plugin/ui-maps/`，见 §6）。

### 调用决策（轻量，不过度投入认知资源）

1. Read `plugin/team.toml` — 有哪些已登记的 Agent
2. `process_list` 匹配各段 `process` — 哪些**现在活着**（活着优先，省去启动与窗口定位成本）
3. 能力匹配靠 Leader 自身对各平台的内部知识（如 Claude Code 长于代码任务），**不维护历史表现评分**——单次表现不固化成选择规则
4. 遇到未登记的新 Agent → 用完后补登记一段

---

## 零.五、工具选择优先级

任何任务，按以下优先级选择工具路径。上层可行则不降级：

```
1. 后台优先 — API / 系统调用 / 进程通信
   → system_shell、文件读写、process_list
2. 读数据优先 — 能读到的不截图
   → Read、Grep、FilesInfo、web_extract
3. 窗口级优先 — 必须视觉时限定窗口范围
   → desktop_window_screenshot + desktop_vision
4. 前台键鼠 — 最后手段
   → desktop_mouse、desktop_input
   仅用于：浏览器交互（规避检测）、前三层都无法绕过的场景
```

判断标准：每步问自己「这一步能不能用更上层的方式完成？」能 → 上移一层。

---

## 一、内外之分

| | 内部 Agent | 外部 Agent 平台 |
|---|---|---|
| **是什么** | `task_dispatch` 创建的 ExecAgent 子任务 | 独立安装的第三方工具 |
| **怎么调用** | `task_dispatch` 工具 | desktop_input / browser_* / 视觉链路 |
| **验证能力** | 由 Leader 读取产出验证 |

**禁止**：对外部 Agent 使用 `task_dispatch`。

---

## 二、窗口类型 → 交互协议

```
看到目标窗口 → 判断类型
│
├─ 普通终端（非 Agent）
│   └─ system_shell 后端执行
│
├─ Agent 终端（Claude Code / Aider / OpenCode / Codex 等）
│   ├─ 这些 Agent 运行在**独立终端窗口**，非 VS Code 集成终端
│   │   ├─ Claude Code：claude.exe (node)，窗口标题通常 "管理员: Windows PowerShell"
│   │   ├─ Aider / OpenCode / Codex：同理，独立终端
│   │   ⚠️ 不要被 VS Code 编辑器窗口标题误导——Agent 不在编辑器里
│   │
│   ├─ 定位方式（不可跳过）：
│   │   1) process_list 找到 Agent 进程（已登记平台按 team.toml 的 process 字段匹配）→ 记下 PID
│   │   2) 查父进程：`Get-CimInstance Win32_Process` 查其 ParentProcessId
│   │      → Agent 通常作为终端（powershell/cmd）的子进程运行
│   │   3) 查父进程 MainWindowHandle → 匹配 windows_list 定位窗口
│   │      ⚠️ 运行时终端标题常被 Agent 覆写为任务名，不能靠标题匹配
│   │   4) ⚠️ 截图 + OCR 确认窗口内容确实是 Agent（提示符/任务输出）
│   │      ❌ 仅凭进程名+窗口标题 → 大概率发错窗口
│   │      ✅ 父进程链 + MainWindowHandle + OCR 三重确认
│   │
│   └─ 确认后三步法：windows_list → window_activate → desktop_input(text, send: "enter")
│       ├─ 短指令（≤200 字）→ 直接输入自然语言
│       ├─ 长指令（>200 字）→ Write 文件，desktop_input 告知文件路径让 Agent 读取
│       ├─ 启动：优先 Leader 自启（按 team.toml 的 open 字段，system_shell Start-Process 开终端 → 输入启动命令）；自启失败再请用户手动打开
│       ⚠️ 部分 Agent 运行中弹出安全确认，Leader 持续留意
│
├─ Web UI Agent 平台
│   └─ browser_navigate → browser_wait_for(输入框) → snapshot → type/click
│
└─ 桌面应用 Agent
    ├─ 启动后等待加载（桌面应用远慢于终端）
    │   ├─ sleep 15s → 截图 → OCR 确认有界面内容
    │   └─ 无内容 → 等 10s 重试，最多 3 轮
    ├─ 加载完成 → OCR 扫描底部 20% 寻找输入框
    │   ├─ 找到 → 计算坐标 → desktop_mouse click → desktop_input
    │   └─ 未找到 → request_user_input 框选（首次）→ 存入缓存
    └─ 缓存机制：rel = 屏幕坐标 - 窗口起始位置，窗口移动不影响
```

### send 参数

`desktop_input` 的 `send` **默认 `"enter"`**（实现：desktop_executors.rs `unwrap_or("enter")`）。⚠️ 只输入不发送必须**显式** `send: "none"`——省略 send 会直接回车发出。按目标应用类型指定：

- 终端/命令行 → `send: "enter"`
- 即时通讯 → `send: "ctrl+enter"` 或 `send: "shift+enter"`
- 仅输入不发送（先填框观察/追加内容）→ `send: "none"`

---

## 三、并行调度策略

**核心洞察**：Leader 自身串行，但外部 Agent 在独立进程中运行，天生并行。Leader 发完指令即走，多个外部 Agent 可同时运转。

```
时间线示例：
│ 发指令给 Agent A ──→ 发指令给 Agent B ──→ 处理内部任务 ──→ 验证 A ──→ 验证 B │
│         │                  │                  │              │           │
│         └── A 后台跑 ──────┴── B 后台跑 ──────┴── 内部任务 ──┘           │
```

**决策树**：

```
发指令给外部 Agent
│
├─ 后续有内部任务 → 先发指令 → 立即 dispatch → 内部完成后验证外部
├─ 无需确认的结果 → 发送即止
├─ 需确认的结果（如源码修改）
│   ├─ 非 Nuphus 自身源码 → 走 §7 门铃制交接（brief + report + 门铃回传），禁止 sleep 空等轮询
│   └─ Nuphus 自身源码 → 发送即止（避免运行中被打断）
└─ 多个 Agent → 依次发完所有指令 → 先完成的先验证
```

---

## 四、交付验证

外部 Agent 无 Checker，**验证责任在 Leader**：

| 任务类型 | 验证方式 |
|----------|----------|
| 代码修改 | Read + Diff + cargo check |
| 文档产出 | Read 检查完整性 |
| 命令执行 | 读取日志或截图 |
| 视觉操作 | screenshot + OCR |

标准：**产出能被下游直接消费**，不只检查"任务是否执行了"。

---

## 五、失败回退

1. 终端交互失败 → 检查是否实为普通终端 → 回退 `system_shell`
2. Web UI 找不到元素 → 延长 wait_for 超时 → 检查登录态
3. 桌面应用 UI 元素定位失败，按优先级回退：
   - 有文字 → 扩大截图区域重试 OCR
   - 纯图标 → hover + 小范围截图 OCR 获取 tooltip
   - tooltip OCR 不清晰 → `request_user_input(icon_confirm)` 让用户确认（OCR 推断结果预填，用户只需修正）
   - 用户无法确认 → `request_user_input(region)` 框选 → `request_user_input(text)` 填入功能名 → 写入 `ui-maps/`
4. UI 参数缓存失效（窗口布局实质变化）→ 通过验证锚点检测 → 重新执行完整识别流程并更新参数文件
5. 外部 Agent 产出不合格 → 走 §7.3 返工流程（brief 升版本重发，同一会话 ≤3 轮）→ 超限报告用户

---

## 六、UI 元素识别与首次参数定义

桌面应用和网页的按钮、图标、输入框等 UI 元素，首次交互时通过视觉手段识别并**固化为参数文件**，后续自动化直接引用，避免重复分析。

### 6.1 识别策略分层

```
看到 UI 元素
│
├─ 有文字（按钮文本、标签、菜单）
│   └─ 窗口级截图 → desktop_perceive 定位 → 记录坐标+文字
│
├─ 纯图标（无文字，如工具栏图标、Activity Bar）
│   ├─ 第1优先：hover + 小范围截图 OCR
│   │   └─ desktop_mouse hover(图标坐标) → sleep 2s
│   │   → desktop_screenshot(以鼠标为中心，上下50px/左右100~200px)
│   │   → desktop_vision → 从 tooltip 文字推断功能
│   │   ⚠️ 截图范围不超出窗口客户区，避免截到其他窗口
│   │   ⚠️ 已知 tooltip 显示方位时可缩小截图范围提升 OCR 精度
│   │
│   ├─ 第2优先（tooltip OCR 不清晰/不确定）：
│   │   → request_user_input(icon_confirm, icon_path, default_name, ...)
│   │   → 一次表单确认名称+快捷键+坐标，返回结构化 JSON
│   │   → 写入 ui-maps/{应用名}.json
│   │
│   └─ 最后手段（用户也无法确认时）：
│       → request_user_input(region, "框选图标区域")
│       → request_user_input(text, "填入功能名称")
│       → 手动组装后存入参数文件
│
└─ 浏览器网页元素
    └─ browser_snapshot → 获取 ref ID → type/click
        └─ CDP 失败 → 降级为桌面截图+OCR
```

### 6.2 截图前置条件

- **窗口置顶**：目标窗口必须在最前，且不被其他窗口遮挡
- **空间隔离**：Nuphus 自身窗口移出目标区域，防止截到自己的 UI
- **等待充足**：hover 后至少等待 1.5~3s，桌面应用 tooltip 渲染远慢于预期
- **区域约束**：截图坐标 = 鼠标坐标 ± 偏移，且 `[x, y]` 不低于窗口客户区左上角

### 6.3 首次参数定义

首次接入一个新桌面应用/网页时，生成其 UI 布局参数文件，包含：

- **窗口识别**：标题匹配模式、进程名、窗口大小特征
- **按钮/图标**：功能名、屏幕相对坐标（rel = 屏幕坐标 - 窗口起始位置，窗口移动不失效）
- **快捷键**：关联的键盘快捷键
- **验证锚点**：特征颜色/像素，用于运行时校验 UI 未变化

参数文件格式：JSON，存放策略分两种：

- **通用应用**（VS Code、Chrome、终端等，多 workflow 共享）→ `plugin/ui-maps/{应用名}.json`
- **Workflow 专属**（某 workflow 特有的 UI 参数）→ `plugin/workflows/{workflow-name}/` 与 .md/.json 同目录

后续自动化流程中，Leader 先检查 `ui-maps/` 是否有目标应用的参数文件：

- **有缓存** → 读取窗口尺寸/位置 → 校验当前窗口状态
  - 位置/大小匹配 → 直接使用相对坐标，跳过视觉分析
  - 位置/大小不匹配 → `desktop_window_move` + `desktop_window_resize` 恢复到参数文件中记录的尺寸和位置 → 验证锚点校验通过即生效
  - 恢复失败或锚点验证失败 → UI 布局已实质变化 → 重新执行识别流程并更新参数文件
- **无缓存** → 执行本节识别流程，**首先记录窗口的尺寸(x, y, width, height)**，再逐元素定位，生成参数文件后执行任务

核心原则：**窗口状态可控则坐标可信**。窗口移动/缩放不是缓存失效的理由——恢复它即可。只有 UI 布局实质变化（如主题切换、版本升级改变布局）才需要重新识别。

### 6.4 与 request_user_input 的协作

`request_user_input` 作为人类视觉锚定的接口，支持以下 `input_type`：

| 类型 | 用途 | 返回格式 |
|------|------|---------|
| `text` | 敏感文本（API Key/密码/验证码） | 纯文本 |
| `screenshot` | 截取屏幕区域 | `{"path", "region":{x,y,w,h}}` |
| `region` | 框选坐标区域 | `{"region":{x,y,w,h}}` |
| `mouse_pos` | 鼠标点击定位 | `{"pos":{x,y}}` |
| `color` | 取色器 | `{"color":{hex,rgb}}` |
| **`icon_confirm`** | **图标功能确认（复合表单）** | **`{"name","shortcut?","rel_x?","rel_y?","note?"}`** |

`icon_confirm` 专为纯图标功能确认设计，一次交互完成：
- 显示图标预览（自动截取或 Leader 传入 `icon_path`）
- 功能名称输入框（预填 OCR 推断结果 `default_name`）
- 快捷键输入框（可选，预填 `default_shortcut`）
- 相对坐标显示（可选，预填 `rel_x`/`rel_y`）
- 备注输入框（可选）
- 支持「重新截取」按钮更新图标预览

调用示例：
```
request_user_input(
  title="确认图标功能",
  prompt="hover 到 Activity Bar 第3个图标，tooltip 显示 Ctrl+Shift+F",
  input_type="icon_confirm",
  icon_path="C:\\temp\\icon_preview.bmp",
  default_name="搜索",
  default_shortcut="Ctrl+Shift+F",
  rel_x=24,
  rel_y=120
)
→ 返回 {"name":"搜索","shortcut":"Ctrl+Shift+F","rel_x":24,"rel_y":120}
```

**原则**：既然请求用户了就要获取精准信息。`icon_confirm` 用一次表单替代多次来回确认，减少认知负担。

Leader 收到返回值后的处理：

```
result = request_user_input(input_type="icon_confirm", ...)
// result.output 为 JSON 字符串: {"name":"搜索","shortcut":"Ctrl+Shift+F","rel_x":24,"rel_y":120}

parsed = JSON.parse(result.output)
写入 ui-maps/{应用名}.json:
  icons[index] = { name: parsed.name, shortcut: parsed.shortcut, rel_x: parsed.rel_x, rel_y: parsed.rel_y, ... }
```

---

## 七、任务交接闭环（Handoff Protocol）

> 外部协作的核心矛盾：外部 Agent 无质检、无回调能力，Leader 串行不能当看门人。
> 解法 = **门铃制交接**：brief 定义验收标准，report 文档承载结果，HTTP 门铃承载完成信号。

### 7.0 使用定位（先决判断）

**默认内部自行处理**。外发仅三类场景：

1. 任务复杂、内部串行过慢 → 拆分一部分给外部**并行**（配合 §3）
2. 外部平台在某领域能力更强（能力互补）
3. 用户明确指定

决定外发后**选平台**：按 §0 调用决策——Read `plugin/team.toml` + `process_list` 看哪些活着 + 自身内部知识定能力匹配，不查历史评分。

**环节闭环总览**（终点即下次起点）：

```
决策(7.0) → 选平台(team.toml + process_list + 内部知识) → 写brief(7.2) → 派发(§2：发文件路径一句话)
  → 外部执行（开工health自检 + progress签收/中途报）
  → 回传(7.1：门铃三层传输 + 失败降级 + report兜底)
  → 验收(7.3：只信产物不信摘要)
  ├─ 达标   → 归档(7.4)：新平台补登记 team.toml，经验入记忆
  └─ 不达标 → brief 升版本重发 ≤3 轮 → 超限报告用户
```

### 7.1 通信架构（三层传输 + 一条兜底）

```
派发（软通道，现状方式）          回传（硬信号，门铃）
────────────────────────────     ─────────────────────────────────
Leader ─desktop_input/browser──→ 外部 Agent（交互会话，保有上下文）
                                  ↓ 完工/进度
终端类 Agent      → curl 直调门铃（自服务）
本地 Web UI Agent → 页面 fetch 直调（localhost 同源不拦，如 OpenClaw）
公网 Web/桌面类   → 【中继】brief 要求其回复末尾输出结构化完成标记
                    ```handoff {"id":"..","status":"done","summary":".."}```
                    Leader 用 browser_evaluate/DOM 提取后代表其中转
                    ⚠️ 公网页面无法直调 localhost（Chrome PNA 拦截，已实测）
兜底：门铃未响 → 自然验收点查 report 文件 / 终端 OCR（见 7.3）
```

**门铃端点**（运行时内嵌，仅绑 127.0.0.1，地址+令牌见 prompt 环境信息段）：

```
POST http://127.0.0.1:{port}/handoff
Header: X-Handoff-Token: {token}        # 每次运行随机生成，从 prompt 环境信息获取
Body:   {"id":"0728-01","status":"done","summary":"一句话结果","report_path":".nuphus/handoff/0728-01-report.md"}

status ∈ progress | done | blocked
GET /handoff/health → 免令牌自检连通
```

**令牌时效**：令牌每次运行随机生成，**Nuphus 重启即轮换**；门铃队列为内存态，重启清空。跨重启的在途任务处理 → 见 7.3 重启场景。

约束不变：**执行中输入框锁定**（无法向用户澄清）+ **外部 Agent 无主动交互能力**（除门铃外无回调）→ brief 一次写全 + 内置歧义处理策略，验收全自助。

### 7.2 任务简报（brief）

路径约定：`.nuphus/handoff/{id}-brief.md`（{id} = MMDD-序号，如 `0728-01`）。

结构 = 内部 dispatch 五段骨架 + 外发特有契约：

1. **任务定义** — 做什么 + 成功标准
2. **上下文** — 已核实事实（文件:行号）/ 约束 / 依赖
3. **质量基线** — 可验证的通过条件
4. **反模式** — 禁止清单
5. **歧义处理** — 遇到不确定时的默认策略（如：按最合理假设推进并记入报告遗留问题；禁止停摆等待提问）
6. **报告契约** — 结果写入 `.nuphus/handoff/{id}-report.md`，固定四段：✅完成项 / 📄改动文件清单 / 🔍验证证据（命令+输出摘要）/ ⚠️遗留问题
7. **门铃契约**（从 prompt 环境信息复制当前地址+令牌）：
   - 终端类/本地 Web 类：「开工前先 `GET http://127.0.0.1:{port}/handoff/health` 自检连通；开工后报一条 `progress`（"已开工"）作签收；完工后执行 `curl -X POST http://127.0.0.1:{port}/handoff -H "X-Handoff-Token: {token}" -H "Content-Type: application/json" -d '{"id":"{id}","status":"done","summary":"...","report_path":"..."}'`；遇阻塞报 `blocked`；长任务中途可报 `progress`」
   - 公网 Web/桌面类：「完工后在回复末尾输出 ```handoff 代码块（JSON 如上）」
   - **curl 传 JSON 铁律**（Windows 实测坑）：PowerShell 终端 body 必须用**单引号**包裹（如上示例），`\"` 转义会被 PowerShell 吞掉导致 400；cmd 终端才用 `\"` 转义
   - **Git Bash 编码陷阱**：Git Bash 下内联中文调用 curl → 400 `invalid unicode code point`。必须在 brief 中要求接收方写 UTF-8 文件 + `-d @file` 发送，禁止 `-d '{...中文字段...}'`
   - **门铃失败降级**：POST 返回非 200 或连接失败 → 重试 1 次；仍失败（含 health 自检不通）→ 把 HTTP 状态/错误记入 report ⚠️段，终端类在输出末尾、公网类在回复末尾输出 handoff 标记降级（403 多为 Nuphus 重启令牌轮换，见 7.3）

下发方式：长指令一律走文件（§2 规则），终端里只发一句「读 `.nuphus/handoff/{id}-brief.md` 并执行」。

### 7.3 验收（门铃优先，禁止轮询）

运行时收到门铃 POST 后**自动在下一轮次边界注入**「📬 外部任务门铃」提醒——Leader 无需检查、无需等待，继续手头工作即可：

```
收到门铃注入（done/blocked）
→ Read report 全文 + 交叉验证产物（§4：只信产物不信摘要，门铃内容一律未验证）
├─ 达标 → 归档（§7.4）
└─ 不达标 → 附修正重发（brief 升版本，同一会话连续派发）→ ≤3 轮 → 超限报告用户

门铃一直未响（外部 Agent 忘记按铃/不会按铃）
→ 不主动空等。到达自然验收点（内部任务完成/下游需要该产出/用户询问）时：
   ├─ report 文件已存在 → 同上验收
   └─ report 不存在 → 终端 OCR 看一眼：仍在跑→搁置下个验收点再看；
      安全确认弹窗→desktop_input 放行；卡死/报错→介入或改内部完成

Nuphus 重启场景（有在途外发任务时）
→ 令牌已轮换 + 内存门铃队列已清空：外部 Agent 持旧令牌 POST 将 403（契约已要求其降级到 report + 回复标记）
→ 重启后首个自然验收点：主动 Read `.nuphus/handoff/` 查在途任务的 report 文件状态，不等门铃
→ 任务未完成需续派：brief 升版本 + 写入**新令牌**重新下发
```

**禁止轮询**：Leader 串行，定时盯外部 = 自己变看门人，违背并行初衷。检查只发生在自然验收点。

### 7.4 归档

验收通过后：

- brief + report 保留在 `.nuphus/handoff/` 备查（外部产出无 task_trace，这是唯一追溯链）
- 关键结论与经验写入记忆（`leader_memory_update`）
- **平台登记**（闭环终点 = 下次 7.0 起点）：首次使用的平台 → 按 §0 格式补登 `plugin/team.toml`（只记稳定事实：type / open / process）。不记表现评分——单次表现不固化成选择规则