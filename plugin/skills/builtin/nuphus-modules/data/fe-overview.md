# 前端 · 总览与基础设施（main.tsx / core/ / ui/ / hooks/）

> 定位：整个 React 前端的**装配层、契约层与基础设施层**——应用启动引导、Tauri IPC 桥与事件消费、前后端类型契约、主题/皮肤/音效/图标等横切能力，以及把散落状态收敛为单一数据源的全部 hooks。
> 规模：57 文件（源 43 + 测试 14）。

## 结构

| 分组 | 文件 | 职责 |
|---|---|---|
| 入口 | `main.tsx`（47） | 主题预初始化防首帧闪烁、Provider 层级（Theme → Lang → ErrorBoundary → App）、PDF 渲染服务注册 |
| 契约层 `core/` | `bridge.ts`（820）/ `types.ts`（972）/ `debug.ts` / `executionActivity.ts` / `progressMessages.ts`（146）/ `pdf-render.ts`（158）/ `types-memory.ts` | **全前端唯一**的 invoke/listen 封装（Tauri 通道 + 浏览器 WebSocket 降级 + dev mock）；与后端 serde 结构逐字对齐的类型契约（NuphusEvent 事件联合体、TurnMeta 权威元数据及纯函数出口）；执行活动纯投影；历史折叠/流式回复合成；pdf.js 离屏渲染供 Rust 侧 eval |
| 横切 `ui/`（23 源） | `ErrorBoundary` / `ErrorScreen` / `SplashScreen` / `PageLayout` / `CommandPalette`（270，Ctrl+K）/ `AppIsland`（129，轻反馈层含 HUD 分流）/ `SkinBackdrop` / 音效合成 / 平台快捷键格式化 等 | 全窗口唯一实例的能力组件 |
| 状态收敛 `hooks/`（12 源） | `useSession.ts`（1316）/ `useEvents.ts`（1495）/ `useExecutionState.ts`（145）/ `useExecutionUI.ts`（437）/ `useAgentControl.ts`（450）/ `useInit.ts`（262）/ `useTheme.tsx`（362）/ `customTheme.ts`（310）/ `useModals.ts` / `useStickyScroll.ts`（505）/ `usePanelDrag.ts` / `useKeyboard.ts` | useSession 是组合器（SessionAPI 唯一出口）；useEvents 是 nuphus-event 事件流的**唯一消费者**；useExecutionState 执行态唯一来源（后端 ExecutionStage，事件+1500ms 轮询）；useStickyScroll 消息流贴底跟随手势唯一入口 |

## 功能作用

- **装配与启动**：React 挂载前完成主题预初始化，按固定层级铺 Provider。
- **IPC 契约与桥**：core/bridge.ts 是唯一 invoke/listen 封装（含 WebSocket 降级与 dev mock 注册表）。
- **横切基础设施**：命令面板、island 轻反馈通道（含 HUD 分流）、皮肤背景唯一写入/渲染点、主题 Provider、头像/情绪形象、音效合成、错误边界/错误屏。
- **状态收敛**：useSession 把初始化、模态开关、执行追踪、Agent 控制、执行态缝合为一个 SessionAPI。

## 规则与坑

- 核心工程理念（散见于各文件注释）：**任何横切能力只允许一个实现点、一个写入点、一个数据源**——三处消费方共用同一份实现，杜绝「各写一半必然漂移」。
- 本模块是主窗口地基（另有 `frontend/src/hud/`、`frontend/src/mobile/` 两个独立入口）。
