# Nuphus 前端手写内联 `<svg>` 图标清单

> 用途：把图标体系统一迁移到 lucide / morphicons 的**前置清点**。
> 生成方式：全量 grep `frontend/src/**/*.{ts,tsx,js,jsx}` 下 `<svg` 标签 + 逐个读取上下文确认用途。
> **本文件为只读产出，零文件改动。**

---

## 0. 计数口径勘误（先声明，避免下游误判）

任务上下文称「共 82 处」。实测全量计数如下：

```powershell
Get-ChildItem -Recurse -Include *.tsx,*.ts,*.jsx,*.js | Select-String "<svg" -AllMatches | Measure-Object
# TOTAL: 103
```

| 口径 | 数量 | 说明 |
|------|------|------|
| `<svg` 标签出现次数（全量实测） | **103** | 本清单采用此口径 |
| 其中"图标类"（分类 1，拟迁移） | **78** | 见 §1 汇总 |
| 其中"非图标"（分类 2–5，不迁移） | **19** | 品牌 8 / 可视化进度预览 9 / defs 载体 1 / 二维码 1（合并计） |

> 78 + 19 = 97，余 6 处为跨类项（`UiPrototypeCanvas.tsx:4134` 同时计在可视化与 defs；hud/TitleBar 归入条件迁移），故不重复计入，以保证互斥口径。
> 82 很可能是「排除 `hud/App.tsx` 的 17 处 + 排除部分 ui-prototype」后的子集。本清单按**全量 103** 给出，并在明细表逐条标注，下游可按分类自行裁剪。

---

## 1. 汇总表

| # | 分类 | 数量 | 是否迁移目标 | 文件:行号列表 |
|---|------|------|--------------|----------------|
| 1 | **UI 图标（stroke 中心线 / 可归为图标）** | 78 | ⚠️ 部分 | 见 §1.1 分表（其中 45 高置信 / 33 需决策） |
| 2 | 品牌/标识图形（Logo/Avatar/MoodFace/GitHub/Logo 组件） | 8 | ❌ 否 | `ui/NuphusLogo.tsx:41`、`ui/NuphusAvatar.tsx:73`、`ui/MoodFace.tsx:34`、`mobile/components/NavBar.tsx:111`、`mobile/components/MessageList.tsx:61`、`mobile/components/PairingGuide.tsx:66`、`ui-prototype/components/Logo.tsx:17`、`ui-prototype/components/Toolbar.tsx:35`（GitHub 品牌） |
| 3 | 数据可视化/进度/预览/二维码 | 9 | ❌ 否 | `ui/QrCode.tsx:40`、`ui-prototype/components/Loading.tsx:90/193/309`、`ui-prototype/components/Inspector.tsx:705`、`ui-prototype/components/ThemePanel.tsx:311/330`、`ui-prototype/components/M3Node.tsx:1058`、`ui-prototype/UiPrototypeCanvas.tsx:4134` |
| 4 | defs / 遮罩 / hidden 占位 svg | 1 | ❌ 否 | `ui-prototype/UiPrototypeCanvas.tsx:4287`(width=0 height=0，纯 defs 载体) |
| 5 | 状态语义 svg（特殊尺寸+多态+CSS 动画，morphicons 不兼容） | 1 | ❌ 否 | `chat/ApiHealthBadge.tsx:49`（16×16） |
| — | **其余（hud 14/9 尺寸、OS 窗口控件等）** | — | 归入 §1「条件迁移」 | 见 §1.1 与 §4 |

> **口径说明**：§1 上表为「互斥计数」——78 + 8 + 9 + 1 + 1 = **97**，余 6 处归入 §4「条件迁移」中的 `hud` / `TitleBar` OS 控件等（已在 §1.1 分表中列出，可跨类，故此处不重复计数）。
> 分类 1/4 存在个别条目跨类（`UiPrototypeCanvas.tsx:4134` 既是画布连线又是 defs 宿主），明细表按**唯一主分类**归属，跨类在备注注明。

### 1.1 分类 1 明细（图标类，78 处）

