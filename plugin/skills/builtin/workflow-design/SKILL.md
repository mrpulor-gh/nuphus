---
title: 工作流设计经验手册
id: workflow-design
type: skill
tags: [workflow, 设计, 经验, 调试, 闭环]
---

# 工作流设计经验手册

> **阶段门禁、产出规范、步骤 schema、检查清单、验收标准均已内置在 WorkAgent L2 提示词中，本文档不复制。**
> 本文档只提供 L2 没有的差量内容：实战经验 + 检索/提炼方法 + 陷阱详情 + 场景模式。

---

## 〇、闭环总览

> 四阶段流程（查→探→固→验）和门禁条件见 L2，此处强调经验积累闭环：

```
接收任务
  │
  ├─→ [查] ui_maps_search 检索同类应用/操作经验
  │       已有经验？→ 直接复用布局参数和操作链，跳过重复探索
  │
  ├─→ [探] 逐屏探索 → ui_maps_save_screen 固化布局
  │
  ├─→ [固] 生成 params.json + workflow.json + guide.md
  │
  ├─→ [验] dry_run 校验 → workflow_run 执行验证
  │       跑通？→ 提炼经验 → ui_maps_save_experience → 闭环
  │       失败？→ 分析原因 → 修正参数 → 重新执行
  │
  └─→ [馈] 异常回写 exceptions，新发现补充到经验
```

**核心原则**：每次设计都是对经验库的充实。不重复探索已被验证的路径。

---

## 一、阶段差量要点

> 各阶段的目标/退出条件见 L2。此处只补充 L2 没写的实战细节。

### 阶段 0：复用检索

检索方法与决策见「二、经验检索指南」。骨架有 screen → 跳过阶段一布局解析；有 experience → 参考 tool_chain 调参复用；无结果 → 完整探索。

### 阶段 1：探索跑通

**探索前先对齐**（硬性门禁，逐项确认后才能进入探索）：

```
对齐清单：
□ 目标应用/页面的启动/打开方式？（需用户手动打开还是工作流自动？）
□ 目标界面的触发方式？（快捷键/点击/URL/API？）
□ 目标界面的关闭/退出方式？（Esc/遮罩/X按钮/自动消失？）
□ 操作流程中哪些步骤需要用户介入？（登录/验证码/确认对话框？）
□ 是否有不可逆操作需要用户确认？（删除/发送/提交？）
□ 是否有已知的异常状态或边界情况？（弹窗/加载延迟/权限不足？）
```

确认后记录到 `params.json` 的 `exceptions` 或其他对应字段。禁止把功能约束误判为坐标/定位问题。

**浏览器页面（snapshot + screenshot 互补）**：
1. `browser_snapshot` 获取页面骨架和 @eN ref
2. snapshot 未返回目标元素 ref → 立即 `browser_screenshot` → `desktop_vision` 分析
3. 两个工具互相补盲，缺什么补什么。**不换 JS 策略重试。**

**反爬预检（浏览器第一屏必须执行）**：
`browser_screenshot` → `desktop_vision` 检测页面是否正常渲染。
检测词：登录/验证码/Captcha/空空如也/请先登录。
命中 → 立即标记 exceptions，预设 `wait` 步骤让用户介入。不尝试绕过。
1688/淘宝/京东/拼多多等电商平台默认视为反爬平台。

### 阶段 2：params.json 字段规范

| 字段 | 内容 |
|------|------|
| `workflow_id` | 工作流唯一标识 |
| `window` | 尺寸、URL、标题模式 |
| `login_detection` | 登录态判定逻辑 |
| `regions` | 区域定义 + 定位特征 |
| `navigation_graph` | 屏间跳转关系 |
| `exceptions` | 异常路径 + 降级策略 |

---

## 二、经验检索指南

**两级搜索**：先骨架后详情，不会爆 output。

**第一级 — 骨架**（发现有哪些可用资源）：
```
ui_maps_search(query="微信 im")
→ [{ app_name: "微信", app_category: "im",
     matched_screens: [{ screen_name: "chat-list", region_count: 3 }, ...],
     matched_experiences: [{ id: "send-msg", name: "发消息", keywords: [...] }, ...]
   }]
```

