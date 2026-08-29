import { lazy, Suspense, useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { invoke } from '../core/bridge'
import type { WorkflowItem } from '../core/types'
import { wfStop, wfPause, wfResume, getToolPermissions } from './lib/api'
import { TenetsDialog } from './dialogs/TenetsDialog'
import { AnnotationsDialog } from './dialogs/AnnotationsDialog'
import { WorkflowRunModal } from './workflow/WorkflowRunModal'
import { TitleBar } from './layout/TitleBar'
import { ChatPanel } from './chat/ChatPanel'
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
  IconFolder,
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
  IconMessageCircle,
  IconCpu,
} from '../ui/Icons'
import { CommandPalette } from '../ui/CommandPalette'
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
import { useSession } from '../hooks/useSession'
import { useEvents } from '../hooks/useEvents'
import { playPopupSound } from '../ui/sound'
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
const ThemesPage = lazy(() => import('./pages/ThemesPage').then(m => ({ default: m.ThemesPage })))
const ProjectPage = lazy(() =>
  import('./pages/ProjectPage').then(m => ({ default: m.ProjectPage })),
)
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
const CanvasPage = lazy(() =>
  import('./workflow-canvas/CanvasPage').then(m => ({ default: m.CanvasPage })),
)
// ── 插件市场体系不开源阶段：入口仅展示筹备提示（PluginComingSoon）。
//    市场 ready 后恢复下面两个 lazy 声明与挂载块即可（可逆）。
// const PluginAppsPage = lazy(() =>
//   import('./pages/PluginAppsPage').then(m => ({ default: m.PluginAppsPage })),
// )
// const PluginDevPage = lazy(() =>
//   import('./pages/PluginDevPage').then(m => ({ default: m.PluginDevPage })),
// )
const PluginComingSoon = lazy(() =>
  import('./pages/PluginComingSoon').then(m => ({ default: m.PluginComingSoon })),
)
const AppShellPage = lazy(() =>
  import('./pages/AppShellPage').then(m => ({ default: m.AppShellPage })),
)
const SnakeGamePage = lazy(() =>
  import('./pages/SnakeGame/SnakeGame').then(m => ({ default: m.default })),
)