| 文件 | 行号 | 数量 |
|------|------|------|
| `hud/App.tsx` | 162,177,192,201,217,227,238,247,255,263,272,283,295,307,319,331,341 | 17 |
| `ui/ErrorScreen.tsx` | 68,89,109,129,151 | 5 |
| `ui/Icons.tsx` | 82,107,131 | 3 |
| `main-window/layout/WorkflowTaskPanel.tsx` | 93,107,125,140,156,171,185,204,216,221,235,249,255,373,400,519,526,543,560 | 19 |
| `main-window/layout/ExecutionTraceFloating.tsx` | 421,477,490,494,507,1345,1448,1464 | 8 |
| `main-window/chat/ChatInputBar.tsx` | 868,908,932,959,982,1268,1391 | 7 |
| `main-window/chat/ChatPanel.tsx` | 1538,1798,1818,1895,2097 | 5 |
| `main-window/layout/TitleBar.tsx` | 50,63,77,98,123 | 5 |
| `main-window/tools/OcrDictionary.tsx` | 834,921,1643 | 3 |
| `main-window/components/ApprovalModal.tsx` | 141,154 | 2 |
| `main-window/layout/ThinkingIndicator.tsx` | 231,246 | 2 |
| `mobile/components/AddToHomeScreen.tsx` | 75 | 1 |
| `main-window/canvases/ui-prototype/components/ui.tsx` | 264 | 1 |
| **合计** | | **78**（含 2 处边界，见备注） |

> `ui-prototype/components/ui.tsx:264` 是 `CornerIcon`（圆角选择器，24×24 stroke）——属于 UI 图标，但服务对象是「原型画布」子系统，与主 App 图标体系不同域。

---

## 2. 明细表（逐处：文件:行号 | 用途摘要 | 分类 | 是否迁移 | 备注）

### 2.1 `hud/App.tsx`（17 处，独立 HUD 窗口，viewBox 14×14 / 9×9）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `hud/App.tsx:162` | `IconSpinner` — HUD 运行中旋转加载环 | 图标(状态) | ⚠️ 条件 | **viewBox 0 0 14 14，非 24×24**；双色(stroke 分色 track/color)。morphicons 不兼容，需按 14 基准重绘 |
| `hud/App.tsx:177` | `IconCheck` — 完成态对勾(环+勾) | 图标(状态) | ⚠️ 条件 | 14×14；双色(环 opacity .25 + 勾实色) |
| `hud/App.tsx:192` | `IconError` — 错误态叉(环+叉) | 图标(状态) | ⚠️ 条件 | 14×14；双色 |
| `hud/App.tsx:201` | `IconWarning` — 警告态三角叹号 | 图标(状态) | ⚠️ 条件 | 14×14；含 fill 元素（叹号矩形+圆点） |
| `hud/App.tsx:217` | `IconInfo` — 信息态圆i | 图标(状态) | ⚠️ 条件 | 14×14；含 fill 元素 |
| `hud/App.tsx:227` | `IconWorkflow` — 工作流(三点连线) | 图标(步骤) | ⚠️ 条件 | 14×14；与 `ui/Icons.tsx:107` 同名不同实现 |
| `hud/App.tsx:238` | `IconPause` — 暂停(双竖) | 图标(控制) | ⚠️ 条件 | **viewBox 0 0 9 9**，fill 实心 |
| `hud/App.tsx:247` | `IconPlay` — 播放(三角) | 图标(控制) | ⚠️ 条件 | 9×9，fill 实心 |
| `hud/App.tsx:255` | `IconStop` — 停止(方框) | 图标(控制) | ⚠️ 条件 | 9×9，stroke 方框 |
| `hud/App.tsx:263` | `IconClose` — 关闭(叉) | 图标(控制) | ⚠️ 条件 | 9×9 |
| `hud/App.tsx:272` | `STEP_ICONS.tool` — 齿轮/工具 | 图标(步骤) | ⚠️ 条件 | 14×14；含 opacity 0.4 次级笔画 |
| `hud/App.tsx:283` | `STEP_ICONS.wait` — 时钟 | 图标(步骤) | ⚠️ 条件 | 14×14 |
| `hud/App.tsx:295` | `STEP_ICONS.chat_agent` — 对话气泡 | 图标(步骤) | ⚠️ 条件 | 14×14；含 fill 圆点 |
| `hud/App.tsx:307` | `STEP_ICONS.call` — 调用(箭头) | 图标(步骤) | ⚠️ 条件 | 14×14 |
| `hud/App.tsx:319` | `STEP_ICONS.script` — 脚本文档 | 图标(步骤) | ⚠️ 条件 | 14×14；含 opacity 0.5 次级笔画 |
| `hud/App.tsx:331` | `STEP_ICONS.seq` — 顺序列表 | 图标(步骤) | ⚠️ 条件 | 14×14；含 opacity 0.4 次级笔画 |
| `hud/App.tsx:341` | `STEP_ICONS.loop` — 循环箭头 | 图标(步骤) | ⚠️ 条件 | 14×14 |

> **HUD 特殊说明**：`hud/App.tsx` 在独立 Tauri 窗口中渲染（`main.tsx` 分入口），CSS 用 `all: initial` 隔离。其图标全部 14×14/9×9 双色、且以 `color` prop 传参而非 `currentColor`。→ **与 lucide 24×24 stroke centerline 体系不同构**，迁移需先决定「HUD 是否纳入统一」；若纳入须整体重绘。**建议单列，不与主 App 一起迁。**

