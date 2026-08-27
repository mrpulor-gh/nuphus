---
title: Agent 平台编排
id: agent-orchestration
type: skill
tags: [agent, 外部Agent, 并行, 编排]
---

# Agent 平台编排

> 外部 Agent（Claude Code / OpenCode / Hermes 等独立平台）协作指南。系统提示词已覆盖的（工具描述/annotation/Constitution）不重复，本技能只写外部 Agent 特有流程。

---

## 1. 登记（team.toml）

渐进登记：每用一个外部 Agent，在 `plugin/team.toml` 追加一段。只记稳定事实（mode/launch/process/window_hint/dispatch_steps/note），禁止记 PID/窗口句柄/坐标（坐标走 ui-maps；PID/hwnd 每次启动必变，hwnd 编号还会被 OS 复用给无关窗口——任何把易变事实固化进配置或缓存当派发依据的做法都是错的，进程管理职责归 Leader 当次实况）。

```toml
[opencode]
mode = "embedded"        # background | embedded | standalone | web —— 决定交互协议（§2）
launch = "powershell -NoExit -Command opencode"   # Leader 手动启动命令（见 §2 启动 SOP）
window_hint = "OpenCode" # 窗口标题特征（windows_list 匹配用）
process = "opencode.exe" # 进程名特征（process_list 识别依赖此字段）
```

**双登记路径（实测均有效）**：
- **手改 team.toml**：直接编辑保存即可被运行中系统即时读取（无需重启），适合快速实验与本轮临时接入；
- **配置中心录入**：桌面 设置 → 外部 Agent → 新增（走 `upsert_external_agent`），适合正式归档——会联动生成 handoff 工作目录（read.md/memory.md/status.json），手改路径不生成这些文件（须后续补 `agent_init` 或由首次 dispatch 补建空骨架）。

调用决策：Read team.toml → process_list 看谁活着（活着优先）→ 按平台能力匹配。不维护历史评分。

### 跑通即归档（强制，当轮完成）

新外部 Agent 首次**跑通验证**后，必须当轮登记归档，禁止「以后再用再配」——不归档则下次会话无从知晓其存在与用法，跑通经验直接丢失。

1. **配置中心录入**：桌面 设置 → 外部 Agent → 新增（走 `upsert_external_agent`，字段填全）：
   - `key`：唯一 id（字母数字-_；保存后不可改）
   - `mode`：background / embedded / standalone / web ——窗口分类依据（§2 交互方式由此决定）
   - `display_name` / `icon`：状态栏与人读标识
   - `launch` / `args`：启动命令与参数（Leader 手动启动时使用，§2 启动 SOP）
   - `window_hint`：窗口标题特征（windows_list 匹配；终端类运行时标题常被覆写，hint 应选稳定前缀）
   - `process`：进程名特征（process_list 识别依赖此字段）
   - `cooldown_secs` / `await_timeout_secs` / `timeout_action` / `confirm_keywords`：启动冷却/超时动作/确认词表（缺省有默认值）
   - `description`：职责一句话（路由提示；新 agent 自动同步为其 `.nuphus/handoff/{key}/read.md` 的职责段）
   - `note`：Leader 专属实测备忘（如某热键不生效、某交互必须换路径等一手观察），随配置读取并在派发结果中回显；UI 禁止编辑
2. **落盘核对**：`plugin/team.toml` 出现该段且原有段未被破坏（写回是段级增量）；新 key 联动生成 handoff 工作目录。
3. 完整段示例（实测可用样本）：

```toml
[opencode]
mode = "embedded"        # background | embedded | standalone | web
display_name = "OpenCode"
icon = "terminal"
launch = "powershell -NoExit -Command opencode"
window_hint = "OpenCode"
cooldown_secs = 20
await_timeout_secs = 90
timeout_action = "screenshot_alive"
confirm_keywords = ["allow", "confirm", "proceed", "yes/no", "approve"]
# note = "实测备忘：…（Leader 专属，UI 不可编辑）

[[opencode.dispatch_steps]]
tool = "desktop_window_activate"
with = { hwnd = "{hwnd}" }

[[opencode.dispatch_steps]]
tool = "__sleep"
with = { ms = 500 }

[[opencode.dispatch_steps]]
tool = "desktop_input"
with = { hwnd = "{hwnd}", mode = "type", text = "{message}", send = "none" }

[[opencode.dispatch_steps]]
tool = "desktop_input"
with = { hwnd = "{hwnd}", mode = "hotkey", keys = ["enter"] }
```