**第二级 — 详情**（精确获取完整数据）：
```
ui_maps_search(query="微信", screen_name="chat-list")
→ [{ app_name: "微信",
     matched_screens: [{ screen_name: "chat-list", regions: [{ name, rect, elements: [...], anchor: {...} }, ...] }],
     matched_experiences: [{ id, name, tool_chain, summary: "完整原文", ... }]  // 仅 screen_ref=="chat-list" 的经验
   }]
```

### 什么时候查

| 时机 | 示例 |
|------|------|
| 开始新设计前 | `ui_maps_search(query="微信 im")` — 查 app 有没有已有 screen |
| 遇到类似操作 | `ui_maps_search(query="发送消息 enter")` — 查操作经验 |
| 定位困难 | `ui_maps_search(query="图标 tooltip hover")` — 查定位经验 |
| 跨应用参考 | `ui_maps_search(app_category="im", query="发送")` — 限定类别 |

### 怎么用查到的结果

1. **有 matched_screens** → 直接用 regions 数据（name/rect/description/elements），跳过阶段一布局解析
2. **有 matched_experiences** → 参考 tool_chain 和 summary，调整参数复用
3. **都没匹配** → 进入完整探索，结束后 save_screen + save_experience 补充经验库

---

## 三、经验提炼指南

### 什么值得存为经验

- ✅ 非显而易见的操作序列（如 hover→等2s→小范围OCR→点击）
- ✅ 跨应用可复用的模式（如登录检测模式）
- ✅ 曾被踩过的坑及解决方案
- ✅ 特殊定位技巧
- ❌ 常规操作（如 browser_navigate 打开网页）
- ❌ 一次性的特定操作
- ❌ 已经在提示词中充分覆盖的内容

### 经验摘要（summary）写法

好的 summary 遵循：**做什么 → 怎么做 → 为什么这样做 → 跨应用适用性**

```
微信桌面版发送文字消息：

搜索框 desktop_input 输入联系人名称 → enter 直接进入对话窗口
（非传统思路：省略了 OCR 定位联系人列表 → click 选中 → click chat-area 获焦，省 2-3 步）

chat-area 已自动获焦，直接 desktop_input 输入文字 → enter 发送。

关键判断：微信搜索框输入正确联系人后 enter，会直接跳转对话且 chat-area 自动获焦。
可跨应用：即时通讯桌面应用中，凡搜索框支持 enter 直达对话的，此模式适用。
```

差的 summary：`"微信发送消息：点击联系人，输入文字，回车发送。"`

---

## 四、产出规范

### 目录结构（严格约定）

```
plugin/workflows/{workflow.id}/
├── workflow.json    # 主工作流定义，id 字段 = 目录名
├── params.json      # 固化参数（可选，UIMap 兼容格式）
└── guide.md         # 使用文档（可选）
```

- 目录名 = `workflow.json` 中的 `id` 字段
- `params.json` 若存在，需包含 `app_name`、`process_name`、`window.title_pattern` 等窗口定位字段（UIMap 兼容格式）
- 不要依赖 `wf.name` 作为目录名（name 可能含空格/中文）
- runtime 自动在根目录生成的 `{name}.json`/`{name}.md` 为内部缓存，不手动创建

## 五、常见陷阱

> 以下 5 条的警示已内置 L2（关键警示 + 检查清单），此处不再展开：
> 跳过布局解析直接找元素 / 窗口尺寸未固化 / 探索阶段写步骤 / 用 if contains 文案做登录检测 / 忘记 SPA 状态残留。

### 把 tooltip OCR 截图截到其他窗口
**症状**：hover 图标后 OCR 识别出桌面其他窗口的文字。
**正确做法**：先 `desktop_window_info` 获取客户区边界，截图范围约束在客户区内。

### 降级路径导致数据丢失
**症状**：重试时重复提交表单。
**正确做法**：重试前检测当前状态，已完成步骤跳过。