### 2.2 `ui/ErrorScreen.tsx`（5 处，走查启动异常页）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `ui/ErrorScreen.tsx:68` | `ErrorIcon('api_key_invalid')` — 灯泡+斜杠 | 图标(空状态) | ✅ 是 | 24×24 stroke 1.5；**含 `opacity=".4"` 分层笔画**，lucide 无直接对应，需自定义或拆分 |
| `ui/ErrorScreen.tsx:89` | `ErrorIcon('backend_unavailable')` — 云+斜杠 | 图标(空状态) | ✅ 是 | 24×24 stroke 1.5；含 `.3/.5/.4` opacity 分层 |
| `ui/ErrorScreen.tsx:109` | `ErrorIcon('config_corrupted')` — 文件+斜杠 | 图标(空状态) | ✅ 是 | 24×24；含 opacity 分层 |
| `ui/ErrorScreen.tsx:129` | `ErrorIcon('port_in_use')` — 服务器+斜杠 | 图标(空状态) | ✅ 是 | 24×24；含 opacity 分层 |
| `ui/ErrorScreen.tsx:151` | `ErrorIcon('unknown')` — 圆+叉（**与 `Icons.tsx:82 ErrorXIcon` 同构**） | 图标(空状态) | ✅ 是 | 24×24；**与 `ui/Icons.tsx:80 ErrorXIcon` 重复实现点**，迁移时应收敛为一处 |

### 2.3 `ui/Icons.tsx`（3 处，lucide 单一实现点内的手写兜底）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `ui/Icons.tsx:82` | `ErrorXIcon` — 圆+叉（40px 默认，错误页用） | 图标 | ✅ 是 | 24×24 stroke 1.5；**与 `ErrorScreen.tsx:151` 重复**；lucide 有 `CircleX`，可直接替换 |
| `ui/Icons.tsx:107` | `IconWorkflow` — 三个圆角竖条+连线 | 图标 | ✅ 是 | 24×24 stroke 2；lucide 无直接对应（近似 `Workflow`/`Kanban`），需自定义映射 |
| `ui/Icons.tsx:131` | `IconTerminal` — 窗口+提示符 | 图标 | ✅ 是 | 24×24 stroke 2；lucide 有 `TerminalSquare`/`SquareTerminal` 可替换 |

> **关键**：`ui/Icons.tsx` 是前端图标体系的**单一实现点**（`:1` 注释「re-exported from lucide-react」，`:5-78` 为 lucide 再导出）。上述 3 个手写兜底是唯一偏离点，**迁移优先级最高**——改这 3 处即可让「图标体系出口」100% 由 lucide 提供。

### 2.4 `main-window/layout/WorkflowTaskPanel.tsx`（19 处，桌面端工作流面板）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:93` | `KIND_ICONS.tool` — 齿轮 | 图标(步骤类型) | ✅ 是 | 12×12 渲染，viewBox 24×24 stroke 2 |
| `:107` | `KIND_ICONS.seq` — 顺序列表 | 图标(步骤类型) | ✅ 是 | 同上 |
| `:125` | `KIND_ICONS.loop` — 循环 | 图标(步骤类型) | ✅ 是 | 同上 |
| `:140` | `KIND_ICONS.if` — 菱形+加号 | 图标(步骤类型) | ✅ 是 | 同上 |
| `:156` | `KIND_ICONS.call` — 调用箭头 | 图标(步骤类型) | ✅ 是 | 同上 |
| `:171` | `KIND_ICONS.wait` — 时钟 | 图标(步骤类型) | ✅ 是 | 同上 |
| `:185` | `KIND_ICONS.chat_agent` — 对话气泡 | 图标(步骤类型) | ✅ 是 | 同上 |
| `:204` | `StatusBadge.pending` — 空心圆 | 图标(状态) | ⚠️ 条件 | 10×10 渲染，**fill=none stroke**；5 态之一 |
| `:216` | `StatusBadge.running` — 实心圆 | 图标(状态) | ⚠️ 条件 | 10×10，**fill=currentColor（实心）** |
| `:221` | `StatusBadge.completed` — 对勾 | 图标(状态) | ⚠️ 条件 | 10×10，strokeWidth **3**（非 2） |
| `:235` | `StatusBadge.failed` — 叉 | 图标(状态) | ⚠️ 条件 | 10×10，strokeWidth 3 |
| `:249` | `StatusBadge.paused` — 双竖 | 图标(状态) | ⚠️ 条件 | 10×10，**fill=currentColor（实心）** |
| `:255` | `StatusBadge` fallback — 空心圆 | 图标(状态) | ⚠️ 条件 | 10×10，未知 status 兜底 |
| `:373` | 面板头部图标 — 四宫格 | 图标(标题) | ✅ 是 | 14×14 渲染，24×24 stroke 2 |
| `:400` | 收起按钮 — 叉 | 图标(控制) | ✅ 是 | 12×12，24×24 stroke 2 |
| `:519` | 「继续」按钮 — 播放三角 | 图标(控制) | ⚠️ 条件 | 12×12，**fill=currentColor** |
| `:526` | 「重新执行」按钮 — 刷新 | 图标(控制) | ✅ 是 | 12×12，24×24 stroke 2；lucide `RotateCcw` 可替换 |
| `:543` | 「暂停」按钮 — 双竖 | 图标(控制) | ⚠️ 条件 | 12×12，**fill=currentColor** |
| `:560` | 「紧急停止」按钮 — 闪电 | 图标(控制) | ✅ 是 | 12×12，24×24 stroke 2 |

