import { lazy, Suspense, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { listen } from '../core/bridge'
import type { WorkflowItem } from '../core/types'
import { wfStop, wfPause, wfResume, wfRun, getToolPermissions, openExternal } from './lib/api'
import { handleExternalAnchorClick } from './lib/externalLink'
import { scheduleIdle } from './lib/idle'
import { TenetsDialog } from './dialogs/TenetsDialog'
import { AnnotationsDialog } from './dialogs/AnnotationsDialog'
import { WorkflowRunModal } from './workflow/WorkflowRunModal'
import { CanvasWorkbenchLoading } from './workflow/CanvasWorkbenchLoading'
import { TitleBar } from './layout/TitleBar'
import { ChatPanel } from './chat/ChatPanel'
import AppContextMenu from '../ui/AppContextMenu'
import { AppIsland } from '../ui/AppIsland'
import { useIslandHostAnchor } from '../ui/islandChannel'
import { ExecutionTraceFloating } from './layout/ExecutionTraceFloating'
import { ThinkingIndicator } from './layout/ThinkingIndicator'
// ModalPage replaced by CompactModal
import { CompactModal } from './layout/CompactModal'
import {
  IconPalette,
  IconPlug,
  IconShield,
  IconPuzzle,
  IconSmartphone,
  IconBrowser,
  IconSparkles,
  IconBot,
  IconBrain,
  IconWrench,
  IconHistory,
  IconFile,
  IconWorkflow,
  IconSquare,
  IconCopy,
  IconX,
  IconKeyboard,
  IconGrid,
  IconCpu,
  IconRefresh,
} from '../ui/Icons'
import { CommandPalette } from '../ui/CommandPalette'
import { routePrimaryK } from './shortcutRouting'
import { useLanguage } from '../locales'
import { ErrorBoundary } from '../ui/ErrorBoundary'
import { Button, IconButton } from '../ui/Button'
import { useKeyboard } from '../hooks/useKeyboard'
import { SplashScreen } from '../ui/SplashScreen'
import { ErrorScreen } from '../ui/ErrorScreen'
import { TaskBubble } from './chat/TaskBubble'
import { WorkflowTaskPanel } from './layout/WorkflowTaskPanel'
import { UserInputPrompt } from './layout/UserInputPrompt'
import { RegionPicker } from './tools/RegionPicker'
import { ScreenCaptureTool } from './tools/ScreenCaptureTool'
import { DesktopToolbar } from './tools/DesktopToolbar'
import { MacosPermissionNotice } from './components/MacosPermissionNotice'
import { useSession } from '../hooks/useSession'
import { useEvents } from '../hooks/useEvents'
import { playPopupSound, ensureAudioCtx } from '../ui/sound'
import '../styles/mobile.css'
import '../styles/desktop-toolbar.css'

// ── 按需加载的模态页面（代码分割） ──
const MemoriesPage = lazy(() =>
  import('./memories/MemoriesPage').then(m => ({ default: m.MemoriesPage })),
)
const KnowledgePage = lazy(() =>
  import('./knowledge/KnowledgePage').then(m => ({ default: m.KnowledgePage })),
)
const SkillsPage = lazy(() => import('./pages/SkillsPage').then(m => ({ default: m.SkillsPage })))
const ModelsPage = lazy(() => import('./pages/ModelsPage').then(m => ({ default: m.ModelsPage })))
type ModelsInitialView = 'provider' | 'jev'
const ThemesPage = lazy(() => import('./pages/ThemesPage').then(m => ({ default: m.ThemesPage })))
const SecurityPage = lazy(() =>
  import('./pages/SecurityPage').then(m => ({ default: m.SecurityPage })),
)
const MobilePage = lazy(() => import('./pages/MobilePage').then(m => ({ default: m.MobilePage })))
const BrowserPage = lazy(() =>
  import('./pages/BrowserPage').then(m => ({ default: m.BrowserPage })),
)
const SoulPage = lazy(() => import('./pages/SoulPage').then(m => ({ default: m.SoulPage })))
const CustomAgentsPage = lazy(() =>
  import('./pages/CustomAgentsPage').then(m => ({ default: m.CustomAgentsPage })),
)
const ExternalAgentsPage = lazy(() =>
  import('./pages/ExternalAgentsPage').then(m => ({ default: m.ExternalAgentsPage })),
)
const McpPage = lazy(() => import('./pages/McpPage').then(m => ({ default: m.McpPage })))
const HelpPage = lazy(() => import('./pages/HelpPage').then(m => ({ default: m.HelpPage })))
const PlannerModal = lazy(() =>
  import('./components/PlannerModal').then(m => ({ default: m.PlannerModal })),
)
const PluginRestoreFab = lazy(() =>
  import('./components/PluginRestoreFab').then(m => ({ default: m.PluginRestoreFab })),
)
const ApprovalModal = lazy(() =>
  import('./components/ApprovalModal').then(m => ({ default: m.ApprovalModal })),
)
const WorkflowPage = lazy(() =>
  import('./workflow/WorkflowPage').then(m => ({ default: m.WorkflowPage })),
)
const CanvasWorkbenchPage = lazy(() =>
  import('./workflow/CanvasWorkbenchPage').then(m => ({ default: m.CanvasWorkbenchPage })),
)
// ── 插件市场体系不开源阶段：入口改为 GitHub 贡献者页（GithubPage，原筹备页已下线）。
//    市场 ready 后恢复下面两个 lazy 声明与挂载块即可（可逆）。
// const PluginAppsPage = lazy(() =>
//   import('./pages/PluginAppsPage').then(m => ({ default: m.PluginAppsPage })),
// )
// const PluginDevPage = lazy(() =>
//   import('./pages/PluginDevPage').then(m => ({ default: m.PluginDevPage })),
// )
const GithubPage = lazy(() => import('./pages/GithubPage').then(m => ({ default: m.GithubPage })))
const AppShellPage = lazy(() =>
  import('./pages/AppShellPage').then(m => ({ default: m.AppShellPage })),
)
const SnakeGamePage = lazy(() =>
  import('./pages/SnakeGame/SnakeGame').then(m => ({ default: m.default })),
)
const UpdatePage = lazy(() => import('./pages/UpdatePage').then(m => ({ default: m.UpdatePage })))
const SettingsCenter = lazy(() =>
  import('./pages/SettingsCenter').then(m => ({ default: m.SettingsCenter })),
)

export default function App() {
  // ── Hooks ──
  const { t } = useLanguage()
  const s = useSession()
  /** +号菜单记忆弹窗：'tenets' | 'annotations' | null */
  const [memoryDialog, setMemoryDialog] = useState<'tenets' | 'annotations' | null>(null)
  const [modelsInitialView, setModelsInitialView] = useState<ModelsInitialView>('provider')

  // ── Workflow 权限确认弹窗出现时播放提示音 ──
  useEffect(() => {
    if (s.showWorkflowPermConfirm) playPopupSound('confirm')
  }, [s.showWorkflowPermConfirm])

  // ── 音效预热：首次用户手势（pointerdown/keydown）即创建并恢复 AudioContext，
  //    在用户手势同步栈中完成，state 直接为 running——此后交互音效即时可用，
  //    避免首次点击时才创建、异步 resume 丢音 ──
  useEffect(() => {
    const warm = () => ensureAudioCtx()
    window.addEventListener('pointerdown', warm, { once: true })
    window.addEventListener('keydown', warm, { once: true })
    return () => {
      window.removeEventListener('pointerdown', warm)
      window.removeEventListener('keydown', warm)
    }
  }, [])

  // ── 画布外壳 chunk 空闲预取：只在浏览器空闲时发起请求，绝不在启动同步路径上加载；
  //    首次点开画布时外壳代码已就绪，配合可见 fallback 消除"点了没反应" ──
  useEffect(() => {
    scheduleIdle(() => {
      void import('./workflow/CanvasWorkbenchPage')
    })
  }, [])

  // ── Voice button navigates to /models ──
  useEffect(() => {
    const handler = (event: Event) => {
      const requestedView = (event as CustomEvent<{ view?: string }>).detail?.view
      setModelsInitialView(requestedView === 'jev' ? 'jev' : 'provider')
      s.setShowModels(true)
    }
    window.addEventListener('nuphus-nav-models', handler)
    return () => window.removeEventListener('nuphus-nav-models', handler)
  }, [s.setShowModels])

  // ── 外链接管：WebView 不处理 target="_blank"（点了没反应）→ 交系统浏览器 ──
  // 捕获阶段统一拦截，覆盖聊天消息、设置中心各页、插件页等所有外链。
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      const handled = handleExternalAnchorClick(e.target, url => {
        void openExternal(url).catch(err => {
          console.warn('[external] open failed:', err)
        })
      })
      if (handled) e.preventDefault()
    }
    document.addEventListener('click', onClick, true)
    return () => document.removeEventListener('click', onClick, true)
  }, [])

  const { dismissRefine } = useEvents(s)

  // ── Keyboard shortcuts (Ctrl+K opens cmd palette from s.cmdItems) ──
  const [runWorkflow, setRunWorkflow] = useState<WorkflowItem | null>(null)
  const [wfRunning, setWfRunning] = useState(false)
  // ── 输入栏 workflow 扳手菜单「工作流画布」直达（2026-09-03 大王定稿）：
  //    有未完成（draft）工作流 → 续编最近草稿；否则新建空白工作流并进入画布。
  //    闸门铁律同 WorkflowPage：任意执行态禁止进入画布（点击级 gate 复核）。──
  const handleWorkflowCanvasDirect = async () => {
    s.setShowCanvas(true)
  }
  /** 输入栏 workflow 扳手菜单「工作流列表」：等同 Ctrl+K → 工作流（WorkflowPage 弹窗） */
  const handleOpenWorkflowList = () => s.setShowWorkflow(true)
  // ── 应用插件全屏宿主（App Plugin 体系；打开即关闭列表弹窗，仿画布模式）──
  const [runningPluginId, setRunningPluginId] = useState<string | null>(null)
  // ── 宿主最小化态：true 时 AppShellPage 保持挂载但 visibility 隐藏（iframe 保活），
  //    主窗口输入框 dock 左侧悬浮 PluginRestoreFab 点击恢复 ──
  const [pluginMinimized, setPluginMinimized] = useState(false)
  // ── Desktop toolbar (Ctrl+U) ──
  const [showDesktopToolbar, setShowDesktopToolbar] = useState(false)
  // ── 设置中心全屏覆盖层（输入栏最左端齿轮按钮 → 左导航 + 右内容）──
  const [showSettingsCenter, setShowSettingsCenter] = useState(false)
  // island 落点锚点：模型页这类全屏宿主（fixed inset:0 + z 2500）会盖住聊天区，
  // 岛在宿主打开时改挂到宿主标题栏（优先级见 ui/islandChannel.ts 的「落点锚点」）
  const modelsIslandAnchor = useIslandHostAnchor('models-page')
  const [scheduleReplay, setScheduleReplay] = useState<{
    workflowId: string
    runId: string
  } | null>(null)
  const cmdIconMap: Record<string, React.ReactNode> = {
    workflows: <IconWorkflow size={14} />,
    canvas: <IconPalette size={14} />,
    memories: <IconHistory size={14} />,
    skills: <IconWrench size={14} />,
    knowledge: <IconFile size={14} />,
    mcp: <IconPlug size={14} />,
    plugins: <IconPuzzle size={14} />,
    models: <IconBrain size={14} />,
    themes: <IconPalette size={14} />,
    security: <IconShield size={14} />,
    mobile: <IconSmartphone size={14} />,
    browser: <IconBrowser size={14} />,
    soul: <IconSparkles size={14} />,
    'snake-game': <IconGrid size={14} />,
    'force-reset': <IconX size={14} />,
    help: <IconKeyboard size={14} />,
    'external-agents': <IconCpu size={14} />,
    'check-update': <IconRefresh size={14} />,
  }
  /**
   * 聊天界面快捷键的可用性闸门：设置中心面板打开时不响应。
   * 这些快捷键作用于面板「背后」的聊天界面（Ctrl+N 新建会话 / Ctrl+U 桌面工具栏 /
   * Ctrl+L 聚焦输入框 / Ctrl+Shift+W 工作流面板），焦点此时已被焦点陷阱收进面板，
   * 继续生效会让用户在弹窗里的操作意外改动背后会话。Ctrl+K 例外 —— 它的语义是
   * 「先退出面板，再执行原分支」，见下。
   */
  const chatShortcutEnabled = () => !showSettingsCenter
  useKeyboard([
    {
      key: 'k',
      ctrl: true,
      handler: () => {
        // 设置中心宿主（z-index 2500）会盖住 Ctrl+K 两条分支的目标弹层
        //（命令面板 300 / 工作流列表 150）→ 先退出设置中心，保证面板可见可用
        setShowSettingsCenter(false)
        routePrimaryK(
          s.mode,
          () => {
            s.setCmdPaletteOpen(false)
            s.setShowWorkflow(true)
          },
          () => s.setCmdPaletteOpen((p: boolean) => !p),
        )
      },
    },
    {
      key: 'l',
      ctrl: true,
      enabled: chatShortcutEnabled,
      handler: () => s.setFocusSignal((p: number) => p + 1),
    },
    { key: 'n', ctrl: true, enabled: chatShortcutEnabled, handler: () => s.handleNewChat() },
    {
      key: 'o',
      ctrl: true,
      enabled: chatShortcutEnabled,
      handler: () => {
        /* TODO: 插件搜索 */
      },
    },
    {
      key: 'u',
      ctrl: true,
      enabled: chatShortcutEnabled,
      handler: () => setShowDesktopToolbar((p: boolean) => !p),
    },
    {
      // 工作流步骤面板：收起 / 展开。
      // 面板可见性 ≡ workflowRunSteps 非空 且 未被用户收起，所以这里只在有运行数据时响应。
      key: 'w',
      ctrl: true,
      shift: true,
      enabled: chatShortcutEnabled,
      handler: () => {
        if (s.workflowRunSteps.length === 0) return
        if (s.workflowPanelDismissed) s.showWorkflowPanel()
        else s.dismissWorkflowPanel()
      },
    },
  ])

  // ── 手机端「新会话」遥控跟随：POST /new-chat 已完成**后端转场**（归档 → 当前槽置
  // None → 清 backup → SessionChanged 双推），NewChatBroadcast 只通知桌面**本地清 view**：
  // reloadChatFromBackend = resetTransientUI + 重拉历史（后端空态 → 空列表欢迎页）。
  // 绝不在此重复调 new_chat_session_cmd——会二次转场并多广播一次 SessionChanged。
  // 本机新建（Ctrl+N / TitleBar / SessionRail+ / 命令面板）走 s.handleNewChat（后端转场）。
  // ref 镜像避免 once-registered 监听捕获陈旧闭包。
  const reloadFromBackendRef = useRef(s.reloadChatFromBackend)
  reloadFromBackendRef.current = s.reloadChatFromBackend
  useEffect(() => {
    let unlisten: (() => void) | undefined
    void listen<{ seq: number; event: { type: string } }>('nuphus-event', ({ event }) => {
      if (event.type === 'new_chat_broadcast') void reloadFromBackendRef.current()
    }).then(u => {
      unlisten = u
    })
    return () => unlisten?.()
  }, [])

  // ── 实际启动工作流（权限与模式均已确认后调用） ──
  // inputs：运行确认弹窗收集的声明式外部输入；工作流未声明 inputs 时保持 undefined（既有链路不变）
  const executeWorkflowRun = async (id: string, inputs?: Record<string, unknown>) => {
    const needSwitch = s.mode !== 'workflow'
    setRunWorkflow(null)
    s.setShowWorkflow(false)

    // 先切模式再启动：执行中后端**拒绝**切模式（执行态判定，见 set_mode_impl）——
    // 此时不阻断也不谎报，消息改由「追加指令」通道送达当前任务（真实处置见下方回执分支）。
    let switched = true
    if (needSwitch) {
      try {
        await s.handleSetMode('workflow')
      } catch {
        switched = false
      }
    }
    if (switched) {
      s.showToast(needSwitch ? '切换到 Workflow 模式执行工作流' : '启动工作流', 'info')
    }

    // 声明了外部输入 → 走确定性 wf_run（与画布同一执行入口），值直接透传后端；
    // Agent 的 workflow_run 工具链不接受结构化输入，且敏感值不应进入会话文本。
    if (inputs !== undefined) {
      try {
        await wfRun(id, false, inputs)
      } catch (e) {
        s.showToast(`启动失败：${String(e)}`, 'error')
      }
      return
    }

    // 发送用户消息给 WorkflowAgent，由其调用 workflow_run 工具启动工作流。
    // ⚠️ 回执必须消费（S1）：执行中发送会被受理为**追加指令**（工作流不会立即启动），
    // 收尾期则被拒收退回——丢弃回执会让这两条路径都静默，用户以为工作流已经在跑。
    // appended 路径的提示由 appendedHint 经既有 showToast 通道给出（不重复弹）。
    const outcome = await s.handleSend(
      `启动工作流 ${id}`,
      undefined,
      'workflow',
      undefined,
      undefined,
      t('workflow.runAppended'),
    )
    if (outcome.rejected) {
      // 收尾期拒收：消息**未被受理**（未入队、内容不会自动重发）→ 明确告知稍后重试。
      // 不复用桌面输入框那句「内容已退回输入框」——这条路径没有可退回的输入框，
      // 用户点的是「运行工作流」，如实说明「尚未启动」即可（S1/S5 同一条纪律）。
      s.showToast(t('workflow.runFinalizingRetry'), 'warning')
    } else if (!outcome.ok) {
      s.showToast(outcome.message || t('workflow.runFailed'), 'error')
    }
  }

  return (
    <div className="app-shell">
      {/* 自绘右键复制菜单（拦截浏览器原生导航菜单，防误点刷新/检查中断运行） */}
      <AppContextMenu />
      {/* 页头 island：应用在前台时的轻反馈（非前台仍走 HUD，见 ui/islandChannel.ts）。
          常驻挂载：队列不丢提示，挂载即开始跟踪窗口焦点 */}
      <AppIsland />
      {/* ── Splash Screen: loading / fade-out state ── */}
      {(s.appState === 'loading' || s.fadeOut) && (
        <SplashScreen items={s.initItems} fadeOut={s.fadeOut} />
      )}

      {/* ── Error Screen: init failure ── */}
      {s.appState === 'error' && s.initError && (
        <ErrorScreen
          error={s.initError as any}
          onRetry={s.runInitialization}
          onOpenSettings={() => {
            s.setAppState('ready' as any)
            s.setShowModels(true)
          }}
          onExit={() => window.close()}
        />
      )}

      {/* ── Main UI: ready state ── */}
      {s.appState === 'ready' && (
        <ErrorBoundary>
          <TitleBar onNewChat={s.handleNewChat} agentState={s.isProcessing ? 'working' : 'idle'} />
          <MacosPermissionNotice mode={s.mode} onOpenSettings={() => s.setShowSecurity(true)} />
          <div className="chat-area">
            {/*
              ⛔ 禁止把「外部 Agent 列表栏」等 UI 反馈写进 s.messages（addMessage）：
              执行期「messages 最后一条 = 正在流式的 agent 气泡」是强约定
              （useEvents execution_started 追加 live 气泡 → 后续 delta 按 id 写它；
              ChatPanel `isCurrentAgent = idx === messages.length - 1`；
              useSession 的 last 判定）。中途插入任何消息都会把回答拦腰截断、
              令流式光标与 last 判定失真——实测会让用户看到 agent 输出被隔断。
              反馈一律走轻提示通道（s.showToast → island/HUD 分流），与「已保存」「已中断」同通道。
            */}
            <ChatPanel
              messages={s.messages}
              // 唯一执行态（后端 ExecutionStage + 事件推送）：ChatPanel 内部派生
              // 「主循环在迭代中」与「后端仍占用」两个谓词，不再各自订阅不同来源
              executionStage={s.executionStage}
              onSend={(input, images, references, sendId) =>
                s.handleSend(input, images, undefined, references, sendId)
              }
              startupStats={s.startupStats}
              onGracefulStop={s.handleGracefulStop}
              onInterrupt={s.handleInterrupt}
              onRetry={s.handleRetryAgent}
              focusSignal={s.focusSignal}
              onNewChat={s.handleNewChat}
              onChatReplaced={() => void s.reloadChatFromBackend()}
              onModeSwitched={m => s.setMode(m)}
              onResumeLast={() => void s.resumeLastSession()}
              onOpenPrinciples={() => setMemoryDialog('tenets')}
              onOpenAnnotations={() => setMemoryDialog('annotations')}
              tokenUsage={s.displayTokenUsage}
              mood={s.mood}
              goalType={s.goalType}
              security={s.security}
              pauseState={s.pauseState}
              onContinue={s.handleContinue}
              onAppendInstruction={s.handleAppendInstruction}
              appendQueue={s.appendQueue}
              onTerminate={s.handleTerminate}
              onApproveSecurity={() => s.setSecurity(null)}
              onRejectSecurity={() => s.setSecurity(null)}
              modelName={s.modelName}
              mainTokenUsage={s.mainTokenUsage}
              execTokenUsage={s.execTokenUsage}
              totalDurationMs={s.totalDurationMs}
              totalCalls={s.liveCalls}
              contextLimit={s.contextLimit}
              apiHealth={s.apiHealth}
              onModelChanged={s.refreshModelInfo}
              mode={s.mode}
              onSetMode={s.handleSetMode}
              onManageCustomAgents={() => s.setShowCustomAgents(true)}
              onManageExternalAgents={() => s.setShowExternalAgents(true)}
              onExternalAgentNotice={text => s.showToast(text, 'info')}
              onToggleWorkAgentMode={s.toggleWorkAgentMode}
              refineState={s.refineState}
              pendingRefine={s.pendingRefine}
              setPendingRefine={s.setPendingRefine}
              onRefine={s.handleRefine}
              onSkipRefine={s.handleSkipRefine}
              refining={s.refining}
              setRefining={s.setRefining}
              onDismissRefine={() => {
                // 复位提炼 UI + 追踪 refs（后台提炼不中断）；toast 明示后台仍在跑
                dismissRefine()
                s.showToast(t('refine.dismissHint'), 'info')
              }}
              isWorkflowRunning={s.workflowRunSteps.length > 0}
              showDesktopToolbar={showDesktopToolbar}
              onToggleDesktopToolbar={() => setShowDesktopToolbar(o => !o)}
              onOpenWorkflowCanvas={() => void handleWorkflowCanvasDirect()}
              onOpenWorkflowList={handleOpenWorkflowList}
              onOpenSettings={() => setShowSettingsCenter(true)}
              onRate={s.handleRate}
              onShowExecTrace={trace => {
                s.setExecTraceOverride(trace)
                s.setShowExecTrace(true)
              }}
              onCommand={id => {
                switch (id) {
                  case 'workflows':
                    s.setShowWorkflow(true)
                    break
                  case 'canvas':
                    s.setShowCanvas(true)
                    break
                  case 'memories':
                    s.setShowMemories(true)
                    break
                  case 'skills':
                    s.setShowSkills(true)
                    break
                  case 'knowledge':
                    s.setShowKnowledge(true)
                    break
                  case 'mcp':
                    s.setShowMcp(true)
                    break
                  case 'plugins':
                    s.setShowPlugins(true)
                    break
                  case 'models':
                    s.setShowModels(true)
                    break
                  case 'themes':
                    s.setShowThemes(true)
                    break
                  case 'security':
                    s.setShowSecurity(true)
                    break
                  case 'mobile':
                    s.setShowMobile(true)
                    break
                  case 'browser':
                    s.setShowBrowser(true)
                    break
                  case 'soul':
                    s.setShowSoul(true)
                    break
                  case 'help':
                    s.setShowHelp(true)
                    break
                  case 'new-chat':
                    s.handleNewChat()
                    break
                  case 'force-reset':
                    s.forceReset()
                    break
                  case 'snake-game':
                    s.setShowSnakeGame(true)
                    break
                  case 'external-agents':
                    s.setShowExternalAgents(true)
                    break
                }
              }}
            />
            <ThinkingIndicator
              key={s.executionCounter}
              step={s.dismissThinking ? '' : s.thinkingStep}
              // 思考条呼吸与执行态同源（running）；收尾/空闲不再显示「执行中」
              isThinking={s.executionStage === 'running'}
              completed={s.completed}
              dismissed={s.dismissThinking}
              phase={s.execPhase}
              timeline={s.timeline}
              mood={s.mood}
              progress={s.progress}
              onExpand={() => s.setShowExecTrace(true)}
              onClose={() => s.setDismissThinking(true)}
            />
          </div>

          <ExecutionTraceFloating
            timeline={s.timeline}
            traceOverride={s.execTraceOverride}
            stepIndex={s.stepIndex}
            progress={s.progress}
            isProcessing={s.isProcessing}
            completed={s.completed}
            expandedCalls={s.expandedCalls}
            onToggleExpand={s.toggleExpand}
            goal={s.goal}
            totalDurationMs={s.totalDurationMs}
            totalCalls={s.totalCalls}
            visible={s.showExecTrace}
            onClose={() => {
              s.setExecTraceOverride(null)
              s.setShowExecTrace(false)
            }}
            onRate={s.handleRate}
            mode={s.mode}
          />

          <WorkflowTaskPanel
            visible={s.workflowRunSteps.length > 0 && !s.workflowPanelDismissed}
            steps={s.workflowRunSteps}
            workflowId={s.lastWorkflowId}
            isPaused={s.isWorkflowPaused}
            onTerminate={() => {
              // 旧实现 `wfStop(id)` 既没 await 也没 catch：IPC 一旦失败就是 unhandled
              // rejection，用户侧表现是"点了终止毫无反应"。把失败原因弹出来。
              const fail = (e: unknown) =>
                s.showToast(`终止失败：${e instanceof Error ? e.message : String(e)}`, 'error')
              if (s.workflowRunId) {
                wfStop(s.workflowRunId).catch(fail)
              } else {
                // 没有 run id 时退化为中断当前执行
                // （interrupt 现已同时取消活动工作流，见 process/lifecycle.rs）
                s.handleInterrupt().catch(fail)
              }
            }}
            onPause={() => s.handleWfPause()}
            onResume={() => s.handleWfResume()}
            onClose={() => s.dismissWorkflowPanel()}
            onReRun={() => {
              const wid = s.lastWorkflowId
              if (wid) executeWorkflowRun(wid)
            }}
            onForceReset={() => {
              s.forceReset()
            }}
          />

          {/* 面板被收起但仍有运行数据时的恢复入口——同时也是暂停/终止的找回入口。
              旧实现把面板 ✕ 直接接到 setWorkflowRunSteps([])，一旦误关就既丢数据、
              又没有任何入口能把它找回来（面板可见性只由该数组驱动，且只由运行事件写入）。 */}
          {s.workflowRunSteps.length > 0 && s.workflowPanelDismissed && (
            <button
              className="wfst-restore-pill"
              onClick={() => s.showWorkflowPanel()}
              title="展开工作流步骤面板 (Ctrl+Shift+W)"
            >
              <IconWorkflow size={13} />
              <span>工作流 · {s.workflowRunSteps.length} 步</span>
              {s.isWorkflowPaused && <span className="wfst-restore-paused">已暂停</span>}
            </button>
          )}

          {/* ── Command Palette（Ctrl+K） ── */}
          <CommandPalette
            open={s.cmdPaletteOpen}
            onClose={() => s.setCmdPaletteOpen(false)}
            items={s.cmdItems}
            iconMap={cmdIconMap}
          />

          {/* ── Planner Modal（按需加载） ── */}
          <Suspense fallback={null}>
            <PlannerModal
              open={s.showPlannerModal}
              plan={s.planData}
              onClose={() => s.setShowPlannerModal(false)}
            />
          </Suspense>

          {/* ── Approval Modal（按需加载） ── */}
          <Suspense fallback={null}>
            <ApprovalModal
              open={s.approvalState.open}
              kind={s.approvalState.kind}
              title={s.approvalState.title}
              content={s.approvalState.content}
              actionId={s.approvalState.actionId}
              tenetCount={s.approvalState.tenetCount}
              onClose={() => s.setApprovalState((prev: any) => ({ ...prev, open: false }))}
            />
          </Suspense>

          {/* ── Workflow Permission Confirmation ── */}
          {s.showWorkflowPermConfirm &&
            createPortal(
              <div className="cmd-modal-overlay" onClick={s.handleWorkflowPermCancel}>
                <div
                  className="cmd-modal cmd-modal-sm"
                  onClick={e => e.stopPropagation()}
                  style={{ maxWidth: 420 }}
                >
                  <div className="cmd-modal-header">
                    <span className="cmd-modal-icon">🔐</span>
                    <span className="cmd-modal-title">安全权限确认</span>
                    <IconButton
                      variant="modal-close"
                      label="关闭"
                      onClick={s.handleWorkflowPermCancel}
                    >
                      <IconX size={14} />
                    </IconButton>
                  </div>
                  <div className="cmd-modal-body">
                    <p
                      style={{
                        fontSize: 13,
                        color: 'var(--spark-muted)',
                        lineHeight: 1.6,
                        margin: '0 0 16px',
                      }}
                    >
                      Workflow
                      模式需要全部安全权限才能正常运行（文件读写、网络搜索、系统自动化）。是否同意开启全部权限？
                    </p>
                    <div style={{ display: 'flex', gap: 8 }}>
                      <Button
                        variant="ghost"
                        onClick={s.handleWorkflowPermCancel}
                        style={{ flex: 1 }}
                      >
                        取消
                      </Button>
                      <Button
                        variant="primary"
                        onClick={s.handleWorkflowPermConfirm}
                        style={{ flex: 1 }}
                      >
                        同意开启
                      </Button>
                    </div>
                  </div>
                </div>
              </div>,
              document.body,
            )}

          {/* ── User Input Prompt ── */}
          {s.userInputRequest && (
            <UserInputPrompt
              title={s.userInputRequest.title}
              prompt={s.userInputRequest.prompt}
              sensitive={s.userInputRequest.sensitive}
              actionId={s.userInputRequest.actionId}
              inputType={s.userInputRequest.inputType || 'text'}
              iconPath={s.userInputRequest.iconPath}
              defaultName={s.userInputRequest.defaultName}
              defaultShortcut={s.userInputRequest.defaultShortcut}
              relX={s.userInputRequest.relX}
              relY={s.userInputRequest.relY}
              defaultNote={s.userInputRequest.defaultNote}
              defaultStage={s.userInputRequest.defaultStage}
              onSubmit={() => s.setUserInputRequest(null)}
              onReject={() => s.setUserInputRequest(null)}
            />
          )}

          {/* ── Task Bubble ── */}
          <TaskBubble
            visible={s.taskBubbleVisible}
            tasks={s.planData?.tasks || []}
            onClose={() => s.setTaskBubbleVisible(false)}
          />

          {/* ── 工具栏触发的选区/截图覆盖层 ── */}
          {s.regionPickerMode === 'capture' && (
            <ScreenCaptureTool
              onClose={() => s.setRegionPickerMode(null)}
              onCaptured={(result: any) => {
                s.setRegionPickerMode(null)
              }}
            />
          )}
          {s.regionPickerMode === 'picker' && (
            <RegionPicker
              mode="picker"
              onClose={() => s.setRegionPickerMode(null)}
              onConfirm={(region: any) => {
                s.setRegionPickerMode(null)
              }}
            />
          )}

          {/* ── Compact Command Modals（按需加载） ── */}
          <CompactModal
            open={s.showMemories}
            onClose={() => s.setShowMemories(false)}
            title={t('app.memories')}
            icon={<IconHistory size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <MemoriesPage />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showKnowledge}
            onClose={() => s.setShowKnowledge(false)}
            title={t('app.knowledge')}
            icon={<IconFile size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <KnowledgePage onClose={() => s.setShowKnowledge(false)} />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showSkills}
            onClose={() => s.setShowSkills(false)}
            title={t('app.skills')}
            icon={<IconWrench size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <SkillsPage />
            </Suspense>
          </CompactModal>
          {/* ── 模型管理：全屏整页（模型页整页化样板；左侧 X = 关页 + 刷新模型信息；X 左置避免与右上窗口控制重叠误关） ── */}
          {s.showModels && (
            <Suspense fallback={null}>
              <div className="models-page-host">
                <div className="models-page-bar">
                  <IconButton
                    variant="modal-close"
                    label="关闭"
                    className="models-page-close"
                    onClick={() => {
                      s.setShowModels(false)
                      setModelsInitialView('provider')
                      s.refreshModelInfo()
                    }}
                  >
                    <IconX size={14} />
                  </IconButton>
                  <span className="models-page-title">{t('app.models')}</span>
                  {/* 拖动条：模型页是全屏覆盖层，会盖住 TitleBar 的 data-tauri-drag-region，
                      导致停留在该页面时窗口无法拖动。这里补一条占满剩余空白的拖动区。 */}
                  <span className="models-page-drag" data-tauri-drag-region />
                  {/* island 落点锚点：宿主（fixed inset:0 + z 2500）会盖住聊天区，
                      岛必须挂进宿主标题栏才可见（几何见 styles/app-pill.css） */}
                  <div className="island-slot" ref={modelsIslandAnchor} />
                </div>
                <div className="models-page-body">
                  <ModelsPage
                    initialView={modelsInitialView}
                    onClose={() => {
                      s.setShowModels(false)
                      setModelsInitialView('provider')
                      s.refreshModelInfo()
                    }}
                    onModelChanged={() => s.refreshModelInfo()}
                  />
                </div>
              </div>
            </Suspense>
          )}
          <CompactModal
            open={s.showThemes}
            onClose={() => s.setShowThemes(false)}
            title={t('app.themes')}
            icon={<IconPalette size={14} />}
            size="auto"
          >
            <Suspense fallback={null}>
              <ThemesPage onClose={() => s.setShowThemes(false)} showToast={s.showToast} />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showUpdate}
            onClose={() => s.setShowUpdate(false)}
            title={t('app.update')}
            icon={<IconRefresh size={14} />}
            size="sm"
          >
            <Suspense fallback={null}>
              <UpdatePage />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showSecurity}
            onClose={() => s.setShowSecurity(false)}
            title={t('app.security')}
            icon={<IconShield size={14} />}
            size="sm"
          >
            <Suspense fallback={null}>
              <SecurityPage onClose={() => s.setShowSecurity(false)} />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showMobile}
            onClose={() => s.setShowMobile(false)}
            title={t('app.mobile')}
            icon={<IconSmartphone size={14} />}
            size="sm"
          >
            <Suspense fallback={null}>
              <MobilePage />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showBrowser}
            onClose={() => s.setShowBrowser(false)}
            title={t('app.browser')}
            icon={<IconBrowser size={14} />}
            size="sm"
          >
            <Suspense fallback={null}>
              <BrowserPage onClose={() => s.setShowBrowser(false)} />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showSoul}
            onClose={() => s.setShowSoul(false)}
            title={t('app.soul')}
            icon={<IconSparkles size={14} />}
            size="auto"
          >
            <Suspense fallback={null}>
              <SoulPage onClose={() => s.setShowSoul(false)} />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showCustomAgents}
            onClose={() => s.setShowCustomAgents(false)}
            title={t('custom.page.title')}
            icon={<IconBot size={14} />}
            size="xl"
          >
            <Suspense fallback={null}>
              <CustomAgentsPage
                onClose={() => s.setShowCustomAgents(false)}
                onActivated={() => {
                  // 激活即进入：切到 Custom 模式 + 关闭配置页（链路闭合）。
                  // 执行中后端会拒绝切模式（执行态判定）→ 把原因提示出来，
                  // 不让拒绝变成未捕获的 promise rejection（静默）。
                  void s.handleSetMode('custom').catch(e => s.showToast(String(e), 'error'))
                  s.setShowCustomAgents(false)
                }}
              />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showExternalAgents}
            onClose={() => s.setShowExternalAgents(false)}
            title={t('extAgents.cfg.title')}
            icon={<IconCpu size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <ExternalAgentsPage onClose={() => s.setShowExternalAgents(false)} />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showMcp}
            onClose={() => s.setShowMcp(false)}
            title={t('cmd.mcp')}
            icon={<IconPlug size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <McpPage onClose={() => s.setShowMcp(false)} />
            </Suspense>
          </CompactModal>
          {/* ── GitHub：社区贡献者页（原付费插件市场筹备页；市场 ready 后恢复 PluginAppsPage 全屏面板）── */}
          <CompactModal
            open={s.showPlugins}
            onClose={() => s.setShowPlugins(false)}
            title={t('app.github')}
            icon={<IconPuzzle size={14} />}
            size="md"
          >
            <Suspense fallback={null}>
              <GithubPage />
            </Suspense>
          </CompactModal>
          {/* ── 开发者中心挂载已随市场体系一并注释（App.tsx lazy 区可逆说明）── */}
          <CompactModal
            open={s.showHelp}
            onClose={() => s.setShowHelp(false)}
            title={t('app.help')}
            icon={<IconKeyboard size={14} />}
            size="md"
          >
            <Suspense fallback={null}>
              <HelpPage />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showSnakeGame}
            onClose={() => s.setShowSnakeGame(false)}
            title={t('cmd.snakeGame')}
            icon={<IconGrid size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <SnakeGamePage />
            </Suspense>
          </CompactModal>
          <CompactModal
            open={s.showWorkflow}
            onClose={() => s.setShowWorkflow(false)}
            title={t('app.workflows')}
            icon={<IconWorkflow size={14} />}
            size="lg"
          >
            <Suspense fallback={null}>
              <WorkflowPage
                onClose={() => s.setShowWorkflow(false)}
                onRunClick={wf => setRunWorkflow(wf)}
                onCanvasClick={wf => s.openCanvas(wf.id)}
              />
            </Suspense>
          </CompactModal>
          {s.showCanvas && (
            <Suspense fallback={<CanvasWorkbenchLoading />}>
              <div className="canvas-workbench-host">
                <CanvasWorkbenchPage
                  workflowId={s.canvasWorkflowId}
                  replayRunId={scheduleReplay?.runId ?? null}
                  onExitReplay={() => setScheduleReplay(null)}
                  onClose={() => {
                    setScheduleReplay(null)
                    s.closeCanvas()
                  }}
                />
              </div>
            </Suspense>
          )}
          {/* ── 设置中心：居中弹窗（左导航 + 右内容；子页一律复用 pages/* 现有实现）── */}
          {showSettingsCenter && (
            <Suspense fallback={null}>
              <SettingsCenter
                onClose={() => setShowSettingsCenter(false)}
                showToast={s.showToast}
                onRunWorkflow={wf => {
                  // 运行确认弹窗（wcf-wrapper z-index 100）低于设置中心宿主（2500）→
                  // 先退出设置中心再弹，与「Ctrl+K → 工作流 → 运行」原链路表现一致
                  setShowSettingsCenter(false)
                  setRunWorkflow(wf)
                }}
                /* 宿主分流：画布 / 模型两类分区在弹窗内容区（上限 844px）内结构性不可用，
                   点击后关闭面板，改走各自既有全屏宿主链路（与 Ctrl+K 入口同一实现）。 */
                onOpenCanvas={workflowId => {
                  setShowSettingsCenter(false)
                  setScheduleReplay(null)
                  s.openCanvas(workflowId ?? null)
                }}
                onOpenScheduleReplay={(workflowId, runId) => {
                  setShowSettingsCenter(false)
                  setScheduleReplay({ workflowId, runId })
                  s.openCanvas(workflowId)
                }}
                onOpenModels={() => {
                  setShowSettingsCenter(false)
                  s.setShowModels(true)
                }}
              />
            </Suspense>
          )}
          {/* ── 应用插件宿主：全屏覆盖层（App Plugin 体系 §4.2）── */}
          {runningPluginId && (
            <Suspense fallback={null}>
              <AppShellPage
                pluginId={runningPluginId}
                minimized={pluginMinimized}
                onMinimize={() => setPluginMinimized(true)}
                onClose={() => {
                  // 关闭宿主 = 返回插件主界面（子级返父级，对齐开发者中心 onClose 语义）
                  setRunningPluginId(null)
                  setPluginMinimized(false)
                  s.setShowPlugins(true)
                }}
                showToast={s.showToast}
              />
            </Suspense>
          )}
          {/* ── 最小化宿主的悬浮恢复按钮：输入框 dock 左侧，点击恢复宿主可见 ── */}
          {runningPluginId && pluginMinimized && (
            <Suspense fallback={null}>
              <PluginRestoreFab
                pluginId={runningPluginId}
                onRestore={() => setPluginMinimized(false)}
              />
            </Suspense>
          )}
          <WorkflowRunModal
            open={runWorkflow !== null}
            workflow={runWorkflow}
            running={wfRunning}
            onRun={async (id, inputs) => {
              // 检查安全权限
              try {
                const perms = await getToolPermissions()
                let parsed: {
                  file_access: boolean
                  web_search: boolean
                  system_automation: boolean
                } | null = null
                if (perms && typeof perms === 'string') {
                  try {
                    parsed = JSON.parse(perms)
                  } catch {}
                }
                const allGranted =
                  parsed?.file_access && parsed?.web_search && parsed?.system_automation
                if (!allGranted) {
                  s.setShowSecurity(true)
                  setRunWorkflow(null)
                  return
                }
              } catch {
                s.showToast('无法检查安全权限，请确认权限已开启', 'warning')
                setRunWorkflow(null)
                return
              }

              // 非 Workflow 模式 → 直接切换并执行（双槽位架构，不丢 Leader 上下文）
              await executeWorkflowRun(id, inputs)
            }}
            onCancel={() => setRunWorkflow(null)}
          />
        </ErrorBoundary>
      )}

      {/* ── +号菜单记忆弹窗：教导原则 / 关系标注 ── */}
      {memoryDialog === 'tenets' && <TenetsDialog onClose={() => setMemoryDialog(null)} />}
      {memoryDialog === 'annotations' && (
        <AnnotationsDialog onClose={() => setMemoryDialog(null)} />
      )}

      {/* ── Desktop toolbar (Ctrl+U) ── */}
      <DesktopToolbar visible={showDesktopToolbar} onClose={() => setShowDesktopToolbar(false)} />
    </div>
  )
}
