---
title: 工作流设计经验手册
id: workflow-design
type: skill
tags: [workflow, 设计, 编排, schema, 调试, 闭环]
---

# 工作流设计经验手册

> 完整工作流编排能力：步骤 schema、变量语法、条件表达式、params 固化、外部输入声明、设计模式、验证闭环、经验闭环。
> L2 提示词只给阶段门禁与交互纪律；本文档是可执行的方法论全集。

---

## 〇、编排闭环总览

```
接收任务
  ├─ [查] ui_maps_search 检索同类经验 → 有 screen/experience 直接复用，跳过重复探索
  ├─ [探] UIA/Accessibility 优先跑通核心路径；只在工作流依赖时补视觉或旁支证据
  ├─ [固] 生成 params.json + workflow.json + guide.md（核心路径参数都有界面证据）
  ├─ [验] workflow_validate 编译校验 → workflow_run 真实成功一次；失败时针对性修正重跑
  └─ [馈] 跑通后 ui_maps_save_experience 提炼经验；新异常回写 params.json exceptions
```

**任务输入形态（先识别再走闭环）**：
- 用户输入带 `[意图表单→工作流]` 前缀，或来自 `request_user_input(step_form)` 的 `{stage, steps}`：这些阶段/子步骤是**用户确认的意图骨架**。表单阶段 = 流程主线分组（写入 workflow.json 保留为用户心智的阶段注释）；**每个子步骤 = 一条探索任务**，逐条走 [探]→[固]→[设]→[验]；骨架为权威输入，不得重新向用户收集流程、整体重构或丢弃补录 steps。
- 自由对话描述：目标、应用和预期结果已足够明确时直接探索，不为了形式完整强制询问意图表单；只有关键业务意图确实缺失时才询问。用户主动选择表单时使用 `request_user_input(input_type="step_form", default_stage=当前阶段名)`。
- 表单行意图是纯文本，无工具参数；工具参数（selector/坐标/窗口等）由你探索后固化进 `params.json` / `with` 字段。

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

## 五、外部输入（inputs 声明）

`workflow.inputs` 声明「运行前由使用者填写」的参数：值在启动时收集、注入变量池，供步骤模板引用。与 params.json 的分工是**设计期固化 vs 运行期输入**。

### 声明字段

```json
{
  "inputs": [
    { "name": "topic",     "type": "string",  "required": true, "description": "要处理的主题" },
    { "name": "rounds",    "type": "number",  "default": 3 },
    { "name": "use_cache", "type": "boolean", "default": true },
    { "name": "out_dir",   "type": "path",    "default": "D:/out" },
    { "name": "extra",     "type": "json" }
  ]
}
```

| 字段 | 必填 | 说明 |
|------|------|------|
| `name` | ✅ | 输入名，即变量名 |
| `type` | 可选 | `"string"`(默认) / `"number"` / `"boolean"` / `"path"` / `"json"`。决定启动控件和运行契约；执行层严格校验 JSON 类型，**不做隐式转换** |
| `required` | 可选 | 默认 `false`；`required` 且无 `default` 时未填 → 启动按钮被禁用并给出原因，后端同样在执行前报错 |
| `default` | 可选 | 未填时生效的兜底值（启动表单预填该值，可直接改） |
| `description` | 可选 | 表单内的填写说明 |
| `sensitive` | 可选 | 默认 `false`，见「敏感输入」 |

### 取值优先级与引用

合并优先级：**显式填写 > 声明 default > 未提供且非必填（该键不注入）**。

| 写法 | 语义 | 适用 |
|------|------|------|
| `{{inputs.topic}}` | 命名空间引用（**推荐**：与 `{params.x}` 对称，意图明确） | with 参数、script code、chat 描述 |
| `{{topic}}` | 顶层变量引用（声明项同时散注入顶层） | 兼容写法 |
| `{{inputs.extra \| get "k"}}` | 命名空间 + 管道下钻 | 结构化输入取字段 |
| `{ "var": "inputs.flag" }` | 条件表达式里的变量引用（VarRef 不是模板） | if / until / assert / for_each.items |

