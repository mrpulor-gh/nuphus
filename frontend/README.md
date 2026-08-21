# Nuphus 前端（React + TypeScript）

> **路径**: `frontend/` | **框架**: React 18 + TypeScript | **构建**: Vite 6

## 技术栈

- **React 18** + react-dom
- **@tauri-apps/api** v2 — Tauri IPC 通信
- **lucide-react** — 图标库
- **Vite** v6 — 构建工具
- **TypeScript** — 类型安全

> 注：项目含 TailwindCSS 依赖但实际未启用（src/styles 无 @tailwind 引用），样式系统为纯 CSS 文件（`src/styles/` + 各模块 CSS，共 48 个文件），基于 CSS 变量设计令牌。

## 源码结构

```
src/
├── main.tsx              # React 入口
├── core/                 # 核心层
│   ├── bridge.ts         # Tauri IPC 封装 + Dev Mock 层（WebSocket 回退）
│   ├── types.ts          # 前端类型定义（ChatMessage/TimelineEntry/NuphusEvent 等）
│   └── types-memory.ts   # 记忆相关类型
├── main-window/          # 主窗口
│   ├── App.tsx           # **主应用** — 状态管理 + NuphusEvent 事件路由 + 初始化流程
│   ├── chat/             # 聊天面板
│   │   ├── ChatPanel.tsx      # 主聊天面板（消息列表/轮换提示语/上下文压缩）
│   │   ├── ChatInputBar.tsx   # 输入框（自适应高度/斜杠命令弹窗/附件拖拽）
│   │   ├── MarkdownContent.tsx # Markdown 渲染（工具调用/代码块/Agent 卡片）
│   │   ├── PauseOverlay.tsx   # 暂停覆盖层（继续/追加/终止控制）
│   │   ├── TaskBubble.tsx     # 任务气泡（子任务状态展示）
│   │   ├── SessionDivider.tsx # 会话分隔线
│   │   └── WelcomeScreen.tsx  # 欢迎屏幕（快捷入口/模式选择）
│   ├── layout/           # 布局组件
│   │   ├── TitleBar.tsx               # 自定义标题栏（拖拽/图标/控制按钮）
│   │   ├── StatusBar.tsx              # 底部状态栏（记忆计数/模型/权限）
│   │   ├── ExecutionTraceFloating.tsx # 执行面板（卡片+终端双模式/点评/流式输出）
│   │   ├── SecurityPrompt.tsx         # 安全确认弹窗
│   │   ├── UserInputPrompt.tsx        # 用户输入弹窗（审批/确认/补充信息）
│   │   ├── ThinkingIndicator.tsx      # 思考状态指示器
│   │   ├── WorkflowTaskPanel.tsx      # 工作流任务进度面板
│   │   └── CompactModal.tsx           # 紧凑模态框
│   ├── pages/            # 功能页面
│   │   ├── ModelsPage.tsx    # 模型配置（Provider/模型/参数/AuxiliaryTask）
│   │   ├── ProjectPage.tsx   # 项目目录管理
│   │   ├── SecurityPage.tsx  # 安全设置（权限模式/审批策略）
│   │   ├── SkillsPage.tsx    # 技能面板（安装/搜索/详情）
│   │   ├── SoulPage.tsx      # 身份关系配置
│   │   └── ThemesPage.tsx    # 主题定制
│   ├── components/       # 组件
│   │   ├── PlannerModal.tsx    # 任务规划模态框
│   │   └── ReviewPanel.tsx     # 计划评审面板（评分+评语）
│   ├── tools/            # 工具面板（CommandPalette 入口）
│   ├── knowledge/        # 知识图谱面板
│   ├── memories/         # 记忆面板
│   ├── workflow/         # 工作流面板
│   └── lib/              # 工具库
│       ├── api.ts        # Tauri 命令封装
│       └── api-memory.ts # 记忆相关 API
├── ui/                   # 通用 UI 组件
│   ├── CommandPalette.tsx   # 命令面板（Ctrl+U 唤出/滚轮/斜杠命令弹窗交互）
│   ├── Button.tsx           # 按钮组件
│   ├── Icons.tsx            # 图标组件
│   ├── ErrorBoundary.tsx    # 错误边界
│   ├── ErrorScreen.tsx      # 错误屏幕
│   ├── SplashScreen.tsx     # 启动闪屏
│   ├── MoodFace.tsx         # 情绪表情
│   ├── NuphusLogo.tsx       # Logo 组件
│   └── Toast.tsx            # 消息提示
├── hooks/                # 自定义 React Hooks
│   ├── useSession.ts     # 会话状态管理（消息/执行/暂停/模式切换）
│   ├── useEvents.ts      # NuphusEvent 事件订阅与处理
│   ├── useKeyboard.ts    # 全局快捷键管理
│   ├── useApi.ts         # API 调用封装
│   └── useTheme.tsx      # 主题上下文
├── styles/               # 样式系统（14 CSS 文件）
│   ├── tokens.css        # CSS 变量设计令牌（颜色/字体/间距/动画）
│   ├── chat.css          # 聊天 + 执行面板样式
│   ├── components.css    # 组件统一样式
│   ├── sidebar.css       # 侧栏样式
│   ├── planner.css       # 规划模态框样式
│   └── ...               # memories/splash/toast/session-divider 等
├── locales/              # 国际化
├── capture-overlay/      # 截图覆盖层（区域选择/取色/模板匹配）
└── mobile/               # 移动端适配
```

## UI 设计系统

- **主题**: 暗色玻璃态设计（`data-theme="dark"`），CSS 变量驱动
- **字体**: 系统 UI `-apple-system, BlinkMacSystemFont, Segoe UI, sans-serif`
- **等宽字体**: `Monaspace Radon, Monaco, Menlo, Consolas, monospace`
- **动画**: 呼吸动画（`inputBreathing`）/ 入场动画（`traceEnter`）/ 脉冲（`dotPulse`）

## 快捷键

| 快捷键 | 功能 |
|--------|------|
| `Ctrl+K` | 唤出命令面板（CommandPalette） |
| `Ctrl+U` | 桌面工具栏 |
| `Ctrl+L` | 聚焦输入框 |
| `Ctrl+N` | 新建对话 |
| `Enter` | 发送消息 |
| `Shift+Enter` | 换行 |

## 关键依赖

```json
{
  "@tauri-apps/api": "^2.10.1",
  "@tauri-apps/plugin-dialog": "^2.7.1",
  "lucide-react": "^1.14.0",
  "react": "^18.3.1"
}
```