---

## 六、复杂场景模式

### 模式 A：登录态检测与自动登录

```json
{ "id": "login_guard", "name": "登录保障", "do": { "seq": [
  { "id": "check", "name": "判断登录态",
    "do": { "chat": "根据 login_detection 特征判断是否已登录", "with": {
      "agent_id": "login-checker",
      "screenshot": true,
      "requirements": ["只输出 LOGGED_IN 或 LOGIN_REQUIRED"]
    } },
    "capture": "login_status" },
  { "id": "branch", "name": "按登录态分支",
    "do": { "if": {
      "condition": { "equals": [ { "var": "login_status" }, "LOGIN_REQUIRED" ] },
      "then": [
        { "id": "goto", "name": "打开登录页",
          "do": { "tool": "browser_navigate", "with": { "url": "{params.login.url}" } } },
        { "id": "manual", "name": "手动登录",
          "do": { "wait": "请完成登录后点击继续" } }
      ],
      "else": []
    } } }
] } }
```
要点：语义判断（chat 步骤 + screenshot）而非文案匹配；登录动作优先 `wait` 让用户完成，不碰凭据。

### 模式 B：for_each 循环遍历

```json
{
  "id": "tour",
  "name": "面板巡览",
  "do": {
    "loop": {
      "for_each": { "items": { "var": "panels" }, "as": "p" },
      "max": 100,
      "do": [
        { "id": "click", "name": "点击",
          "do": { "tool": "desktop_mouse",
            "with": { "action": "click", "x": "{{p | get \"ix\"}}", "y": "{{p | get \"iy\"}}" } },
          "on_error": "abort" }
      ]
    }
  }
}
```

要点：
- `for_each.items` 引用变量（`{ "var": "panels" }`），`for_each.as` 指定当前迭代项变量名，子步骤中通过 `{{p | get "field"}}` 引用
- `max` 可选，默认 100，防止死循环
- `with` 字段支持 `| get` 管道；`code` 字段（脚本步骤）同样支持（v0.1 起）

### 模式 C：表单提交 + 结果验证

```
seq: 提交
├─ 填写 + 提交
├─ browser_wait_for(结果页元素)
├─ assert: 成功标志存在
└─ if: 失败 → screenshot → chat 步骤分析 → 决定重试或终止
```

### 模式 D：桌面应用多窗口操作

```
seq: 跨窗口
├─ desktop_windows_list → 记录所有 hwnd
├─ 窗口 A 操作 → desktop_window_activate(hwnd_a) → ...
├─ 窗口 B 操作 → desktop_window_activate(hwnd_b) → ...
```
hwnd 会变化，每次操作前重新 windows_list。

### 模式 E：大工作流拆分

**何时拆**：单层步骤 > 15 个或嵌套深度 > 3 层时，拆分为子工作流。

```
主工作流:             子工作流 login-flow:
seq                    seq
├─ call: login-flow    ├─ browser_navigate(login)
├─ call: fetch-data    ├─ browser_type(user)
├─ call: send-msg      ├─ browser_type(pass)
└─ call: logout        └─ browser_click(submit)
```

**产出方式**：先 Write 主 workflow.json（含 call 步骤），再逐个 Write 子工作流 JSON 到同目录。不要试图在一轮对话中输出所有 JSON。

### 模式 F：SPA 搜索框提交

**优先级从高到低**（视觉定位优先，不依赖页面 JS 实现）：
1. `browser_snapshot` 找到搜索按钮 `@eN` → `browser_click`
2. 无 ref → `browser_screenshot` → OCR 找按钮坐标 → JS 模拟点击坐标
3. 以上都不行 → `browser_evaluate` 触发 `form.submit()`（最后手段）

---

## 七、工具使用要点

`desktop_input` 输入和发送是一次调用，不要拆成两步。普通文本直接用，不用剪切板。
剪切板只用于 >500 字的大段文本，用后清空。敏感内容禁用剪切板。

### 坐标体系