**双写与未声明键**：声明项同时写入 `variables["inputs"].x` 与顶层 `x`；运行接口传入的**未声明**键只散注入顶层（不自动提升为声明、不报错）。

> 坑：整串 `{{inputs.x}}` 保留原始类型（数字/布尔/对象不字符串化），内嵌文本会字符串化；条件里写 `{{inputs.x}}` 无效，必须用 VarRef 形式。

### 与 params.json 的分工

| | params.json（`{params.x}`） | inputs 声明（`{{inputs.x}}`） |
|---|---|---|
| 谁在何时写 | 你：探索后设计期固化，随工作流文件走 | 使用者：运行前在启动表单填 |
| 是否随运行变化 | 否 | 是（同一工作流可多套输入） |
| 典型内容 | 窗口尺寸、区域坐标、登录特征、跳转图 | 主题、目录、条数、开关、凭据 |
| 判定原则 | 有界面证据的固定事实一律固化 | **只有必须由人现填的才声明**（声明越多，启动越繁琐） |

### 校验规则

- 引用 `{{inputs.x}}` 但 `x` 未声明 → **error**（阻断保存与执行）。
- 声明了但全流程未被任何步骤引用 → **warning**（提示多余声明，不阻断）。
- 名称必须匹配 `[A-Za-z_][A-Za-z0-9_]*`，不得重复或使用 `inputs` / `params` / `_index` 等保留名。
- `default` 和运行时显式值必须符合声明类型；错误不做字符串/数字/布尔值之间的自动转换。
- 无 `inputs` 字段的旧工作流行为完全不变。
- `workflow.doc`（计划 markdown）不参与引用扫描：正文提及 `{{inputs.x}}` 不算引用。

### 敏感输入（sensitive）

密码控件、不回显；值不进日志、不进事件文本。**当前边界**：值仍会随变量快照写入 `variables_snapshot`（本地运行记录），加密落盘尚未实现——敏感输入只应放「本机可接受落盘」的凭据。

### 启动入口与已知边界

| 入口 | 行为 |
|------|------|
| 工作流列表 / 设置中心的「运行」 | 运行确认弹窗按声明渲染输入表单（未声明 inputs 时弹窗与原来完全一致） |
| 画布「运行」与 `R` 快捷键 | 共用同一输入表单，声明了 inputs 时先收集再启动，不绕过必填 |

- **断点续连**：暂停快照在输入注入**之后**恢复，本次新填的输入对快照中已有的同名变量**不生效**（续跑保持上下文一致），UI 暂不提示；需要换值 → 让工作流从头执行。
- **子工作流**：`call.with.inputs` 先按父变量池解析模板，再按目标工作流声明执行必填、默认值和类型校验；声明项写入子流程的 `inputs.x` 与顶层 `x`。旧工作流的未声明映射仍只写顶层。
- **定时任务**：`schedule_cron add` 可传 `inputs` 对象，只接受目标已声明字段；显式值形成本地快照，声明默认值不固化并在每次触发时读取当前版本。敏感值在 Windows 通过 DPAPI 加密，macOS/Linux 沿用明文降级，但列表和日志均不展示值。
- cron 固定为五字段格式，并使用 IANA 时区（如 `Asia/Shanghai`、`America/New_York`）；非法表达式或时区会在注册时直接失败，DST 由时区数据库处理。

---

## 六、设计模式库

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

## 七、验证闭环方法论

```
workflow_validate（编译校验：步骤合法性/工具名/必填/变量引用/call 环）
  → 干净环境 workflow_run（第 1 次：探路，记录偏差）
  → 分析异常 → 修正 params / workflow（设计缺陷才改文件；运行时阻塞就地解决）
  → 失败才重跑（同 id 断点续连，禁新建复制）→ 一次真实成功即可交付
  → 不主动制造无关异常；只验证实际遇到或会阻塞运行的异常路径
```