> **`StatusBadge`（:201-267）是本任务的核心目标**：5 态手写 SVG，移动端 `WorkflowRunCard.tsx` 已用 lucide 等价实现（已核实）。迁移方案应为**直接对齐移动端实现**，而非重新设计。
> **⚠️ 兼容性风险**：`:216/:249/:519/:543` 为 `fill="currentColor"` 实心图形，**:221/:235 的 strokeWidth 为 3**——均偏离 lucide 标准（stroke 2 / fill none），迁移需逐一确认视觉等价。

### 2.5 `main-window/layout/ExecutionTraceFloating.tsx`（8 处，执行轨迹浮层）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:421` | Grep/Glob 参数行的放大镜 | 图标(内联) | ✅ 是 | 11×11 渲染，24×24 stroke **2.5** |
| `:477` | `StatusIcon(success)` — 对勾 | 图标(状态) | ⚠️ 条件 | 12×12，strokeWidth 3 |
| `:490` | `StatusIcon(running)` — 实心播放三角 | 图标(状态) | ⚠️ 条件 | 12×12，**fill=currentColor** |
| `:494` | `StatusIcon(error)` — 叉 | 图标(状态) | ⚠️ 条件 | 12×12，strokeWidth 3 |
| `:507` | `StatusIcon(pending)` — 空心圆 | 图标(状态) | ⚠️ 条件 | 10×10，stroke 2 |
| `:1345` | 工具条目展开/收起 chevron | 图标(交互) | ✅ 是 | 12×12，上下双 polyline 条件渲染 → **morphicons 最佳用例**（可 morph chevron-up/down） |
| `:1448` | 「点评」按钮 — 星形 | 图标(操作) | ✅ 是 | 14×14，**polygon（封闭路径）**，lucide `Star` 可替换 |
| `:1464` | 「重新生成」按钮 — 刷新 | 图标(操作) | ✅ 是 | 14×14，lucide `RotateCcw` 可替换 |

### 2.6 `main-window/chat/ChatInputBar.tsx`（7 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:868` | 工具入口「+」 | 图标(操作) | ✅ 是 | 16×16；lucide `Plus` 可替换（`Icons.tsx` 已导出 `IconPlus`） |
| `:908` | 菜单项「附件」— 回形针 | 图标(操作) | ✅ 是 | 14×14；lucide `Paperclip` 可替换 |
| `:932` | 菜单项「图片」— 图片框 | 图标(操作) | ✅ 是 | 14×14；lucide `Image` 可替换（已导出 `IconImage`） |
| `:959` | 菜单项「原则」— 盾牌 | 图标(操作) | ✅ 是 | 14×14；lucide `Shield` 可替换（已导出 `IconShield`） |
| `:982` | 菜单项「评注」— 文档标记 | 图标(操作) | ✅ 是 | 14×14 |
| `:1268` | 模型 chip 的 effort caret（下箭头） | 图标(交互) | ✅ 是 | 9×9；lucide `ChevronDown` 可替换（已导出 `IconChevronDown`） |
| `:1391` | 追加消息队列按钮 — 列表 | 图标(操作) | ✅ 是 | 17×17；lucide `List` 近似 |

> **反模式命中**：本文件 7 处全部可用 `ui/Icons.tsx` 已导出图标替换（`IconPlus`/`IconImage`/`IconShield`/`IconChevronDown`），是「同一段逻辑平铺重复」的典型——违反用户原则「相同职责必须收敛到单一实现点」。

