---
title: 工作流设计经验手册
id: workflow-design
type: skill
tags: [workflow, 设计, 编排, schema, 调试, 闭环]
---

# 工作流设计经验手册

> 完整工作流编排能力：步骤 schema、变量语法、条件表达式、params 固化、设计模式、验证闭环、经验闭环。
> L2 提示词只给阶段门禁与交互纪律；本文档是可执行的方法论全集。

---

## 〇、编排闭环总览

```
接收任务
  ├─ [查] ui_maps_search 检索同类经验 → 有 screen/experience 直接复用，跳过重复探索
  ├─ [探] 逐屏探索 → ui_maps_save_screen 固化布局（每屏经用户确认）
  ├─ [固] 生成 params.json + workflow.json + guide.md（参数即契约，全部有界面证据）
  ├─ [验] workflow_validate 编译校验 → workflow_run 执行 → 连续 3 次一致 + 至少一个异常路径
  └─ [馈] 跑通后 ui_maps_save_experience 提炼经验；新异常回写 params.json exceptions
```

---

## 一、步骤 Schema（V2，唯一真相源 `src/workflow/step_schema.json`）

### 公共字段

| 字段 | 必填 | 说明 |
|------|------|------|
| `id` | ✅ | 步骤唯一标识（断点续连按 id 跳过已完成步骤） |
| `name` | ✅ | 人类可读名称 |
| `description` | 可选 | 详细说明 |
| `on_error` | 可选 | `"abort"`(默认) / `"skip"` / `{retry:{max,backoff_ms?,backoff_multiplier?}}` / `{allow_codes:{codes:[...]}}` |
| `capture` | 可选 | **字符串**：步骤输出存入变量名（无对象格式） |
| `timeout_secs` | 可选 | 步骤超时秒数 |
| `do` | ✅ | 动作定义，仅下列一种 key |

### 动作类型（do 的 13 种形态）

| kind | 写法 | 要点 |
|------|------|------|
| tool | `{"tool":"desktop_mouse","with":{...}}` | with 支持 `{{var}}` 模板 |
| seq | `{"seq":[...]}` | 顺序容器，可嵌套 |
| loop | `{"loop":{"for_each"\|"repeat"\|"until":..., "max":100, "do":[...]}}` | until=条件**满足即停**；max 防死循环 |
| if | `{"if":{"condition":{...},"then":[...],"else":[...]}}` | else 可省略 |
| call | `{"call":"wf-id","with":{"inputs":{...},"outputs":{...}}}` | inputs 传子变量；outputs 子变量名→父变量名回写 |
| wait | `{"wait":"提示语","auto":[...]}` | auto=等待期自动执行步骤（可选） |
| chat | `{"chat":"LLM任务描述","with":{...}}` | LLM 决策节点（原 chat_agent） |
| script | `{"script":{"runtime":"python"\|"node"\|"ahk"\|"pwsh","code":"...","cwd":"?"}}` | code 支持 `{{var}}` 替换 |
| assert | `{"assert":{"condition":{...},"message":"?"}}` | on_error=skip 时变非阻断校验 |
| mcp | `{"mcp":{"server":"key","tool":"name","with":{...}}}` | server 对应 servers.yaml |
| sleep | `{"sleep":N}` | 秒，0.1–3600 |
| break | `{"break":true}` | 跳出当前循环 |
| continue | `{"continue":true}` | 跳过当前迭代 |

### chat 步骤 with（LLM 决策配置）

`agent_id`（ChatAgentConfig ID）/ `screenshot`（执行前截图注入）/ `tools`（白名单）/ `knowledge`（知识库路径）/ `model` / `temperature` / `max_tokens` / `system_prompt` / `persona` / `goal` / `constraints` / `requirements` / `max_iterations`（ReAct 最大轮数，**与 max_steps 语义重叠，统一用 max_iterations**）

### 容器 on_error

seq / loop / if 容器同样支持 on_error；子步骤失败且容器设置了 skip 时继续执行容器内后续步骤。

---

## 二、变量与模板语法

### 三套引用，边界必须分清