### 1.1 dispatch_steps 工具调用序列配置规范（Leader 必懂）

`dispatch_steps` 是派发工具的确定性执行序列：投递时按声明顺序逐条调用桌面工具，**全程不经 LLM**。配错一步，指令就打不进外部 Agent——这是 Leader 配置新 agent 时的第一责任区。

```toml
[[{key}.dispatch_steps]]
tool = "桌面工具名"          # desktop_window_activate / desktop_input / __sleep 等
with = { hwnd = "{hwnd}", … } # 参数表；值中的 {hwnd}/{message} 等占位符投递时渲染
```

**可用占位符**（写在 `with` 值里，执行前替换）：
- `{hwnd}` —— 目标窗口句柄（来自本次启动/捕获实况）
- `{message}` —— 渲染后的投递指令全文
- `{title}` / `{pid}` —— 窗口标题 / 进程 PID（捕获到即有）
- `{task_id}` / `{brief_path}` —— 本次任务号与 brief 绝对路径

**编写规则（实测教训固化）**：
1. **首步必须激活窗口**：`desktop_window_activate` + `{hwnd}`——无激活的后写操作会打到别的窗口；
2. **一行一动作**：一条 step 只做一件事；输入与回车分离（type `send="none"` + 单独 hotkey enter），给 TUI 渲染留节奏；
3. **TUI 后加短等待**：激活后插 `__sleep` 300–500ms 再输入，避免键入抢在焦点切换之前；
4. **message 保持单行英文短指令**（如 "Read {brief_path} and execute it."）：brief 文件承载全部细节——中文长文直输有 IME 上屏不确定性，多行文本会被终端按行拆成多条误执行；
5. **占位符必须原样书写在 with 值中**：漏写 `{hwnd}` 或拼错占位名会导致参数空缺/字面注入，工具报错时应首先检查这里；
6. **禁止在 steps 里塞业务逻辑**：steps 只负责「把 message 送进目标窗口」，任务内容一律走 brief 文件。

**常见故障对照**：

| 症状 | 根因 |
|---|---|
| 工具报 step 失败 hwnd 无效 | agent 已被关闭/PID 过期 → 重走 §2 启动 SOP 取新窗口 |
| 终端出现 `{hwnd}` 字面文本 | 占位符拼写错误或该变量本轮未提供 |
| 指令只进去一半 | 多行文本被终端逐行执行 → 改单行 message + 走文件 |
| 输入后 TUI 无反应 | 未等 TUI 就绪就发送 → 首步激活与输入之间补 `__sleep` |

---

## 2. 交互协议

### 窗口分类（先分类，再选交互方式——禁止跨类操作）

| 类别 | team.toml 标识 | 窗口特征 | 正确交互 | 明令禁止 |
|---|---|---|---|---|
| 终端类 | mode=background、embedded | 控制台/TUI（conhost、Windows Terminal），无任何 GUI 控件 | 激活窗口后 desktop_input **直接打字**，`enter` 发送；OCR 仅用于读回显确认响应（§2 窗口定位）| ❌ 找输入框/按钮等控件；❌ 走 §6 ui-maps 控件定位 |
| Web 类 | mode=web | 浏览器/WebView 渲染的页面 | 按 §6 定位页面输入框（textarea/contenteditable），send 多为 `ctrl+enter` | ❌ 当终端直打 |
| 桌面类 | mode=standalone | 原生 GUI 应用（独立输入框/按钮） | 有 ui-maps 缓存 → 按 §6 缓存定位；**无缓存 → 首次必须先视觉分析定位（见下）** | ❌ 未定位就盲打坐标 |

判错代价（实测教训）：对终端窗口执行「找输入框」会无限空转——终端根本没有该控件，轻则浪费轮次，重则误点窗口内文本导致 TUI 进入意外状态。

**桌面类无 ui-maps 首跑流程（强制）**：窗口激活 → 截图 + vision 读屏（识别窗口布局/目标控件语义位置）→ `desktop_perceive` 精确定位控件坐标与可交互性 → 确认后才执行操作 → **当轮把定位结论写入 `plugin/ui-maps/{应用名}.json`**（window 定位 + interact 步骤 + verify_anchor，格式见 §6）。视觉分析只做一次，之后一律走缓存；布局实质变化才重新识别。