export default function App() {
  // ── Hooks ──
  const { t } = useLanguage()
  const s = useSession()
  /** +号菜单记忆弹窗：'tenets' | 'annotations' | null */
  const [memoryDialog, setMemoryDialog] = useState<'tenets' | 'annotations' | null>(null)

  // ── Workflow 权限确认弹窗出现时播放提示音 ──
  useEffect(() => {
    if (s.showWorkflowPermConfirm) playPopupSound('confirm')
  }, [s.showWorkflowPermConfirm])

  // ── Voice button navigates to /models ──
  useEffect(() => {
    const handler = () => s.setShowModels(true)
    window.addEventListener('nuphus-nav-models', handler)
    return () => window.removeEventListener('nuphus-nav-models', handler)
  }, [s.setShowModels])

  const { dismissRefine } = useEvents(s)

  // ── Keyboard shortcuts (Ctrl+K opens cmd palette from s.cmdItems) ──
  const [runWorkflow, setRunWorkflow] = useState<WorkflowItem | null>(null)
  const [wfRunning, setWfRunning] = useState(false)
  // ── 工作流节点画布（Pro 位；全屏覆盖层，无路由——主窗口不用 react-router）──
  const [canvasWorkflowId, setCanvasWorkflowId] = useState<string | null>(null)
  // ── 应用插件全屏宿主（App Plugin 体系；打开即关闭列表弹窗，仿画布模式）──
  const [runningPluginId, setRunningPluginId] = useState<string | null>(null)
  // ── 宿主最小化态：true 时 AppShellPage 保持挂载但 visibility 隐藏（iframe 保活），
  //    主窗口输入框 dock 左侧悬浮 PluginRestoreFab 点击恢复 ──
  const [pluginMinimized, setPluginMinimized] = useState(false)
  // ── Desktop toolbar (Ctrl+U) ──
  const [showDesktopToolbar, setShowDesktopToolbar] = useState(false)
  const cmdIconMap: Record<string, React.ReactNode> = {
    workflows: <IconWorkflow size={14} />,
    memories: <IconHistory size={14} />,
    skills: <IconWrench size={14} />,
    knowledge: <IconFile size={14} />,
    mcp: <IconPlug size={14} />,
    plugins: <IconPuzzle size={14} />,
    models: <IconBrain size={14} />,
    themes: <IconPalette size={14} />,
    project: <IconFolder size={14} />,
    security: <IconShield size={14} />,
    mobile: <IconSmartphone size={14} />,
    browser: <IconBrowser size={14} />,
    soul: <IconSparkles size={14} />,
    'snake-game': <IconGrid size={14} />,
    'new-chat': <IconMessageCircle size={14} />,
    'force-reset': <IconX size={14} />,
    help: <IconKeyboard size={14} />,
    'external-agents': <IconCpu size={14} />,
  }
  useKeyboard([
    { key: 'k', ctrl: true, handler: () => s.setCmdPaletteOpen((p: boolean) => !p) },
    { key: 'l', ctrl: true, handler: () => s.setFocusSignal((p: number) => p + 1) },
    { key: 'n', ctrl: true, handler: () => s.handleNewChat() },
    {
      key: 'o',
      ctrl: true,
      handler: () => {
        /* TODO: 插件搜索 */
      },
    },
    { key: 'u', ctrl: true, handler: () => setShowDesktopToolbar((p: boolean) => !p) },
  ])

  // ── 实际启动工作流（权限与模式均已确认后调用） ──
  const executeWorkflowRun = async (id: string) => {
    const needSwitch = s.mode !== 'workflow'
    invoke('hud_update', {
      text: needSwitch ? '切换到 Workflow 模式执行工作流' : '启动工作流',
      phase: 'info',
    })
    setRunWorkflow(null)
    s.setShowWorkflow(false)

    if (needSwitch) {
      await s.handleSetMode('workflow')
    }

    // 发送用户消息给 WorkflowAgent，由其调用 workflow_run 工具启动工作流
    await s.handleSend(`启动工作流 ${id}`, undefined, 'workflow')
  }

  return (
    <div className="app-shell">
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
          <div className="chat-area">
            <ChatPanel
              messages={s.messages}
              isProcessing={s.isProcessing}
              onSend={(input, images, references) =>
                s.handleSend(input, images, undefined, references)
              }
              startupStats={s.startupStats}
              onGracefulStop={s.handleGracefulStop}
              onInterrupt={s.handleInterrupt}
              onRetry={s.handleRetryAgent}
              focusSignal={s.focusSignal}
              onNewChat={s.handleNewChat}
              onChatReplaced={() => void s.reloadChatFromBackend()}
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
              onTerminate={s.handleTerminate}
              onApproveSecurity={() => s.setSecurity(null)}
              onRejectSecurity={() => s.setSecurity(null)}
              modelName={s.modelName}
              mainTokenUsage={s.mainTokenUsage}
              execTokenUsage={s.execTokenUsage}
              totalDurationMs={s.totalDurationMs}
              totalCalls={s.liveCalls}
              contextLimit={s.contextLimit}
              onModelChanged={s.refreshModelInfo}
              mode={s.mode}
              onSetMode={s.handleSetMode}
              onManageCustomAgents={() => s.setShowCustomAgents(true)}
              onManageExternalAgents={() => s.setShowExternalAgents(true)}
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
              onRate={s.handleRate}
              onShowExecTrace={trace => {
                s.setExecTraceOverride(trace)
                s.setShowExecTrace(true)
              }}
              onOpenPalette={() => s.setCmdPaletteOpen(true)}
              onCommand={id => {
                switch (id) {
                  case 'workflows':
                    s.setShowWorkflow(true)
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
                  case 'project':
                    s.setShowProject(true)
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
              isThinking={s.isProcessing}
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
            visible={s.workflowRunSteps.length > 0}
            steps={s.workflowRunSteps}
            workflowId={s.lastWorkflowId}
            isPaused={s.isWorkflowPaused}
            onTerminate={() => {
              if (s.workflowRunId) wfStop(s.workflowRunId)
              else s.handleInterrupt()
            }}
            onPause={() => s.handleWfPause()}
            onResume={() => s.handleWfResume()}
            onClose={() => s.setWorkflowRunSteps([])}
            onReRun={() => {
              const wid = s.lastWorkflowId
              if (wid) executeWorkflowRun(wid)
            }}
            onForceReset={() => {
              s.forceReset()
            }}
          />

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
          <CompactModal
            open={s.showModels}
            onClose={() => {
              s.setShowModels(false)
              s.refreshModelInfo()
            }}
            title={t('app.models')}
            icon={<IconBrain size={14} />}
            size="md"
          >
            <Suspense fallback={null}>
              <ModelsPage
                onClose={() => s.setShowModels(false)}
                onModelChanged={() => s.refreshModelInfo()}
              />
            </Suspense>
          </CompactModal>
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
            open={s.showProject}
            onClose={() => s.setShowProject(false)}
            title={t('app.project')}
            icon={<IconFolder size={14} />}
            size="auto"
          >
            <Suspense fallback={null}>
              <ProjectPage onClose={() => s.setShowProject(false)} />
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
                  // 激活即进入：切到 Custom 模式 + 关闭配置页（链路闭合）
                  s.handleSetMode('custom')
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
          {/* ── 插件市场：筹备提示弹窗（市场体系不开源阶段；市场 ready 后恢复 PluginAppsPage 全屏面板）── */}
          <CompactModal
            open={s.showPlugins}
            onClose={() => s.setShowPlugins(false)}
            title={t('plugins.listTitle')}
            icon={<IconPuzzle size={14} />}
            size="md"
          >
            <Suspense fallback={null}>
              <PluginComingSoon />
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
                onCanvasClick={wf => {
                  // 画布是全屏目的地：关闭列表弹窗，避免双层堆叠
                  s.setShowWorkflow(false)
                  setCanvasWorkflowId(wf.id)
                }}
              />
            </Suspense>
          </CompactModal>
          {/* ── 工作流节点画布：全屏覆盖层 ── */}
          {canvasWorkflowId && (
            <Suspense fallback={null}>
              <CanvasPage workflowId={canvasWorkflowId} onClose={() => setCanvasWorkflowId(null)} />
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
            onRun={async id => {
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
                invoke('hud_update', {
                  text: '无法检查安全权限，请确认权限已开启',
                  phase: 'warning',
                })
                setRunWorkflow(null)
                return
              }

              // 非 Workflow 模式 → 直接切换并执行（双槽位架构，不丢 Leader 上下文）
              await executeWorkflowRun(id)
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