| 写法 | 语义 | 适用 |
|------|------|------|
| `{{var}}` | 模板替换。**整串**时保留原始类型（数字/布尔/对象不字符串化）；**内嵌文本**（如 `"x={{var}}px"`）时字符串化 | with 参数值、script code |
| `{{var \| get "f"}}` / `{{var \| json "k"}}` / `{{var \| len}}` / `{{ENV:HOME}}` / `{{var \| default "v"}}` | 管道表达式 | 同上 |
| `{params.window.url}` | **单花括号**，仅整串引用 params.json 字段，返回原始类型；点号路径下钻 | 引用固化参数 |
| `{ "var": "name" }` | 对象形式变量引用（VarRef），支持点号路径 `{ "var": "coords.x" }` | 条件表达式、loop.for_each.items |

> 坑：坐标字段若内嵌 `{{x}}` 会字符串化，务必整串引用或用 `| get` 下钻保持数字类型。

### 变量池来源

`workflow_run(inputs)` 注入 → params.json（`{params.x}` 兑现）→ 各步骤 `capture` 写入。子工作流有独立变量池，靠 `call.with.inputs/outputs` 跨池传递。

---

## 三、条件表达式（Condition，12 种）

二元（[VarRef, 值] 两元数组）：`equals` / `not_equals` / `contains` / `starts_with` / `regex` / `gt` / `lt` / `gte` / `lte`
一元：`not_empty` / `empty`
恒真：`always`

```json
{ "if": { "condition": { "equals": [ { "var": "login_status" }, "LOGGED_IN" ] }, "then": [...] } }
```

---

## 四、params.json 固化规范

| 字段 | 内容 |
|------|------|
| `workflow_id` | 工作流唯一标识 |
| `window` | 尺寸 / URL / 标题模式（**窗口尺寸必须固化**，W2） |
| `login_detection` | 登录态判定特征 |
| `regions` | 区域定义 + 定位特征（每参数有界面证据） |
| `navigation_graph` | 屏间跳转关系 |
| `exceptions` | 异常路径 + 降级策略（探索中异常即时记录，W4） |

模板骨架：

```json
{
  "workflow_id": "demo-flow",
  "window": { "title_pattern": "App 标题", "width": 1280, "height": 800 },
  "login_detection": { "indicator": ["登录", "Sign in"] },
  "regions": [{ "name": "chat-list", "rect": { "x": 0, "y": 100, "w": 300, "h": 600 }, "anchor": { "type": "text", "value": "会话" } }],
  "navigation_graph": { "chat-list": { "to": ["chat-window"], "trigger": "click-contact" } },
  "exceptions": [{ "condition": "登录弹窗", "fallback": "wait 用户介入" }]
}
```

---

## 五、设计模式库

### 模式 A：登录态检测守卫

```json
{ "id": "guard", "name": "登录保障", "do": { "seq": [
  { "id": "check", "name": "判断登录态",
    "do": { "chat": "根据 login_detection 特征判断是否已登录", "with": {
      "agent_id": "login-checker", "screenshot": true,
      "requirements": ["只输出 LOGGED_IN 或 LOGIN_REQUIRED"] } },
    "capture": "login_status" },
  { "id": "branch", "name": "按登录态分支",
    "do": { "if": {
      "condition": { "equals": [ { "var": "login_status" }, "LOGIN_REQUIRED" ] },
      "then": [
        { "id": "manual", "name": "手动登录", "do": { "wait": "请完成登录后点击继续" } }
      ],
      "else": [] } } }
] } }
```

要点：**语义判断**（chat+screenshot）而非文案匹配；登录动作交用户，不碰凭据。

### 模式 B：for_each 遍历

```json
{ "id": "tour", "name": "遍历", "do": { "loop": {
  "for_each": { "items": { "var": "panels" }, "as": "p" },
  "max": 100,
  "do": [ { "id": "hit", "name": "点击", "do": { "tool": "desktop_mouse",
    "with": { "action": "click", "x": "{{p | get \"ix\"}}", "y": "{{p | get \"iy\"}}" } } } ]
} } }
```

### 模式 C：表单提交 + 结果验证

```
seq: 提交
├─ 填写 + 提交（desktop_input 输入+发送一次调用，不拆分）
├─ browser_wait_for(结果页元素)
├─ assert: 成功标志存在
└─ if: 失败 → screenshot → chat 分析 → 重试或终止
```

### 模式 D：多窗口操作