| 工具 | 输入坐标系 | 输出坐标系 |
|------|-----------|-----------|
| `desktop_perceive` | 图片像素坐标（取决于截图来源） | 同左 |
| `desktop_window_screenshot` | — | 客户区像素 + `screen_x`/`screen_y` 偏移 |
| `desktop_screenshot(region)` | — | 区域像素 + `screen_x`/`screen_y` 偏移 |
| `desktop_mouse` (无 hwnd) | **屏幕绝对坐标** | — |
| `desktop_mouse` (有 hwnd) | 屏幕绝对坐标 | — |

**规则：永远用屏幕坐标调用 `desktop_mouse`。**
perceive 结果 = 客户区坐标时，手动加截图返回的 `screen_x`/`screen_y` 偏移。

### 工具选择速查

| 场景 | 首选 | 备选 |
|------|------|------|
| 定位网页元素 | `browser_snapshot` → @eN ref | `browser_screenshot` + OCR |
| 桌面布局解析 | Vision 全窗口语义分析 | `desktop_find_text`（需字库） |
| 定位桌面文字坐标 | 按 Vision 划定的功能区精准定位 | `desktop_find_text`（需字库） |
| 定位桌面图标 | hover + 小范围 Vision OCR tooltip | `request_user_input(icon_confirm)` |
| 等待网页加载 | `browser_wait_for(selector)` | `system_sleep`（不得已） |
| 验证页面状态 | `browser_snapshot` + `chat` 步骤 | `browser_extract` 文本匹配 |
| 验证桌面状态 | `desktop_window_screenshot` + OCR | `desktop_find_image` 模板匹配 |
| 查经验/布局 | `ui_maps_search(query="微信 im")` — 返回 screens + experiences | `Read` 直接读 ui-maps JSON |
| 跨应用参考 | `ui_maps_search(app_category="im", query="发送")` | 不限类别模糊搜索 |

### 浏览器 snapshot + screenshot 配合流程

```
browser_snapshot（骨架 + ref）
       ↓
  目标元素有 ref？
   ├─ 有 → browser_click / browser_type
   └─ 无 → browser_screenshot → desktop_vision 分析
```
**snapshot 拿不到 ref，立刻截图。不换 JS 策略重试。**

### DOM 数据提取规则

- **卡片边界**：用语义 class 限定（如 `[class*="offer-item"]`），不用固定层级 `closest`
- **图片提取**：按 `data-src` → `data-original` → `src` 顺序 fallback
- **数据隔离**：先 `querySelector` 限定卡片范围再正则。禁止全卡片 `textContent` 跨产品污染

### 连续失败熔断

同类操作失败 ≥3 次 → 停止 → 判断类别：

| 类别 | 动作 |
|------|------|
| 工具限制 | 换工具配合链 |
| 定位失败 | `request_user_input(region)` |
| 反爬拦截 | `request_user_input` 让用户处理 |

### browser_extract 返回空

不重复调用。改用：
`browser_screenshot` → `desktop_vision(prompt:"提取页面所有可见文本")`

---

## 八、Step JSON 速查附录

### loop（for_each）

```json
{ "id": "l1", "name": "遍历列表",
  "do": { "loop": { "for_each": { "items": { "var": "items" }, "as": "it" },
    "max": 100, "do": [/* 子步骤中用 {{it | get "field"}} 引用迭代项字段 */] } } }
```

### loop（until）

```json
{ "id": "l2", "name": "翻页循环",
  "do": { "loop": { "until": { "equals": [ { "var": "has_next" }, "false" ] },
    "max": 100, "do": [/* 子步骤，可用 break / continue */] } } }
```

### loop（repeat）

```json
{ "id": "l3", "name": "重试3次",
  "do": { "loop": { "repeat": 3, "do": [/* 子步骤 */] } } }
```

### if（条件分支）

```json
{ "id": "i1", "name": "需滚动？",
  "do": { "if": {
    "condition": { "equals": [ { "var": "need_scroll" }, "yes" ] },
    "then": [/* matched steps */],
    "else": [/* else steps，可省略 */]
  } } }
```

