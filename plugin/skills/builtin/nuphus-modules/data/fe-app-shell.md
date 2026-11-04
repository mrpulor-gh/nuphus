# 前端 · 主窗口骨架与布局（App.tsx / layout/）

> 定位：**主窗口的组件树总装与全局浮层编排中心**——以「无 router、状态驱动」的方式把聊天区、15 个 CompactModal 弹窗族、5 类全屏宿主、执行追踪/思考条/后台任务等浮层组装成一棵树，并独占持有快捷键路由、外链接管、皮肤背景恢复等「只能有一个挂载时点」的横切职责。
> 规模：13 源码 + 4 同目录测试。

## 结构

| 文件 | 规模 | 职责 |
|---|---|---|
| `App.tsx` | 1258 | 主窗口根组件：lazy 分割、hook 组装、全局弹窗/浮层编排、快捷键、横切恢复 |
| `shortcutRouting.ts` | 22 | 主快捷键语义路由（Ctrl+K/Ctrl+F 全模式一致） |
| `layout/CompactModal.tsx` | 132 | **弹窗归一**：全项目唯一模态弹窗实现（portal + 150ms 退场动画 + 可选拖拽） |
| `layout/TitleBar.tsx` | 140 | 自绘标题栏：品牌菜单、窗口控制、移动端汉堡菜单 |
| `layout/StatusBar.tsx` | 140 | 底部状态栏：ctx 进度条、缓存命中率、exec token、步数、耗时（由 ChatPanel 挂载，非 App 直挂） |
| `layout/SecurityPrompt.tsx` | 188 | 安全审批弹窗（允许一次/本次会话/拒绝），**5 分钟超时自动拒** |
| `layout/DesktopActionStatus.tsx` | 23 | 桌面动作回执徽标（「已发送 · 效果未确认」而非通用绿色完成） |
| `layout/WorkflowTaskPanel.tsx` | 419 | 工作流步骤树面板（Win11 风格、手风琴参数只读展开、终止/暂停/紧急停止） |
| `layout/ThinkingIndicator.tsx` | 253 | 思考状态条：阶段色、工具调用徽标、换行脉冲、MoodFace |
| `layout/ExecutionTraceFloating.tsx` | 1564 | 执行追踪浮层：终端/卡片双模式、LCS diff、历史折叠、贴底跟随、点评 |
| `layout/UserInputPrompt.tsx` | 543 | 用户输入请求弹窗：text / icon_confirm / step_form / 截取类四形态 |
| `layout/AppearancePanel.tsx` | 1221 | 外观浮窗（非模态常驻）：系统主题/我的主题/5 条不透明度/5 个核心色 |
| `layout/BackgroundTasksPanel.tsx` | 377 | 后台任务账本：入口胶囊 + 列表面板 + 二次确认结束（2s 轮询） |

## 功能作用

- **Provider/hook 组装**：useLanguage / useSession / useAppUpdate / useEvents / useTheme 的唯一汇聚点；`s.*` 的 60 余个字段/回调经 props 向下广播给 ChatPanel 等子组件。
- **按需加载（代码分割）**：21 个 `lazy()` 页面/弹窗，首屏只加载 TitleBar + ChatPanel 骨架；画布外壳 chunk 另走 `scheduleIdle` 空闲预取。
- **全局弹窗与浮层管理**：15 个 CompactModal + 5 类全屏宿主（模型页/画布/工具页/设置中心/插件宿主）+ 各浮层，全部由 App 层统一挂载与传参。
- **快捷键**：经 useKeyboard 注册 6 条全局快捷键，语义路由抽到 shortcutRouting.ts。
- **横切恢复点**：会话字号恢复、皮肤背景恢复、外链接管、音效预热——「App 常驻、只有一个挂载时点」的职责，注释明确禁止下沉到子页面。

## 规则与坑

- CompactModal 弹窗归一是 DESIGN-RATIONALE 铁律（全项目唯一模态形态）。
- 移动端「面板注入」用 CSS 覆写而非第二套 UI（fe-app-shell.md §4.6）。
- 快捷键历史分裂（各页各自处理 Ctrl+K/F）已修复为 shortcutRouting 全模式一致。