每次操作前 `desktop_windows_list` 重取 hwnd（hwnd 会变）→ activate → 操作。

### 模式 E：大工作流拆分

单层 >15 步或嵌套 >3 层 → 拆子工作流。主文件含 `call` 步骤，子工作流 JSON 同目录。用 `with.outputs` 回传结果。

### 模式 F：SPA 搜索框提交（优先级从高到低）

1. `browser_snapshot` 找 @eN ref → click
2. 无 ref → `browser_screenshot` → OCR 找按钮坐标 → JS 模拟点击
3. 都不行 → `browser_evaluate` 触发 form.submit()（最后手段）

### 模式 G：验证码/滑块

`desktop_mouse_drag` 起点→终点；无法自动处理 → `wait` 用户介入并标记 exceptions。

---

## 六、验证闭环方法论

```
workflow_validate（编译校验：步骤合法性/工具名/必填/变量引用/call 环）
  → 干净环境 workflow_run（第 1 次：探路，记录偏差）
  → 分析异常 → 修正 params / workflow（设计缺陷才改文件；运行时阻塞就地解决）
  → 重跑（同 id 断点续连，禁新建复制）→ 连续 3 次结果一致
  → 至少触发一个异常路径验证降级
```

验收：跑通 ∧ 3 次一致 ∧ 异常路径生效 ∧ 降级不丢数据不重复提交 ∧ 无敏感数据残留。

**运行时故障恢复**：失败 → 识别阻塞（验证码/弹窗/登录态/网络）→ 就地解决（browser_*/desktop_* 同会话，状态保留）→ 同 id 续跑。同一步骤同阻塞连续 3 次失败 → 停止，用 completed_steps 汇报。

---

## 七、经验检索 / 提炼

### 两级检索

1. 骨架：`ui_maps_search(query="微信 im")` → 有哪些 screen / experience
2. 详情：`ui_maps_search(query="微信", screen_name="chat-list")` → 完整 regions + 关联经验

### 什么值得存（存「法」不存「案」）

- ✅ 非显而易见操作序列 / 跨应用可复用模式 / 踩过的坑 / 特殊定位技巧
- ❌ 常规操作（browser_navigate）/ 一次性案例 / 提示词已覆盖内容

### summary 写法

`做什么 → 怎么做 → 为什么 → 跨应用适用性`（非传统思路要标注省了几步）。

---

## 八、工具使用要点

| 场景 | 首选 | 备选 |
|------|------|------|
| 定位网页元素 | `browser_snapshot` → @eN ref | screenshot + OCR |
| 桌面布局解析 | Vision 全窗口语义分析 | perceive 精确坐标 |
| 定位桌面文字 | Vision 划定功能区 | `desktop_find_text`（需字库） |
| 等待加载 | `browser_wait_for(selector)` | system_sleep（不得已） |
| 验证状态 | snapshot + chat 语义判断 | extract 文本匹配 |
| 查经验 | ui_maps_search 两级检索 | Read ui-maps JSON |

**坐标体系**：`desktop_mouse` 一律用**屏幕绝对坐标**；perceive 结果为客户区坐标时手动加 `screen_x/screen_y` 偏移。

**输入**：`desktop_input` 输入+发送一次调用；普通文本直接输入，>500 字用 clipboard 并事后 clean；敏感内容禁用 clipboard。

---

## 九、陷阱清单

| 陷阱 | 正确做法 |
|------|---------|
| 跳过布局解析直接找元素（W1） | 逐屏 vision+perceive 解析，保存 ui-maps |
| 窗口尺寸未固化（W2） | params.json window 字段固化 |
| 探索阶段写步骤（W3） | 核心路径手动跑通后才设计 |
| if contains 文案做登录检测 | chat 语义判断 + screenshot |
| 忘记 SPA 状态残留 | 新流程前重置（about:blank / resize 固化尺寸） |
| tooltip OCR 截到其他窗口 | 先 desktop_window_info 拿客户区边界，截图限域 |
| 重试重复提交表单 | 重试前检测状态，已完成步骤跳过 |
| 纯色/低纹理模板匹配误报 | find_image 模板需含纹理；用 region 限定加速 |
| 动态 UI 区匹配失败 | 识别为动态区域，改用文本/语义定位 |