条件 `var` 支持点号路径下钻：`{ "var": "coords.need_scroll" }` → 取 `variables["coords"]["need_scroll"]`。

可用条件操作：`equals` / `not_equals` / `contains` / `starts_with` / `regex`（取 `VarRef[]`）、`not_empty` / `empty`（取 `VarRef`）、`gt` / `lt` / `gte` / `lte`（取 `VarRef[]`）、`always`（取 boolean）。

### script

```json
{ "id": "s1", "name": "算坐标",
  "do": { "script": { "runtime": "python",
    "code": "import json\nwinfo=json.loads('''{{winfo}}''')\nprint(json.dumps({'ix':winfo['wx']+587}))" } },
  "capture": "coords", "on_error": "abort" }
```

`code` 字段支持 `{{var}}` 替换和 `| get` / `| default` 管道（v0.1 起）。

### tool

```json
{ "id": "t1", "name": "点击",
  "do": { "tool": "desktop_mouse", "with": { "action": "click", "x": "{{coords | get \"ix\"}}" } },
  "capture": "result", "on_error": "abort" }
```

### call（子工作流）

```json
{ "id": "c1", "name": "登录",
  "do": { "call": "login-flow", "with": { "inputs": { "user": "{{username}}" }, "outputs": { "token": "auth_token" } } } }
```

### wait（人工介入）

```json
{ "id": "w1", "name": "手动登录",
  "do": { "wait": "请在浏览器中完成登录后点击继续", "auto": [/* 可选：等待期间可执行的检查步骤 */] } }
```

### chat（LLM 决策）

**ChatAgent 配置四段法**：

ChatAgent 的 persona 必须按四层独立语义设计，每层职责不交叉：

| 层次 | 字段 | 含义 | 允许内容 | 禁止内容 |
|------|------|------|----------|----------|
| 身份定义 | `persona` | 它是谁 | 角色、专业领域、工作方式 | 任务目标、操作步骤 |
| 任务目标 | `goal` | 要达成什么 | 可衡量的目标描述 | 具体怎么做 |
| 约束条件 | `constraints` | 不能做什么 | 边界、红线、禁止行为 | 正向操作指令 |
| 操作规范 | `requirements` | 怎么做 | 工具使用规范、操作流程 | 任务细节、对话内容 |

**正确示例** — 终端操作员：

```json
{
  "id": "terminal-operator",
  "name": "终端操作员",
  "persona": "你是终端操作员，通过桌面自动化工具与 CLI 程序交互。",
  "goal": null,
  "constraints": [
    "不要预设或硬编码对话内容",
    "不要跳过状态确认步骤"
  ],
  "requirements": [
    "发送前先截图确认终端状态",
    "使用 desktop_input mode=type，send=enter 发送"
  ]
}
```

**反模式** — 把任务目标塞进 requirements：

```json
// ❌ 错误：requirements 里混杂了任务约束（"第2轮引用第1轮"）
{
  "persona": "你是终端操作员",
  "requirements": [
    "输入前激活窗口",
    "第2轮必须引用第1轮内容",  // ← 这是任务目标，不是操作规范
    "第3轮引用前两轮"
  ]
}
```

> `goal` 为 null 时，由 workflow step 的 `chat` 消息承载任务目标；有值时作为默认目标。

```json
{ "id": "ca1", "name": "判断登录态",
  "do": { "chat": "根据截图判断是否已登录", "with": {
    "agent_id": "login-checker",
    "screenshot": true,
    "persona": "登录态判断专家",
    "goal": "判断当前页面是否已登录",
    "constraints": ["只输出 LOGGED_IN 或 LOGIN_REQUIRED，不输出其他内容"],
    "requirements": ["先截图观察页面状态再判断"]
  } },
  "capture": "login_status" }
```

### assert（断言验证）

```json
{ "id": "a1", "name": "验证成功",
  "do": { "assert": {
    "condition": { "equals": [ { "var": "status" }, "ok" ] },
    "message": "状态异常"
  } },
  "on_error": "abort" }
```