### 窗口定位（terminal 类，不可跳过）

```
1. process_list 找 Agent 进程（按 team.toml process 字段）→ 记 PID
2. 查父进程（Get-CimInstance Win32_Process）→ 父进程 MainWindowHandle → windows_list 定位
3. 截图 + OCR 确认是 Agent（提示符/任务输出）——运行时标题常被覆写（如 OpenCode 显示「OC | 任务名」），不能靠标题
```

**宿主归属注意（实测）**：TUI 进程自身（如 opencode.exe / powershell.exe）的 MainWindowHandle 可能为 0——顶层窗口可能宿主在 Windows Terminal 或 conhost 名下，也可能正在启动中尚未建窗。判定顺序：先 windows_list 全表扫 window_hint/标题特征；无果再等 5–10s 重查一次（冷启动 TUI 渲染需要时间）；仍无果按 §5 回退用 Start-Process `-WindowStyle Normal` 重新拉起。**禁止凭进程存在就认定「窗口存在」，也禁止把任何缓存句柄当激活目标——每次以当次枚举实况为准。**

有 `plugin/ui-maps/{应用名}.json` 缓存 → 按缓存的 locate/default_pos 直接定位，校验窗口状态后使用（见 §6）。

### 发送

- 短指令（≤200 字）→ desktop_input 直接输入，send 按目标：终端 `enter` / 即时通讯 `ctrl+enter` / 仅输入 `none`
- 长指令（>200 字）→ Write 文件，desktop_input 只发「读 {文件路径} 并执行」

### 派发闭环（首次建立共识，后续直接派）

**闭环 = 派发 → 门铃 → 验收**，每轮正式任务都走。读规则只做一次：

```
首次派发（新会话 / agent 无上下文）：
  先走读规则环——派「读 {brief} 并回报关键理解」→ agent 门铃 progress（含理解要点）
  → Leader 验收理解一致（不符 → 返工重读）→ 通过即建立共识

同会话后续派发：
  共识已建立，直接派正式任务 → agent 执行 → 门铃 done/blocked → 验收（§7.4）
```

首次通过后即共识，禁止：用 OCR 盯 Read 动作（盯梢不是闭环）。

### 启动（Leader 主导四步 SOP——外部 Agent 的第一步动作）

**Step 1 读配置与注意事项**：Read team.toml 对应段 → 记住 `launch` 启动命令、`window_hint`、`process`、`confirm_keywords` 与 `note` 实测备忘（逐条记住——投递方式可能因此不同）。

**Step 2 查已有实例（复用优先）**：process_list 按 `process` 字段查活进程；windows_list 按 `window_hint` 扫窗口。有且健康 → 直接记下 PID/hwnd 进入 Step 4。

**Step 3 首轮手动启动并记录 PID**：无实例才执行——`system_shell` 用 `Start-Process` 以 **`-WindowStyle Normal` 显式带窗启动** `launch` 命令；等待窗口出现后记录 PID 与 hwnd。禁止依赖工具内部的隐式冷启动或历史缓存句柄（易变事实不可作为派发依据，§1 铁律）。自启失败请用户手动打开。

**Step 4 首次握手验证状态通路**：派发前先让外部 Agent 读取协议文件（`.nuphus/handoff/{agent}/read.md`）并按其中契约回报一次门铃事件（progress 含关键理解）——收到即证明「投递通道 + 门铃回传」双向打通，再进入正式任务（§7.0）。通路未验证就发正式任务 = 状态盲飞。

---

## 3. 并行调度

外部 Agent 独立进程天生并行。Leader 发完即走：

```
├─ 后续有内部任务 → 先发指令 → 立即 dispatch → 内部完成后验证外部
├─ 无需确认的结果 → 发送即止
├─ 需确认的结果（如源码修改）→ 一律走 §7 门铃交接（含 Nuphus 自身源码：发送即止不打断，完工经门铃/自然验收点收 report）
└─ 多个 Agent → 依次发完 → 先完成的先验证
```

---

## 4. 交付验证

外部 Agent 无 Checker，验证责任在 Leader（只信产物不信摘要）：