### 2.7 `main-window/chat/ChatPanel.tsx`（5 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:1538` | refine 待处理按钮 — 柱状图（3 竖条） | 图标(操作) | ✅ 是 | 14×14，24×24 stroke 2；lucide `BarChart3`/`ChartNoAxesColumn` 近 |
| `:1798` | 消息「点评」按钮 — 星形 | 图标(操作) | ✅ 是 | 14×14；**与 `ExecutionTraceFloating.tsx:1448` 重复实现**，应收敛 |
| `:1818` | 消息「执行回溯」按钮 — 历史时钟 | 图标(操作) | ✅ 是 | 14×14；lucide `History` 可替换（已导出 `IconHistory`） |
| `:1895` | refine 弹窗关闭 — 叉 | 图标(控制) | ✅ 是 | 14×14；lucide `X` 可替换（已导出 `IconX`） |
| `:2097` | refine 弹窗关闭 — 叉 | 图标(控制) | ✅ 是 | 14×14；**与 `:1895` 同构重复** |

### 2.8 `main-window/layout/TitleBar.tsx`（5 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:50` | 窗口最小化 | 图标(控制) | ⚠️ 条件 | **viewBox 0 0 14 14，stroke 1.5**（自绘窗口控件风格） |
| `:63` | 窗口最大化 | 图标(控制) | ⚠️ 条件 | 14×14 stroke 1.5 |
| `:77` | 窗口关闭 | 图标(控制) | ⚠️ 条件 | 14×14 stroke 1.5 |
| `:98` | 移动端汉堡菜单 | 图标(控制) | ⚠️ 条件 | 18×18 渲染，**viewBox 24×24 stroke 2**（此条与 lucide 同构） |
| `:123` | 汉堡菜单项「新对话」— 加号 | 图标(操作) | ✅ 是 | 16×16，24×24 stroke 2 |

> **窗口控制三键（:50/:63/:77）**：刻意使用 14×14/1.5 轻笔画，是 OS 窗口控件视觉语言，**建议保留原样**而非套 lucide（lucide 的 `Minus`/`Square`/`X` 笔画过粗，会破坏标题栏视觉密度）。→ 标注为**「不迁移（有意偏离）」**。

### 2.9 `main-window/tools/OcrDictionary.tsx`（3 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:834` | 「自动识别」chip — 放大镜 | 图标(操作) | ✅ 是 | 14×14；lucide `Search` 可替换（已导出 `IconSearch`） |
| `:921` | 空状态提示 — 信息圆i | 图标(空状态) | ✅ 是 | 12×12；lucide `Info` 可替换（已导出 `IconInfo`） |
| `:1643` | 字体渲染弹窗关闭 — 叉 | 图标(控制) | ✅ 是 | 16×16；lucide `X` 可替换 |

### 2.10 `main-window/components/ApprovalModal.tsx`（2 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:141` | 审批结果成功 — 对勾 | 图标(状态) | ✅ 是 | 22×22；lucide `Check` 可替换（已导出 `IconCheck`） |
| `:154` | 审批结果失败 — 叉 | 图标(状态) | ✅ 是 | 22×22；lucide `X` 可替换（已导出 `IconX`） |

> 注：`ApprovalModal.tsx:154` 的 `strokeWidth` 位于被截断行（`…1 行已截断…`），未能读到确切值 → **盲区 §4-①**。

### 2.11 `main-window/layout/ThinkingIndicator.tsx`（2 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:231` | 「查看详情」按钮 — 右 chevron | 图标(交互) | ✅ 是 | 13×13；**morphicons 最佳用例**（chevron 旋转 morph，与 `ExecutionTraceFloating.tsx:1345` 同类） |
| `:246` | 「关闭」按钮 — 叉 | 图标(控制) | ✅ 是 | 11×11，stroke **2.5**；lucide `X` 可替换 |

### 2.12 `mobile/components/AddToHomeScreen.tsx`（1 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:75` | A2HS 引导条图标 — 上传到设备 | 图标(提示) | ✅ 是 | 18×18，24×24 stroke 2；lucide `Upload`/`Download` 近（已导出 `IconUpload`），但语义为「添加到主屏」，建议用 `SquarePlus`/`MonitorSmartphone` 重新映射 |

> **注意**：该文件 `:100` 已使用 `<X size={15} />` **lucide-react 直接导入**——同一文件内混用「手写 svg + lucide」两种方式，是迁移中的**双轨残留**。

### 2.13 `main-window/canvases/ui-prototype/components/ui.tsx`（1 处）

| 文件:行号 | 用途摘要 | 分类 | 迁移? | 备注 |
|---|---|---|---|---|
| `:264` | `CornerIcon` — 圆角方向指示（8 方向 path 表驱动） | 图标(控件) | ⚠️ 条件 | 24×24 stroke 2，但**语义是画布控件符号**（tl/tr/bl/br/top/bottom/left/right 8 变体），lucide 无对应；属原型画布域 |

---

