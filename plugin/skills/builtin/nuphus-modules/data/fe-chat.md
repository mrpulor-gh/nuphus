# 前端 · 聊天区（main-window/chat/）

> 定位：**桌面 AI Agent 的唯一对话界面**——把「消息渲染管线（Markdown/代码高亮/本地图片/文件预览）+ 会话列表与项目分组 + 轮次导航 + 输入栏（附件/引用/模型/语音/模式）+ 执行态可视化 + 审批与用户输入弹窗」组装成一条从「用户键入」到「气泡呈现」的完整链路，且对后端执行态零副本（只做谓词派生）。
> 规模：30 源码 + 17 测试 + 3 CSS ≈ 13100 行。前端信息密度最高、与后端耦合最重的区域。

## 结构

| 层 | 文件 | 职责 |
|---|---|---|
| 呈现层 | `ChatPanel.tsx`（3021）/ `MarkdownContent.tsx`（1155）/ `PreviewOverlay.tsx`（1198） | 聊天区总装（消息流/slash 命令/模型 effort 弹窗/refine 弹窗/暂停/点评/lightbox）；自研零依赖 Markdown 渲染（代码高亮、diff 行级着色、本地绝对路径与裸 URL 识别点击、图片/音频附件、引用 chip）；全屏文件预览 + HTML 标注回流（P1~P4） |
| 导航层 | `SessionRail.tsx`（1824）/ `TurnRail.tsx`（311）+ `turnRailModel.ts`（153） | 会话工作台抽屉（按项目文件夹分组、5s 轮询 + 外部变更检测、置顶/归档/排序）；对话轮次刻度轨（滚动位置推断激活轮） |
| 输入层 | `ChatInputBar.tsx`（1283）/ `ReferenceBar.tsx`（250）/ `attachments.ts`（84） | 输入栏（textarea、工具菜单、语音、发送三态、mode/effort 弹窗、ctx 用量、追加队列）；待发送引用/图片/文件 chip；拖拽/粘贴/选择文件统一转换通道 |
| 执行态层 | `ExecIndicator.tsx`（143）/ `TaskBubble.tsx`（182）+ `TaskDetailModal.tsx`（162）/ `PauseOverlay.tsx`（221）/ `ApiHealthBadge.tsx`（259）/ `VideoProgressBadge.tsx` / `ExternalAgentsStatusBar.tsx`（651） | Exec 指示器（idle/running/completed + 60s 退场）；派发台账面板与详情；后端主动暂停决策弹窗；连接健康徽标；外部 Agent 悬浮胶囊 |
| 辅助层 | `WelcomeScreen` / `ParticleField` / `OnboardingModal` / `NewChatModal` / `CreateProjectModal` / `SessionDivider` / `RefineCollapseBar` / `VoiceButton.tsx`（460） | 欢迎页表演、首装引导、会话与项目创建、refine 分段与折叠、语音输入（STT）按钮与状态机 |
| 纯逻辑层 | `sessionGroups.ts`（381）/ `messageSelection.ts` / `annotationPayload.ts` / `modelPopupOrder.ts` / `taskRunFormat.ts` / `inputFocus.ts` | 项目分组纯函数（桌面/移动共用）等，被测试钉死 |

## 功能作用

- 消息渲染管线：`composeAssistantReplies()` 把原始 ChatMessage[] 合并/去重/丢弃空壳后渲染为气泡。
- 输入栏发送三态 + 追加队列（执行中发送=追加指令）；选中文字 → 引用追问（QUOTE_MAX_CHARS=2000）。
- 审批与用户输入弹窗承接后端安全链的 RequireConfirmation 与 request_user_input。

## 规则与坑（两条铁律）

- **执行态单源**：`ExecutionStage = 'idle' | 'running' | 'finalizing'`（hooks/useExecutionState.ts:28）是唯一来源，各组件只做谓词派生（isProcessing / backendLocked），注释明确**禁止**再各自订阅 is_busy / can_switch（ChatPanel.tsx:134）。
- **台账纯投影**：Exec 相关 UI（ExecIndicator / TaskBubble）只读后端 `agent::task_run` 台账的本轮快照，**不做 id 配对、不维护状态机**（TaskBubble.tsx:49-65）。