| 任务类型 | 验证方式 |
|----------|----------|
| 代码修改 | Read + Diff + cargo check |
| 文档产出 | Read 检查完整性 |
| 命令执行 | 读取日志或截图 |
| 视觉操作 | screenshot + OCR |

标准：产出能被下游直接消费。

---

## 5. 失败回退

1. 终端交互失败 → 确认是否实为普通终端 → 回退 system_shell
2. Web UI 找不到元素 → 延长 wait_for 超时 → 检查登录态
3. 桌面 UI 定位失败 → 有文字扩 OCR / 纯图标 hover+tooltip OCR / 不清晰走 icon_confirm 用户确认（见 §6）
4. ui-maps 缓存失效（布局实质变化）→ 重新识别并更新参数文件
5. 产出不合格 → §7.4 返工（brief 升版本重发 ≤3 轮）→ 超限报告用户
6. **agent_dispatch 工具超时/失败接管 SOP**（实测有效）：① Read `.nuphus/handoff/{agent}/status.json` + briefs/ —— 确认上板是否已完成（brief 存在即算）；② process_list/windows_list 按 team 配置核对进程与窗口实况；③ 已上板但投递未完成 → 直接 `desktop_window_activate` 激活窗口后 `desktop_input` 直输「Read {brief_path} and execute it.」补完投递；④ 进程已死或从未启动 → 重走 §2 启动 SOP；⑤ 全程以文件与实况为准，禁止凭工具报错文本猜根因。

---

## 6. UI 识别与 ui-maps

### 识别策略

```
有文字 → 窗口级截图 → desktop_perceive 定位
纯图标 → hover + 小范围截图 OCR tooltip → 不清晰 → request_user_input(icon_confirm)
最后手段 → request_user_input(region/text) 用户框选
```

截图前置：窗口置顶、空间隔离（Nuphus 窗口移出目标区）、hover 后等 1.5~3s。

### ui-maps 缓存

- 位置：`plugin/ui-maps/{应用名}.json`（通用应用）或 `plugin/workflows/{workflow}/`（workflow 专属）
- 内容：window 定位（process/parent/locate/title_note）+ interact 步骤 + verify_anchor
- 使用：有缓存 → 恢复窗口尺寸/位置 → 锚点校验通过即用；布局实质变化才重新识别
- 核心：**窗口状态可控则坐标可信**。窗口移动/缩放不是缓存失效理由——恢复它即可

### 纯图标确认

走 `icon_confirm`（工具内置表单，字段级一次一问），返回值写 ui-maps。

---

## 7. 任务交接闭环（Handoff Protocol）

> 外部 Agent 无质检无回调，Leader 不能当看门人。解法 = **门铃制**：brief 定义标准，report 承载结果，HTTP 门铃承载完成信号。

**机制边界（先分清谁做什么）**：

- **状态 = 机制自动**：agent 门铃 POST（progress/done/blocked）→ 后端自动写 `status.json` → 前端状态栏自动轮询显示。**Leader 零触发、不手动改 status.json**。
- **Leader 只做四件事**：①启动/复用外部 Agent 并派发（§2 启动 SOP + agent_dispatch）②首次握手验收理解与通路（读协议环，§7.0 步骤2）③每轮收门铃后验收产物（§7.4）④归档（§7.6）。

### 7.0 Leader 派发操作要求（无上下文时照此执行）

**状态机制**：agent 门铃 POST → 后端自动写 status.json → 前端状态栏自动轮询。Leader 零触发、不手动改 status.json。

**Leader 完整操作序列**：

```
0. 前置：外部 Agent 已按 §2 启动 SOP 启动/复用，PID 与窗口实况在手；
   handoff 工作目录已初始化（read.md 存在；缺失则先 agent_init 或走配置中心录入补齐）。
1. 上板+派发（主路径）：调 agent_dispatch 工具（参数 agent / task_id / brief / project 可选 /
   message 可选）——一次完成：写入 briefs/{task_id}-brief.md（内嵌契约原文）、置状态、
   渲染 message 并执行 team.toml dispatch_steps 投递。
   → brief 结构见 7.3；★「门铃契约」段由 dispatch 自动拼接 contract 原文，
     禁止手写转述、省略字段（实测事故：手写契约漏 token/header，agent 端 403/422 连环试错）。
2. 首次握手（本生命周期第一次对接该 agent）：终端只发
   「Read .nuphus/handoff/{agent}/read.md and report your understanding via the doorbell.」
   → agent 读协议文件 → 回报 progress（含关键理解）→ Leader 核对理解一致（不符返工重读）
   → 同时完成「投递通道 + 门铃回传」双向通路验证 → 共识建立。
3. 正式任务派发：共识已立，直接 agent_dispatch 发正式 brief → 等门铃注入（progress/done/blocked），
   状态栏自动反映，不轮询不打断。
4. 收 done/blocked → Read report 全文 + 交叉验证产物（§4）→ 达标归档（§7.6）/ 不达标返工（≤3 轮）。
5. 工具异常兜底：agent_dispatch 返回错误或被外层超时切断时，一律按 §5 第 6 条接管 SOP
   以文件与实况为准恢复链路——上板通常已完成，只需补投递。
```