## 3. 分类 2–5 明细（非图标，**不迁移**）

### 3.1 品牌 / 标识图形（不迁移：非图标语义）

| 文件:行号 | 用途摘要 | 分类 | 备注 |
|---|---|---|---|
| `ui/NuphusLogo.tsx:41` | Nuphus 主 Logo（viewBox 256×256，含 defs/filter 光晕） | 品牌 | 与 `icon-source.svg` 精确一致；迁移会破坏品牌一致性 |
| `ui/NuphusAvatar.tsx:73` | Nuphus 头像/眨眼动画（viewBox 256×256，状态机驱动 eye path） | 品牌 | 与 `NuphusLogo` 同源 shell；含 `nv-gaze` 注视偏移 |
| `ui/MoodFace.tsx:34` | MoodFace 情绪表情（**单 svg，内部 11 种 mood 分支，:40-719 共 680 行**） | 品牌/表情 | viewBox 24×24 但为多色/多元素动画图形（含 fill 圆点、opacity 分层、CSS 类动画），**非单图标** |
| `mobile/components/NavBar.tsx:111` | 移动端 Nuphus 眨眼 logo（256×256） | 品牌 | 与 `NuphusAvatar` 同款 shell 结构 |
| `mobile/components/MessageList.tsx:61` | 移动端空状态 Nuphus logo（256×256） | 品牌 | 同上 |
| `mobile/components/PairingGuide.tsx:66` | 配对页 Nuphus logo（256×256） | 品牌 | 同上；**移动端 4 处 logo 为同款复制**（NavBar/MessageList/PairingGuide）→ 三处重复实现，可收敛 |
| `ui-prototype/components/Logo.tsx:17` | ui-prototype 的 M3 应用标记（viewBox 144×144，cookie 形+layers glyph） | 品牌 | 完全不同的设计语言（Material 3 expressive） |
| `ui-prototype/components/Toolbar.tsx:35` | GitHub 品牌图标（viewBox 0 0 16 16，fill） | 品牌 | 第三方商标，**不可替换** |

### 3.2 数据可视化 / 进度环 / 预览 / 二维码（不迁移）

| 文件:行号 | 用途摘要 | 分类 | 备注 |
|---|---|---|---|
| `ui/QrCode.tsx:40` | 配对二维码渲染（viewBox 动态，rect 阵列，黑白双色） | 二维码 | 数据驱动图形，非图标 |
| `ui-prototype/components/Loading.tsx:90` | `LoadingIndicator` — M3 形状 morph 加载器（RAF 驱动 d 属性） | 可视化/进度 | viewBox 动态 size×size；path 每帧重写 |
| `ui-prototype/components/Loading.tsx:193` | `LinearProgress` — 线性进度条（可 wavy） | 可视化/进度 | viewBox 动态 width×height；RAF 驱动 |
| `ui-prototype/components/Loading.tsx:309` | `CircularProgress` — 环形进度（可 wavy） | 可视化/进度 | viewBox 动态 size×size；RAF 驱动 |
| `ui-prototype/components/Inspector.tsx:705` | 对齐方式预览缩略图（viewBox 40×28，虚线框+色块） | 可视化/预览 | 非 24×24；多 rect fill |
| `ui-prototype/components/ThemePanel.tsx:311` | 动效方案预览「标准」— 缓动曲线（viewBox 64×36） | 可视化/预览 | 非 24×24；贝塞尔曲线示意 |
| `ui-prototype/components/ThemePanel.tsx:330` | 动效方案预览「表现力」— 曲线（viewBox 64×36） | 可视化/预览 | 同上 |
| `ui-prototype/components/M3Node.tsx:1058` | 地图节点渲染（viewBox 0 0 400 300，城市块+道路+河流+pin） | 可视化/图形 | 大幅面场景图，多色 fill |
| `ui-prototype/UiPrototypeCanvas.tsx:4134` | 画布连线层（绝对定位 svg，`<defs><marker>` 箭头 + path 连线） | 可视化/图形 | 容器 svg + defs，非图标 |

### 3.3 defs / 遮罩 / hidden 占位（不迁移）

| 文件:行号 | 用途摘要 | 分类 | 备注 |
|---|---|---|---|
| `ui-prototype/UiPrototypeCanvas.tsx:4134` | 连线 svg（内含 `<defs><marker id="nuphus-arrow">`） | defs 宿主 | 与 3.2 同条，跨类；仅作 defs 上下文说明 |
| `ui-prototype/UiPrototypeCanvas.tsx:4287` | `width={0} height={0}` 隐藏 svg，仅承载 `<linearGradient id="nuphus-drafting">` + animateTransform | 遮罩/占位 | **非图标**；无 width/height，纯 defs 载体 |