验收：跑通 ∧ workflow_run 真实成功一次 ∧ 降级不丢数据不重复提交 ∧ 无临时 token/candidate ID/自由坐标或敏感数据残留。

**运行时故障恢复**：失败 → 识别阻塞（验证码/弹窗/登录态/网络）→ 就地解决（browser_*/desktop_* 同会话，状态保留）→ 同 id 续跑。同一步骤同阻塞连续 3 次失败 → 停止，用 completed_steps 汇报。

---

## 八、经验检索 / 提炼

### 两级检索

1. 骨架：`ui_maps_search(query="微信 im")` → 有哪些 screen / experience
2. 详情：`ui_maps_search(query="微信", screen_name="chat-list")` → 完整 regions + 关联经验

### 什么值得存（存「法」不存「案」）

- ✅ 非显而易见操作序列 / 跨应用可复用模式 / 踩过的坑 / 特殊定位技巧
- ❌ 常规操作（browser_navigate）/ 一次性案例 / 提示词已覆盖内容

### summary 写法

`做什么 → 怎么做 → 为什么 → 跨应用适用性`（非传统思路要标注省了几步）。

---

## 九、工具使用要点

| 场景 | 首选 | 备选 |
|------|------|------|
| 定位网页元素 | `browser_snapshot` → @eN ref | screenshot + OCR |
| 定位桌面控件 | `desktop_semantic_observe` 读取 UIA/Accessibility 候选 | Vision → perceive 精确坐标 |
| 固化桌面动作 | 保存候选返回的 `workflow_step`，运行时调用 `desktop_semantic_action` | 无稳定语义定位器时再固化坐标方案 |
| 桌面布局解析 | UIA/Accessibility 语义树 | Vision 全窗口语义分析 |
| 定位桌面文字 | UIA/Accessibility 控件名称 | Vision 划定功能区 → `desktop_find_text`（需字库） |
| 等待加载 | `browser_wait_for(selector)` | system_sleep（不得已） |
| 验证状态 | snapshot + chat 语义判断 | extract 文本匹配 |
| 查经验 | ui_maps_search 两级检索 | Read ui-maps JSON |

**桌面语义动作**：探索时先调用 `desktop_semantic_observe`，从有限候选中执行并验证；写入工作流时只保存返回的稳定 `workflow_step`，禁止保存临时 `candidate_id` 或 `observation_token`。只有目标应用不暴露有效 UIA/Accessibility 控件时，才使用截图、OCR、YOLO 和鼠标坐标路径。

**坐标体系**：确需使用 `desktop_mouse` 时一律用**屏幕绝对坐标**；perceive 结果为客户区坐标时手动加 `screen_x/screen_y` 偏移。

**输入**：`desktop_input` 输入+发送一次调用；普通文本直接输入，>500 字用 clipboard 并事后 clean；敏感内容禁用 clipboard。

---

## 十、陷阱清单

| 陷阱 | 正确做法 |
|------|---------|
| 跳过语义观察直接猜坐标（W1） | 先 UIA/Accessibility 观察；不可用时再逐屏 vision+perceive，保存 ui-maps |
| 窗口尺寸未固化（W2） | params.json window 字段固化 |
| 探索阶段写步骤（W3） | 核心路径手动跑通后才设计 |
| if contains 文案做登录检测 | chat 语义判断 + screenshot |
| 忘记 SPA 状态残留 | 新流程前重置（about:blank / resize 固化尺寸） |
| tooltip OCR 截到其他窗口 | 先 desktop_window_info 拿客户区边界，截图限域 |
| 重试重复提交表单 | 重试前检测状态，已完成步骤跳过 |
| 纯色/低纹理模板匹配误报 | find_image 模板需含纹理；用 region 限定加速 |
| 动态 UI 区匹配失败 | 识别为动态区域，改用文本/语义定位 |