**降级手段（仅调试用）**：POST http://127.0.0.1:{port}/handoff/dispatch
（Header X-Handoff-Token，body {"agent","task_id","brief"}）返回契约字符串——日常派发不走这条路，工具不可用时才临时替代，流程同上拆手做。

**禁止**：手写 brief 文件绕过 dispatch；手动改 status.json；每轮重复读协议；用 OCR 盯 Read 动作；手写转述门铃契约；跳过首次握手直接发正式任务。

### 7.1 派发前强制 Checklist（缺一不可）

```text
□ 1. 已调 agent_dispatch 上板+派发（一次建 brief + 置 status + 内嵌契约，见 7.0）——禁止手写 brief 文件绕过
□ 2. brief 门铃契约段 = dispatch 自动拼接的 contract 原文（含 X-Handoff-Token 头 + 可直接复制执行的上报示例），禁止手写转述/凭记忆写令牌
□ 3. 已要求接收方写 report 到契约给出的 report_path（固定四段）
□ 4. 已确认目标 agent 的 read.md 为最新协议版（含「开工即报 progress」纪律）——旧版协议文件会让状态栏缺失「执行中」态
□ 5. 已确认门铃纪律随契约下发：开工 health 自检 → 过程 progress → 完工 done（含 report_path）
□ 6. 下发：长指令走文件（终端只发「Read {brief 路径} and execute it.」单行短指令）
□ 7. 执行中不轮询不打断（禁止 sleep 空等；状态自动流转，看门铃注入即可）
□ 8. 本生命周期首次对接：先走首次握手（让 agent 读 read.md 回报理解，验证通路，见 §2 启动 Step4 / §7.0 步骤2）；共识已立后直接派正式任务
```

### 7.2 回传（门铃）——实证契约

```
POST http://127.0.0.1:{port}/handoff        # 默认 18771，实际以 prompt 环境信息/contract 为准
Header: X-Handoff-Token: {token}            # prompt 环境信息，每次运行随机，重启轮换；缺失或错误 → 403
Body:   {"id":"{agent}::{task_id}","status":"done|progress|blocked","summary":"...","report_path":"..."}
GET /handoff/health → 免令牌自检
```

**id 格式（关键）**：完整事件 id 必须 `{agent}::{task_id}`（如 `opencode::0824-02`）。`task_id` 本身可用 `MMDD-序号` 风格——错误用法是把 `MMDD-序号` 单独当事件 id（缺 agent 前缀）。门铃按 `::` 前缀归组更新 status.json：不带前缀的事件不报错但**静默不归组**（状态栏永远不动，难以察觉）。

**错误码自诊断**（agent 与 Leader 排查共用）：

| 响应 | 含义 | 动作 |
|---|---|---|
| 200 | 已受理 | —（同 id 终态重复 POST 幂等忽略，仍回 200） |
| 403 | 缺/错 X-Handoff-Token 头（响应体无任何提示） | 核对 token 来源：prompt 环境信息 / contract 原文；重启后旧 token 一律失效 |
| 422 | JSON 缺必填字段（id/status/summary） | 补全四个字段：id/status/summary/report_path(可 null) |
| 400 | status 非法值 或 id/summary 为空 | status 只允许 done/progress/blocked |

语义细节：progress 同 id 折叠（只注入最新一条）；done/blocked 终态幂等且到达后清除同 id 陈旧 progress；summary 换行压平、500 字截断。