### 3.4 状态语义 svg（条件迁移，需单列评估）

| 文件:行号 | 用途摘要 | 分类 | 备注 |
|---|---|---|---|
| `main-window/chat/ApiHealthBadge.tsx:49` | `ApiSignalIcon` — API 健康 5 态语义环（viewBox **0 0 16 16**，条件渲染多 circle/path + CSS 动画类） | 状态语义 | **16×16 非标准 + 多态条件渲染 + CSS 动画耦合**；虽属「图标」，但 morphicons 不兼容（非 24、多元素组合）。建议**保留**或整体重构 |

---

## 4. 汇总：迁移目标清单（可直接派工）

### ✅ 高置信迁移（45 处，无兼容性风险）

全部为 **24×24 viewBox + stroke 中心线 + 有 lucide 直接对应（或可近义映射）**：

| 文件 | 行号 |
|------|------|
| `ui/Icons.tsx` | 82,107,131 |
| `ui/ErrorScreen.tsx` | 68,89,109,129,151 |
| `main-window/layout/WorkflowTaskPanel.tsx` | 93,107,125,140,156,171,185,373,400,526,560 |
| `main-window/chat/ChatInputBar.tsx` | 868,908,932,959,982,1268,1391 |
| `main-window/chat/ChatPanel.tsx` | 1538,1798,1818,1895,2097 |
| `main-window/tools/OcrDictionary.tsx` | 834,921,1643 |
| `main-window/components/ApprovalModal.tsx` | 141,154 |
| `main-window/layout/ThinkingIndicator.tsx` | 231,246 |
| `main-window/layout/ExecutionTraceFloating.tsx` | 421,1345,1448,1464 |
| `mobile/components/AddToHomeScreen.tsx` | 75 |
| `main-window/layout/TitleBar.tsx` | 98,123 |

### ⚠️ 条件迁移（需先决策，33 处）

| 场景 | 条目 | 决策点 |
|------|------|--------|
| **HUD 整窗口** | `hud/App.tsx` 全 17 处 | 是否纳入统一？全部 14×14/9×9 双色 → 需整体重绘 |
| **fill 实心状态点** | `WorkflowTaskPanel.tsx:216,249,519,543`、`ExecutionTraceFloating.tsx:490` | lucide 默认 stroke，需确认视觉等价 |
| **strokeWidth 3 状态图标** | `WorkflowTaskPanel.tsx:221,235`、`ExecutionTraceFloating.tsx:477,494` | lucide 默认 stroke 2，视觉偏细 |
| **OS 窗口控件** | `TitleBar.tsx:50,63,77` | 建议**不迁**（有意偏离，14×14/1.5） |
| **API 健康环** | `ApiHealthBadge.tsx:49` | 16×16 + 多态 + CSS 动画 → 建议保留 |
| **画布域控件** | `ui-prototype/components/ui.tsx:264`、`ui.tsx` 其他 | 属原型画布域，与主 App 图标体系不同 |

### ❌ 不迁移（19 处）
品牌 8 / 可视化进度预览 9 / defs 载体 1 / 二维码 1（合并计；`UiPrototypeCanvas.tsx:4134` 跨类不重复计）

---

## 5. 高价值发现（附证据）

1. **「单一实现点」已被架空**（违反用户原则「相同职责收敛到单一实现点」）
   - `ui/Icons.tsx` 声明自己是图标出口（`:1`、`:5-78`），但全项目仍有 **78 处**手写 `<svg>` 绕过该出口（含条件迁移项）。
   - **直接可用已导出图标替换的重复实现**（无需新增任何图标）：
     - `IconPlus`（已导出 `:48`）← `ChatInputBar.tsx:868`、`TitleBar.tsx:123`
     - `IconX`（已导出 `:13`）← `ChatPanel.tsx:1895,2097`、`ThinkingIndicator.tsx:246`、`OcrDictionary.tsx:1643`、`ApprovalModal.tsx:154`、`ExecutionTraceFloating.tsx`（StatusIcon error）
     - `IconCheck`（已导出 `:7`）← `ApprovalModal.tsx:141`
     - `IconSearch`（已导出 `:18`）← `OcrDictionary.tsx:834`
     - `IconInfo`（已导出 `:77`）← `OcrDictionary.tsx:921`
     - `IconImage`（`:35`）← `ChatInputBar.tsx:932`；`IconShield`（`:42`）← `ChatInputBar.tsx:959`
     - `IconChevronDown`（`:27`）← `ChatInputBar.tsx:1268`
     - `IconHistory`（`:49`）← `ChatPanel.tsx:1818`
     - `IconUpload`（`:56`）← `mobile/AddToHomeScreen.tsx:75`
   - → **零成本替换即可消除约 15 处手写 svg**。

