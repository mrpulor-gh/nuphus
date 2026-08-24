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

渐进登记：每用一个外部 Agent，在 `plugin/team.toml` 追加一段。只记稳定事实（type/open/process），禁止记 PID/窗口句柄/坐标（坐标走 ui-maps）。

```toml
[opencode]
type = "terminal"       # terminal | web-ui | desktop — 决定交互协议（§2）
open = "终端执行 opencode"
process = "opencode.exe"
```

调用决策：Read team.toml → process_list 看谁活着（活着优先）→ 按平台能力匹配。不维护历史评分。

---

## 2. 交互协议

### 窗口定位（terminal 类，不可跳过）

```
1. process_list 找 Agent 进程（按 team.toml process 字段）→ 记 PID
2. 查父进程（Get-CimInstance Win32_Process）→ 父进程 MainWindowHandle → windows_list 定位
3. 截图 + OCR 确认是 Agent（提示符/任务输出）——运行时标题常被覆写（如 OpenCode 显示「OC | 任务名」），不能靠标题
```

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

### 启动

Leader 自启（team.toml open 字段，system_shell Start-Process 开终端 → 输入启动命令）；自启失败请用户手动打开。

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
- **Leader 只做四件事**：①派发（走 dispatch 端点）②首次派发验收理解（读规则环）③每轮收门铃后验收产物（§7.4）④归档（§7.6）。

### 7.0 Leader 派发操作要求（无上下文时照此执行）

**状态机制**：agent 门铃 POST → 后端自动写 status.json → 前端状态栏自动轮询。Leader 零触发、不手动改 status.json。

**Leader 完整操作序列**：

```
1. 调 POST http://127.0.0.1:18771/handoff/dispatch（X-Handoff-Token: {prompt 环境信息}）
   body: {"agent":"{平台}","task_id":"MMDD-序号","brief":"","project":"可选"}
   （brief 先传空串——契约由本步返回，避免鸡生蛋；brief 允许后补是既有能力）
   → 返回 contract：门铃 URL / 令牌 / 上报 POST 示例 / 产物路径 / report_path
2. Write brief 到 contract 给出的 briefs/{task_id}-brief.md（结构见 7.3）。
   ★「门铃契约」段 = contract 原文原样嵌入（含 X-Handoff-Token 头 + 完整可复制的
   POST 请求示例，id 为 {agent}::{task_id} 形态）——禁止手写转述、省略字段、
   只写文字描述不带示例（实测事故：手写门铃契约漏掉 token/header/schema，
   agent 端 403/422 连环试错，闭环第一步即断）
3. 首次派发：终端发「读 {contract 中 brief 路径} 并回报关键理解」→ 等门铃 progress（含理解要点）
   → 验收理解一致（不符返工重读）→ 建立共识
   同会话后续派发：直接发正式任务指令（契约已生效，无需重复读规则）
4. 每轮等门铃注入（progress/done/blocked）→ 状态栏自动反映，不轮询
5. 收 done/blocked → Read report 全文 + 交叉验证产物（§4）→ 达标归档（§7.6）/ 不达标返工（≤3 轮）
```

**禁止**：手写 brief 文件绕过 dispatch；手动改 status.json；每轮重复读规则；用 OCR 盯 Read 动作；手写转述门铃契约（见步骤 2 ★）。

### 7.1 派发前强制 Checklist（缺一不可）

```text
□ 1. 已走 POST /handoff/dispatch 派发（一次建 brief + 置 status + 取契约，见 7.0）——禁止手写 brief 文件绕过
□ 2. brief 门铃契约段 = dispatch 返回 contract 原文（含 X-Handoff-Token 头 + 可直接复制执行的 POST 示例），禁止手写转述/凭记忆写令牌
□ 3. 已要求接收方写 report 到契约给出的 report_path（固定四段）
□ 4. 已要求门铃契约：开工前 health 自检 → 读规则环 progress（含理解要点）→ 正式任务环 progress → 完工 done（含 report_path）
□ 5. 下发：长指令走文件（契约返回后终端只发「读 {brief 路径} 并执行」）
□ 6. 执行中不轮询不打断（禁止 sleep 空等；状态自动流转，看门铃注入即可）
□ 7. 首次派发先读规则环（验收理解建立共识）；同会话后续直接派正式任务，见 §2 派发闭环
```

### 7.2 回传（门铃）——实证契约

```
POST http://127.0.0.1:{port}/handoff        # 默认 18771，实际以 prompt 环境信息/contract 为准
Header: X-Handoff-Token: {token}            # prompt 环境信息，每次运行随机，重启轮换；缺失或错误 → 403
Body:   {"id":"{agent}::{task_id}","status":"done|progress|blocked","summary":"...","report_path":"..."}
GET /handoff/health → 免令牌自检
```

**id 格式（关键，旧文档「MMDD-序号」为误）**：必须 `{agent}::{task_id}`（如 `opencode::0824-02`）。门铃按 `::` 前缀归组更新 status.json——不带前缀的事件不报错但**静默不归组**（状态栏永远不动）。

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

`.nuphus/handoff/{id}-brief.md`：任务定义（+成功标准）/ 上下文（文件:行号）/ 质量基线 / 反模式 / 歧义处理（按最合理假设推进并记入遗留，禁止停摆）/ 报告契约（四段：✅完成项 / 📄改动文件 / 🔍验证证据 / ⚠️遗留）/ 门铃契约（= dispatch 返回 contract 原文原样嵌入，见 7.0 步骤 2 ★——禁止手写转述）

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
| `error` | 出错 | 介入：查终端报错 |
| `idle` / `ready` | 空闲/就绪 | 无在途任务 |

注意：状态栏**只读不写**，state 由外部 Agent 门铃 POST 驱动（`progress`/`done`/`blocked`）；Leader 不要试图直接改 status.json。状态栏与门铃同源（status.json），门铃已响则状态栏必同步，二者互证。

**重启重置（设计意图）**：应用重启会把 status.json 重置为 idle/空 task_id（运行时态不跨重启）。在途任务经重启后，验收依据 = brief/report 文件（`.nuphus/handoff/`），状态栏只反映重启后的新事件；续派需重新 dispatch。

**契约未送达的探测信号**：状态栏 `in_progress` 停留但 `last_event` 长时间为 null → agent 大概率没拿到可用契约或上报被门铃拒绝（403/422，见 7.2 错误码表）。不要干等：Read brief 检查门铃契约段是否为 contract 原文（含 token/header/示例）→ 缺失则补发正确契约并让 agent 重报；agent 在终端反复试错探测端点也是同一信号。

### 7.6 归档

brief + report 保留 `.nuphus/handoff/`（外部产出无 task_trace，这是唯一追溯链）；经验入记忆；新平台补登 team.toml。