- 终端类：curl 直调；**PowerShell 必须发 UTF-8 字节体**（`-Body ([Text.Encoding]::UTF8.GetBytes($json))`），字符串 body 会把中文压成 `?`（实测）；纯 ASCII body 才可用单引号直传
- 公网 Web 类：回复末尾输出 ```handoff 代码块，Leader 提取后代表中转
- 门铃失败降级：重试 1 次 → 仍失败记入 report ⚠️ 段 + 终端输出 handoff 标记

### 7.3 brief 结构

`.nuphus/handoff/{agent}/briefs/{task_id}-brief.md`（dispatch 自动落盘，禁止手写绕过）：任务定义（+成功标准）/ 上下文（文件:行号）/ 质量基线 / 反模式 / 歧义处理（按最合理假设推进并记入遗留，禁止停摆）/ 报告契约（四段：✅完成项 / 📄改动文件 / 🔍验证证据 / ⚠️遗留）；文末由 dispatch 自动拼接门铃契约原文（agent/task_id/端点/令牌/CLI 上报示例/产物与 report 绝对路径）——禁止手写转述、省略字段。

### 7.4 验收（禁止轮询）

收到门铃 done/blocked → Read report 全文 + 交叉验证产物（§4）→ 达标归档 / 不达标返工（brief 升版本重发 ≤3 轮）。

门铃未响 → 不主动空等；自然验收点（内部任务完成/下游需要/用户询问）时：report 已存在 → 验收；不存在 → 终端 OCR 看一眼（仍在跑→搁置；安全确认弹窗→放行；卡死→介入）。

Nuphus 重启（有在途任务）→ 令牌已轮换（旧令牌 403，契约已要求降级到 report）→ 重启后查 `.nuphus/handoff/` report 状态；续派用新令牌。

### 7.5 状态栏协同（外部 Agent 状态可视化）

前端 `ExternalAgentsStatusBar`（桌面底部）每 3s 轮询 `list_agent_statuses`，读取 `.nuphus/handoff/{agent}/status.json`（门铃 POST 驱动 state 流转）。Leader 利用它做**自然验收点的一眼确认**，不替代门铃/报告：

| 状态栏 state | 含义 | Leader 动作 |
|---|---|---|
| `in_progress` | Agent 正在跑 | 搁置，等门铃/下个验收点 |
| `done` | 完工（门铃 done 已落 status） | 走验收（§7.4） |
| `blocked` | 阻塞（agent 报 blocked） | 介入：查 report ⚠️/终端 OCR，解决后续派 |
| `blocked`（确认类） | 卡在需要人工批准的界面（权限/许可/执行确认） | **不代批**——向用户转呈「需在目标应用中手动授权什么」，用户授予后让 agent 重试或重派 |
| `error` | 出错 | 介入：查终端报错 |
| `idle` / `ready` | 空闲/就绪 | 无在途任务 |

注意：状态栏**只读不写**，state 由外部 Agent 门铃 POST 驱动（`progress`/`done`/`blocked`）；Leader 不要试图直接改 status.json。状态栏与门铃同源（status.json），门铃已响则状态栏必同步，二者互证。

**重启重置（设计意图）**：应用重启会把 status.json 重置为 idle/空 task_id（运行时态不跨重启）。在途任务经重启后，验收依据 = brief/report 文件（`.nuphus/handoff/`），状态栏只反映重启后的新事件；续派需重新 dispatch。

**契约未送达的探测信号**：状态栏 `in_progress` 停留但 `last_event` 长时间为 null → agent 大概率没拿到可用契约或上报被门铃拒绝（403/422，见 7.2 错误码表）。不要干等：Read brief 检查门铃契约段是否为 contract 原文（含 token/header/示例）→ 缺失则补发正确契约并让 agent 重报；agent 在终端反复试错探测端点也是同一信号。


**投递链路中断的探测信号**：`in_progress` + `last_event: null` + 终端无任何反应（agent 从未收到指令）→ 上板已完成但投递步骤失败。典型成因：派发前 agent 已被关闭、窗口句柄过期、dispatch 被外层超时切断。处置按 §5 第 6 条接管 SOP：核对进程/窗口实况，激活后补输「Read {brief_path} and execute it.」即可恢复，无需重新上板。

### 7.6 归档

brief + report 保留 `.nuphus/handoff/`（外部产出无 task_trace，这是唯一追溯链）；经验入记忆；新平台补登 team.toml。