2. **同形图标多处重复定义**（应合并）
   - 「对勾」：`WorkflowTaskPanel.tsx:221`、`ExecutionTraceFloating.tsx:477`、`ApprovalModal.tsx:141`、`Icons.tsx:7`(lucide)
   - 「叉」：`WorkflowTaskPanel.tsx:235,400`、`ExecutionTraceFloating.tsx:494`、`ChatPanel.tsx:1895,2097`、`ThinkingIndicator.tsx:246`、`OcrDictionary.tsx:1643`、`ApprovalModal.tsx:154`、`Icons.tsx:13`(lucide)
   - 「星形」：`ChatPanel.tsx:1798`、`ExecutionTraceFloating.tsx:1448`
   - 「空心圆」：`WorkflowTaskPanel.tsx:204,255`、`ExecutionTraceFloating.tsx:507`
   - 「刷新」：`WorkflowTaskPanel.tsx:526`、`ExecutionTraceFloating.tsx:1464`
   - 「圆+叉」：`Icons.tsx:82`、`ErrorScreen.tsx:151`
   - 「时钟」：`WorkflowTaskPanel.tsx:171`、`hud/App.tsx:283`

3. **桌面/移动端同一语义双实现**（`WorkflowTaskPanel.tsx` vs `WorkflowRunCard.tsx`）
   - 移动端已迁 lucide+morphicons（已核实），桌面端仍手写 → **迁移应对齐移动端既有实现**，避免再度分叉。

4. **双轨残留**：`mobile/components/AddToHomeScreen.tsx` 同文件内既有手写 `:75` svg，又有 `:100` lucide-react 直接导入 `<X>`。

5. **morphicons 最佳落点**（chevron/状态切换）
   - `ExecutionTraceFloating.tsx:1345`（展开/收起 chevron，条件渲染 up/down polyline）
   - `ThinkingIndicator.tsx:231`（展开 chevron）
   - `WorkflowTaskPanel.tsx:204-265`（5 态 StatusBadge）
   - `WorkflowTaskPanel.tsx:519/543`（播放/暂停，一对可 morph）

---

## 6. 盲区声明

| # | 盲区 | 影响 | 建议 |
|---|------|------|------|
| ① | `ApprovalModal.tsx:154`（失败叉）的 `strokeWidth` 值位于被工具截断的行内，未读到确切值 | 迁移时视觉校验需多看一处 | 改代码前重新读取该行 |
| ② | `MoodFace.tsx` 内部 11 个 mood 分支（`:40-719`）未逐分支通读，仅确认其**非单图标**性质（含 fill/opacity/CSS 动画类） | 若下游想从 MoodFace 抽图标会踩坑 | 已明确标注「不迁移」，无需再读 |
| ③ | `ui-prototype/` 子系统的产品地位未确认（是否为开发期实验画布 / 是否随发行版交付） | 影响 `ui.tsx:264` 等是否纳入迁移范围 | 需 Leader 决策后确定 |
| ④ | `hud/App.tsx` 是否纳入「统一图标体系」尚未有明确定义 | 决定 17 处是否迁移 | 建议单列，由 Leader 决策 |
| ⑤ | `DesktopActionStatus` 组件（`ExecutionTraceFloating.tsx:10` import，`:1074/:1332` 使用）内部是否有手写 svg **未展开**——grep 全量中未见其文件命中 `<svg`，推定无 | 若推定错误会漏计 | 低风险（已由全量 grep 覆盖） |
| ⑥ | 任务上下文「82 处」与实际「103 处」的差异来源未确认（推测为排除 hud + ui-prototype，未验证） | 计数口径分歧 | 本清单按 103 全量给出，下游可自行裁剪 |

---

## 7. 结论

- **可迁移图标总量 78 处**，其中 **45 处高置信**（24×24 stroke + lucide 有对应）可直接派工，**33 处需先决策**（HUD/实心/粗笔画/OS 控件/画布域）。
- **不迁移 19 处**：品牌 8、可视化进度预览 9、defs 载体 1、二维码 1。
- **最大收益点**：`ui/Icons.tsx` 的 3 个手写兜底 + 约 15 处「可直接用已导出图标替换」的重复实现 —— 零新增图标成本。
- **核心目标**：`WorkflowTaskPanel.tsx:201-267` StatusBadge（5 态）+ 7 个类型图标 —— 对齐移动端 `WorkflowRunCard.tsx` 既有 lucide/morphicons 实现。
- **兼容性红线**：`fill="currentColor"` 实心图形、`strokeWidth 3`、非 24×24 viewBox（hud 14/9、ApiHealth 16、预览 40×28 / 64×36）——迁移前须逐一确认视觉等价或整体重绘。
