import React, {
  useState,
  useRef,
  useEffect,
  useLayoutEffect,
  useCallback,
  useMemo,
  type ReactNode,
} from 'react'
import { createPortal } from 'react-dom'
import type {
  ChatMessage,
  ChatReference,
  PendingImage,
  PendingFile,
  SendOutcome,
  TimelineEntry,
} from '../../core/types'
import type { SecurityCheck } from '../../core/types'
import { composeAssistantReplies } from '../../core/progressMessages'
import type { ExecutionStage } from '../../hooks/useExecutionState'
import {
  refineForceMinPct,
  setRefineForceThreshold,
  type RefineState,
} from '../../hooks/useExecutionUI'
import { useStickyScroll } from '../../hooks/useStickyScroll'
import { TurnRail } from './TurnRail'
import { isCustomProviderId } from '../lib/customProvider'
import { setIslandAnchor } from '../../ui/islandChannel'
import { toAssetUrl, resolveLocalImageUrl } from '../../ui/assetUrl'
import {
  getCurrentConfig,
  configureLlm,
  switchModel,
  getSupportedProviders,
  getContextLimit,
  getToolPermissions,
  hudUpdate,
  isLlmConfigured,
  getReasoningEffort,
  setReasoningEffort,
  listModels,
  getEffectiveModel,
  getProviderContext,
  setRelation as persistRelationToBackend,
  getProjectDir,
  getProjectBookmarks,
  setProjectDir as setProjectDirCmd,
  setProjectBookmarks as setProjectBookmarksCmd,
  TOOL_PERMISSIONS_CHANGED_EVENT,
  browserShowWindow,
  browserOpenUrl,
} from '../lib/api'
import type { ProviderInfo, ModelInfo, ProjectBookmark, ToolPermissions } from '../lib/api'
import { friendlyIpcError } from '../lib/ipcError'
import { orderProviderModels, readRecentModels, rememberRecentModel } from './modelPopupOrder'
import { getProviderFavorites } from '../lib/favorites'
import { buildQuoteRef } from './messageSelection'
import { filesToAttachments } from './attachments'
import { focusInputAtEnd } from './inputFocus'
import { RefIcon } from './ReferenceBar'
import { WelcomeScreen } from './WelcomeScreen'
import { OnboardingModal } from './OnboardingModal'
import { SessionDivider } from './SessionDivider'
import { RefineCollapseBar } from './RefineCollapseBar'
import type { ApiHealthState } from '../../core/types'
import { PauseOverlay } from './PauseOverlay'
import { ChatInputBar } from './ChatInputBar'
import { VideoProgressBadge } from './VideoProgressBadge'
import ExternalAgentsStatusBar from './ExternalAgentsStatusBar'
import SessionRail from './SessionRail'
import { loadRelation } from '../lib/relation'
import { ProviderIcon, hasProviderIcon } from '../components/ProviderIcon'
import {
  IconCopy,
  IconCheck,
  IconX,
  IconWorkflow,
  IconHistory,
  IconWrench,
  IconFile,
  IconBrain,
  IconPalette,
  IconShield,
  IconBrowser,
  IconSparkles,
  IconSquare,
  IconGrid,
  IconEye,
  IconMic,
  IconImage,
  IconRadio,
  IconSettings,
  IconStar,
  IconChartColumn,
  IconChevronsDown,
} from '../../ui/Icons'
import { RatingModal } from '../layout/ExecutionTraceFloating'
import { MoodFace } from '../../ui/MoodFace'
import { useLanguage } from '../../locales'
import { TurnMetaBar, type TurnMetaBarActiveTool } from '../../ui/TurnMetaBar'
import type { TurnMeta } from '../../core/types'
import { LetterAvatar } from '../../ui/LetterAvatar'
import { playUiSound } from '../../ui/sound'
import { useWheelSelection } from '../../ui/wheelSelection'
import { formatPrimaryShortcut } from '../../ui/platformShortcut'
import { modelSetupHint } from '../shortcutRouting'
import type { MoodState } from '../../ui/MoodFace'
import '../../styles/chat.css'
import { StatusBar } from '../layout/StatusBar'
import { SecurityPrompt } from '../layout/SecurityPrompt'
import { Button, IconButton } from '../../ui/Button'
import MarkdownContent from './MarkdownContent'
import { PreviewOverlay } from './PreviewOverlay'
import { AppearancePanel } from '../layout/AppearancePanel'
function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

interface ChatPanelProps {
  messages: ChatMessage[]
  /**
   * 实时执行时间线（useSession.timeline）：仅用于派生「当前正在跑的工具」，
   * 驱动气泡执行态提示（工具结束——成功或失败——立即消失）。
   * 缺省空数组，非执行场景与测试无须提供。
   */
  timeline?: TimelineEntry[]
  /**
   * 后端权威执行态（唯一来源，见 useExecutionState）。本组件内部只做谓词派生：
   * `running` = agent 主循环在迭代中（气泡光标 / 思考条）；`!== 'idle'` = 后端仍占用
   * （会话 rail 锁 / mode 锁 / 终止按钮）。禁止再各自订阅 is_busy / can_switch。
   */
  executionStage: ExecutionStage
  /** 返回发送的真实结果；sendId 为调用方指定的发送标识：后端受理事件按它精确对齐，
   *  缺省时由 useSession 生成（老调用方行为不变）。 */
  onSend: (
    input: string,
    images?: string[],
    references?: ChatReference[],
    sendId?: string,
  ) => Promise<SendOutcome>
  startupStats: { tools: number; memories: number }
  onGracefulStop?: () => void
  onInterrupt?: () => void
  onRetry?: (input: string, messageId?: string) => void
  focusSignal?: number
  onNewChat?: (title?: string) => Promise<boolean>
  /** Session Rail 切换/新建成功后：重拉 get_chat_history 替换气泡 */
  onChatReplaced?: () => void
  /** Session Rail 跨 mode 切换成功后：同步前端 mode state（mode chip 一致性） */
  onModeSwitched?: (mode: string) => void
  /** 欢迎页「继续对话」：先调后端 resume_latest_session 装镜像再渲染完整历史 */
  onResumeLast?: () => void
  /** +号菜单：打开教导原则弹窗 */
  onOpenPrinciples?: () => void
  /** +号菜单：打开关系标注弹窗 */
  onOpenAnnotations?: () => void
  tokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  goalType?: { type: string; label: string; confidence: number } | null
  security?: SecurityCheck | null
  pauseState?: { actionId: string } | null
  onContinue?: (actionId: string) => void
  onAppendInstruction?: (actionId: string, instruction: string) => void
  appendQueue?: string[]
  onTerminate?: (actionId: string) => void
  onApproveSecurity?: (id: string) => void
  onRejectSecurity?: (id: string) => void
  mood?: MoodState
  modelName?: string
  mainTokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  execTokenUsage?: {
    inputTokens: number
    outputTokens: number
    cacheHitTokens: number
    genTps?: number
    ttftMs?: number
  } | null
  totalDurationMs?: number
  totalCalls?: number
  /** 本轮元数据（耗时 / token / 步数）——最后一条 assistant 气泡实时元数据条用；
   *  历史消息各用自己的 msg.meta，不走这个 prop */
  turnMeta?: TurnMeta | null
  contextLimit?: number
  apiHealth?: ApiHealthState
  onApiHealthRead?: () => void
  onModelChanged?: () => void
  /** 贴底跟随的 followReset 下行 ref：useEvents 跑在 App 层（本组件之上），其
   *  execution_started / execution_completed 分支需要「恢复跟随 + 滚底」—— 函数不能
   *  经 props 上行，改为父层下发 ref、本组件每次渲染回填 useStickyScroll 的
   *  followReset 最新闭包（useEvents 经 h.refs 同读此 ref）。缺省（未注入）时
   *  useEvents 侧静默跳过，回底按钮与冻结/宽限语义不受影响。 */
  followResetRef?: React.MutableRefObject<(() => void) | null>
  refineState?: RefineState | null
  pendingRefine: (RefineState & { skippedTurns: number }) | null
  setPendingRefine: React.Dispatch<
    React.SetStateAction<(RefineState & { skippedTurns: number }) | null>
  >
  onRefine?: () => void
  onSkipRefine?: () => void
  /** 提炼执行中（全局）：驱动提炼中全屏遮罩（弹窗路径与 refine-pending-btn 路径统一） */
  refining?: boolean
  setRefining?: (v: boolean) => void
  /** 大窗口强制线滑块的本地草稿（session 层持有，模型切换时由 useEvents 清空） */
  forceDraft?: number | null
  setForceDraft?: (v: number | null) => void
  /** 手动关闭「提炼中」弹窗/遮罩：复位提炼 UI + 追踪 refs（后台提炼不中断，
   *  完成后 session_refined / refine_failed 照常落地）。缺省退化为仅收起遮罩 */
  onDismissRefine?: () => void
  onCommand?: (id: string) => void
  mode?: string
  onSetMode?: (mode: string) => void
  onManageCustomAgents?: () => void
  onManageExternalAgents?: () => void
  /** 外部 Agent 从列表栏移出时的用户面反馈（App 注入 `showToast` → HUD 轻提示）。
   *  ⛔ 不得走会话消息：见 App.tsx chat-area 处的时序约定说明 */
  onExternalAgentNotice?: (text: string) => void
  onToggleWorkAgentMode?: () => Promise<void>
  onRate?: (
    name: string,
    rating: number,
    comment: string,
    saveAsStrategy: boolean,
    userQuestion?: string,
    assistantContent?: string,
  ) => void
  /** 气泡执行回溯：点击打开执行面板显示该轮历史执行过程（traceItems） */
  onShowExecTrace?: (trace: TimelineEntry[]) => void
  /**
   * 聊天区左上角（`.chat-header-left`）的自绘 chrome 槽位：**由 App 层组合**。
   * 现在挂的是「Exec 执行指示器」（ExecIndicator）——它的数据源是本轮 task_runs
   * 台账、点击目标是 App 层既有的 showExecTrace 开关，两者都已活在 App 层；
   * 走槽位可避免把执行台账再穿一层 props 进本组件（本组件对执行细节零感知）。
   * 缺省不渲染（无槽位则左组保持空）。
   */
  execIndicator?: ReactNode
  /** Leader 暂停是否禁用（workflow 运行时） */
  isWorkflowRunning?: boolean
  /** 输入栏最左端齿轮按钮：打开设置中心全屏覆盖层（状态由 App 层持有） */
  onOpenSettings?: () => void
  /**
   * 外观浮窗的外部打开请求（App 层 `useModals.showThemes`：Ctrl+K 命令面板
   * 「外观」置位）。置 true 即展开外观浮窗；本组件不持有这个全局态。
   */
  appearanceOpen?: boolean
  /**
   * 外观浮窗收起时回写外部请求。必须有：同值 setState 不触发 effect，
   * 不回写则「Esc 收起 → 再点 Ctrl+K → 外观」不再是一次 true 跳变，入口失效。
   */
  onAppearanceDismiss?: () => void
}

/**
 * 旧版本把项目书签存在 localStorage 的两套键里（`nuphus_project_bookmarks` 用
 * `{name,path}`、`nuphus_projects` 用 `{label,path}`，互不相通）→ 一次性合并进
 * 后端单一事实源并清理旧键。返回 `null` 表示无需迁移。
 */
function migrateLegacyProjectBookmarks(existing: ProjectBookmark[]): ProjectBookmark[] | null {
  const merged = [...existing]
  let changed = false
  for (const key of ['nuphus_project_bookmarks', 'nuphus_projects']) {
    let raw: string | null = null
    try {
      raw = localStorage.getItem(key)
    } catch {
      continue
    }
    if (raw !== null) {
      try {
        const arr = JSON.parse(raw)
        if (Array.isArray(arr)) {
          for (const item of arr) {
            const path = String(item?.path || '').trim()
            if (!path || merged.some(b => b.path === path)) continue
            const name =
              String(item?.name || item?.label || '').trim() ||
              path
                .replace(/[\\/]+$/, '')
                .split(/[\\/]/)
                .pop() ||
              path
            merged.push({ name, path })
          }
        }
      } catch {
        /* 数据损坏：内容忽略，但键仍清理，避免每次打开都重试 */
      }
      try {
        localStorage.removeItem(key)
        changed = true
      } catch {
        /* localStorage 不可用：跳过 */
      }
    }
  }
  return changed ? merged : null
}

export function ChatPanel({
  messages: messageRecords,
  timeline = [],
  executionStage,
  onSend,
  onGracefulStop,
  onInterrupt,
  onRetry,
  focusSignal,
  onNewChat,
  onChatReplaced,
  onModeSwitched,
  onResumeLast,
  onOpenPrinciples,
  onOpenAnnotations,
  tokenUsage,
  goalType,
  security,
  pauseState,
  onContinue,
  onAppendInstruction,
  appendQueue,
  onTerminate,
  onApproveSecurity,
  onRejectSecurity,
  mood,
  modelName,
  mainTokenUsage,
  execTokenUsage,
  totalDurationMs,
  totalCalls,
  turnMeta,
  contextLimit,
  apiHealth,
  onApiHealthRead,
  onModelChanged,
  followResetRef,
  refineState,
  pendingRefine,
  forceDraft,
  setForceDraft,
  setPendingRefine,
  onRefine,
  onSkipRefine,
  refining,
  setRefining,
  onDismissRefine,
  onCommand,
  mode,
  onSetMode,
  onManageCustomAgents,
  onManageExternalAgents,
  onExternalAgentNotice,
  onToggleWorkAgentMode,
  startupStats,
  isWorkflowRunning,
  onOpenSettings,
  appearanceOpen,
  onAppearanceDismiss,
  onRate,
  execIndicator,
  onShowExecTrace,
}: ChatPanelProps) {
  const messages = useMemo(() => composeAssistantReplies(messageRecords), [messageRecords])
  const { t } = useLanguage()
  // ── 执行态谓词（同一来源 executionStage 的两个派生，禁止再引入第二个来源）──
  // isProcessing：主循环在迭代中 —— 气泡光标 / 思考条呼吸 / 「发送=追加」提示。
  // backendLocked：后端仍占用（Running ∨ Finalizing）—— 会话 rail 锁 / mode 锁 / 终止按钮。
  // 二者都取自 executionStage 同一个值，不再出现 `!canSwitch || locked` 式多源 OR。
  const isProcessing = executionStage === 'running'
  const backendLocked = executionStage !== 'idle'

  // 当前正在执行的工具（大王 2026-10-22 定）：气泡执行态提示只跟随**这一条**——
  // 工具结束（成功或失败）立即消失，不等整轮结束。取 timeline 最后一条 running 的
  // tool_call；非执行态 / 无工具时为 null（失败/中断若丢了 tool_call_end，
  // isProcessing 转 false 同样清空）。
  const activeTool = useMemo<TurnMetaBarActiveTool | null>(() => {
    if (!isProcessing) return null
    for (let i = timeline.length - 1; i >= 0; i--) {
      const entry = timeline[i]
      if (entry.kind === 'tool_call' && entry.status === 'running') {
        return { id: entry.id, toolName: entry.toolName, params: entry.params }
      }
    }
    return null
  }, [isProcessing, timeline])
  const [pauseMode, setPauseMode] = useState<'menu' | 'preparing' | 'input'>('menu')
  const [appendInput, setAppendInput] = useState('')
  const [pauseSubmitting, setPauseSubmitting] = useState(false)
  const [selectedOption, setSelectedOption] = useState(0)
  const [pausing, setPausing] = useState(false)
  const [showPauseLocal, setShowPauseLocal] = useState(false)

  // ── 文件预览覆盖层（AI 回复路径点击） ──
  const [previewPath, setPreviewPath] = useState<string | null>(null)

  // ── 远程链接点击 → 统一进 Agent 浏览器（CDP），不再走系统浏览器 ──
  // 2026-10-09 大王定调：点外链的常见动机是「Agent 让我登录某个网站」，登录态必须落在
  // 工作流/Agent 用的那个 profile 里；在系统浏览器登录，工作流照样过不去。后端在新标签
  // 打开（不导航当前页），因此不会把 Agent 正在自动化的页面顶掉。
  const openUrl = useCallback((url: string) => {
    browserOpenUrl(url).catch(e => {
      hudUpdate(friendlyIpcError(e, '打开链接失败'), 'warning')
    })
  }, [])

  /**
   * header 浏览器按钮：把 Agent 浏览器（Nuphus 的 CDP 实例）提到前台。
   *
   * 它与系统浏览器是两件事：外链走 open_external 给人看，这个窗口是 Agent
   * 自动化真正干活的地方（独立 profile）——想看它进行到哪一步时置前。
   *
   * 冷启动要拉起整个 Chrome 进程（不是切个标签页），所以点击后**立刻**回一条提示，
   * 让「点了到底有没有用」当场可见；按钮自身进入 pending 直到后端返回。pending 期间
   * 再次点击直接丢弃：后端浏览器是单槽串行的，连点只会把等待排成一队，越点越慢。
   */
  const [browserOpening, setBrowserOpening] = useState(false)
  const browserOpeningRef = useRef(false)
  const showBrowserWindow = useCallback(() => {
    if (browserOpeningRef.current) return
    browserOpeningRef.current = true
    setBrowserOpening(true)
    hudUpdate(t('browser.starting'), 'info')
    // 默认页 = Nuphus 官网：只在冷启动/空白页时被后端采纳（就地导航，不新开标签）；
    // Agent 执行中不传——执行里浏览器页就是工作页，唤出绝不导航，防止顶掉正在
    // 自动化的页面。窗口已有真实页面时后端同样忽略此参数。
    const show = isProcessing ? browserShowWindow() : browserShowWindow('https://www.nuphus.com/')
    show
      .catch(e => {
        hudUpdate(friendlyIpcError(e, '打开浏览器失败'), 'warning')
      })
      .finally(() => {
        browserOpeningRef.current = false
        setBrowserOpening(false)
      })
  }, [t, isProcessing])

  // 执行中重渲染节奏（2026-10-19 大王定调）：1000ms 顶层 tick——消息流里的动态
  // 内容（执行步骤计数等）保持可见的连续感。此前为查卡死曾整树删除，实测「半天
  // 才动一次」体验不可接受；卡死根因经审计另有其因（渲染饱和误判），节奏不是病根。
  const [, forceTick] = useState(0)
  useEffect(() => {
    if (!isProcessing) return
    const timer = window.setInterval(() => forceTick(n => n + 1), 1000)
    return () => window.clearInterval(timer)
  }, [isProcessing])
  /**
   * 自定义头像的**可渲染 URL**（用户侧 / 智能体侧）。
   *
   * 持久化存的是本机文件路径，而路径→URL 是异步的（`resolveLocalImageUrl` 可能
   * 要经 Rust 读文件；dev server 下 asset:// 通道根本不可用，见 assetUrl.ts），
   * 因此在这里预解析一次存 state，供消息渲染热路径同步取用。
   * 依赖是两个路径字符串：用户在设置页换头像后值变化 → 自动重新解析。
   */
  const [avatarUrls, setAvatarUrls] = useState<{ user: string | null; nuphus: string | null }>({
    user: null,
    nuphus: null,
  })

  const [pauseActionBusy, setPauseActionBusy] = useState(false)

  /**
   * 自定义头像的持久化路径（顶层读一次每次渲染都读）。
   *
   * localStorage 不是响应式的：把它作为 effect 依赖，「用户在设置页换头像」
   * 体现为这两个字符串变化 → effect 重跑 → 重新解析 URL。比自建版本号可靠。
   */
  const userAvatarPath = localStorage.getItem('nuphus_user_avatar') || ''
  const nuphusAvatarPath = localStorage.getItem('nuphus_nuphus_avatar') || ''

  /**
   * 预解析自定义头像：本地路径 → 可渲染 URL。
   *
   * 路径→URL 是异步的（`resolveLocalImageUrl` 可能要经 Rust 读文件；dev server 下
   * asset:// 通道根本不可用，见 assetUrl.ts），所以不能在消息渲染的热路径里同步取。
   * 解析失败（文件被删等）返回 null → 回落到字母头像，而不是留一个破图。
   */
  useEffect(() => {
    let alive = true
    void Promise.all([
      resolveLocalImageUrl(userAvatarPath),
      resolveLocalImageUrl(nuphusAvatarPath),
    ]).then(([user, nuphus]) => {
      if (alive) setAvatarUrls({ user, nuphus })
    })
    return () => {
      alive = false
    }
  }, [userAvatarPath, nuphusAvatarPath])

  // ── 点评弹窗 ──
  const [ratingMsg, setRatingMsg] = useState<{
    id: string
    content: string
    userQuestion?: string
  } | null>(null)
  // Pending image data URLs — attached to next user message
  const [pendingImages, setPendingImages] = useState<PendingImage[]>([])
  // Lightbox for image click-to-zoom
  const [lightboxUrl, setLightboxUrl] = useState<string | null>(null)
  // Pending file paths (drag-drop) — shown in ReferenceBar, attached to next user message
  const [pendingFiles, setPendingFiles] = useState<PendingFile[]>([])
  // Pending resource references (skill/knowledge/workflow) — attached to next user message
  const [pendingReferences, setPendingReferences] = useState<ChatReference[]>([])

  // Load tool permissions for WORKFLOW mode check
  const [toolPermissions, setToolPermissions] = useState<ToolPermissions | undefined>()
  useEffect(() => {
    let active = true
    const refresh = () => {
      void getToolPermissions()
        .then(r => {
          if (active && r && typeof r === 'string') {
            try {
              const data = JSON.parse(r)
              setToolPermissions({
                file_access: data.file_access ?? data.fileAccess ?? true,
                web_search: data.web_search ?? data.webSearch ?? true,
                system_automation: data.system_automation ?? data.systemAutomation ?? false,
              })
            } catch {
              /* ignore */
            }
          }
        })
        .catch(() => {})
    }
    const handleChanged = (event: Event) => {
      const permissions = (event as CustomEvent<ToolPermissions>).detail
      if (permissions) setToolPermissions(permissions)
    }
    const handleVisibility = () => {
      if (document.visibilityState === 'visible') refresh()
    }

    refresh()
    window.addEventListener(TOOL_PERMISSIONS_CHANGED_EVENT, handleChanged)
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', handleVisibility)
    return () => {
      active = false
      window.removeEventListener(TOOL_PERMISSIONS_CHANGED_EVENT, handleChanged)
      window.removeEventListener('focus', refresh)
      document.removeEventListener('visibilitychange', handleVisibility)
    }
  }, [])

  const handlePauseChoice = useCallback(
    async (choice: string) => {
      if (pauseActionBusy) return

      // 本地暂停对话框（Case 1）已移除——执行控制入口不再触发本地暂停菜单；
      // 仅保留后端主动暂停（workflow/系统）的决策处理。
      if (showPauseLocal) {
        setPauseActionBusy(true)
        try {
          switch (choice) {
            case 'continue':
              setShowPauseLocal(false)
              break
            case 'append':
              setPauseMode('preparing')
              break
            case 'terminate':
              setShowPauseLocal(false)
              onGracefulStop?.()
              break
            case 'interrupt':
              setShowPauseLocal(false)
              onInterrupt?.()
              break
          }
        } finally {
          setPauseActionBusy(false)
        }
        return
      }

      // Case 2: Backend-initiated pause
      if (!pauseState) return
      setPauseActionBusy(true)
      try {
        switch (choice) {
          case 'continue':
            await onContinue?.(pauseState.actionId)
            break
          case 'append':
            setPauseMode('preparing')
            break
          case 'terminate':
            await onTerminate?.(pauseState.actionId)
            break
          case 'interrupt':
            onInterrupt?.()
            break
        }
      } finally {
        setPauseActionBusy(false)
      }
    },
    [
      pauseState,
      showPauseLocal,
      pauseActionBusy,
      onContinue,
      onTerminate,
      onInterrupt,
      onGracefulStop,
    ],
  )

  const handleSubmitAppend = useCallback(async () => {
    if (!pauseState || !appendInput.trim() || pauseSubmitting) return
    setPauseSubmitting(true)
    try {
      await onAppendInstruction?.(pauseState.actionId, appendInput)
      setShowPauseLocal(false)
      setPauseMode('menu')
      setAppendInput('')
    } finally {
      setPauseSubmitting(false)
    }
  }, [pauseState, appendInput, pauseSubmitting, onAppendInstruction])

  // Keyboard navigation for pause menu
  useEffect(() => {
    if (!pauseState && !showPauseLocal) return
    const handler = (e: KeyboardEvent) => {
      if (pauseMode === 'menu' || pauseMode === 'preparing') {
        if (e.key === 'ArrowDown') {
          e.preventDefault()
          setSelectedOption(prev => Math.min(prev + 1, 3))
        } else if (e.key === 'ArrowUp') {
          e.preventDefault()
          setSelectedOption(prev => Math.max(prev - 1, 0))
        } else if (e.key === 'Enter') {
          e.preventDefault()
          const choices = ['continue', 'append', 'terminate', 'interrupt']
          handlePauseChoice(choices[selectedOption])
        } else if (e.key === 'Escape') {
          e.preventDefault()
          if (pauseMode === 'preparing') {
            setPauseMode('menu')
          } else if (showPauseLocal) {
            setShowPauseLocal(false)
          }
        }
      } else if (pauseMode === 'input') {
        if (e.key === 'Escape') {
          e.preventDefault()
          setPauseMode('menu')
        }
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [pauseState, showPauseLocal, pauseMode, selectedOption, handlePauseChoice])

  // Reset pause modal state when pauseState changes (but not if appending)
  useEffect(() => {
    if (pauseState) {
      if (pauseMode === 'preparing' || pauseMode === 'input') return
      setPauseMode('menu')
      setAppendInput('')
      setPauseSubmitting(false)
      setSelectedOption(0)
    }
  }, [pauseState, pauseMode])

  // When pause is cleared (setPauseState(null)), reset all pause UI state
  useEffect(() => {
    if (!pauseState) {
      setPauseMode('menu')
      setAppendInput('')
      setPauseSubmitting(false)
      setSelectedOption(0)
    }
  }, [pauseState])

  // Clear local pause when backend responds with pauseState
  useEffect(() => {
    if (pauseState && showPauseLocal) {
      setShowPauseLocal(false)
    }
  }, [pauseState, showPauseLocal])

  // Preparing timeout: transition to input after backend pause is received
  useEffect(() => {
    if (pauseMode !== 'preparing') return
    if (!showPauseLocal && !pauseState) return
    const timer = setTimeout(() => setPauseMode('input'), 800)
    return () => clearTimeout(timer)
  }, [pauseMode, showPauseLocal, pauseState])

  // Allow Escape to cancel preparing state and return to menu
  useEffect(() => {
    if (!pauseState || pauseMode !== 'preparing') return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setPauseMode('menu')
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [pauseState, pauseMode])

  const [input, setInput] = useState('')
  const [refineSelected, setRefineSelected] = useState(0) // 0=refine, 1=skip
  const [showRefineConfirm, setShowRefineConfirm] = useState(false)
  /** 强制线滑杆的后端拒绝反馈（越界 Err）；渲染在滑杆下方而非静默吞掉——
   *  「看得见失败」是该控件的契约。开合弹窗/执行提炼时清空。 */
  const [forceError, setForceError] = useState<string | null>(null)
  /* ── 外观浮窗 ──
     开关 state 放本组件内：面板常驻保活（打开/关闭不卸载内容），未保存调整跨开合存活。
     唯一的外部入口是 Ctrl+K 命令面板 —— 它经 App 层把 useModals.showThemes 置 true，
     这里同步展开；收起时回写，保证「收起 → 再按 Ctrl+K」仍是一次有效跳变。
     slash 菜单的 /themes 不走外部态，直接本地开（见 executeSlash）。 */
  const [showAppearance, setShowAppearance] = useState(false)
  const appearanceToggleRef = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (appearanceOpen) setShowAppearance(true)
  }, [appearanceOpen])
  const closeAppearance = useCallback(() => {
    setShowAppearance(false)
    onAppearanceDismiss?.()
  }, [onAppearanceDismiss])
  // 分母缺失（contextLimit 0/未知）→ 保持 0（前端消费端显示 "--"），不伪装 128000
  const [contextTotal, setContextTotal] = useState(contextLimit || 0)
  useEffect(() => {
    if (contextLimit != null && contextLimit > 0) setContextTotal(contextLimit)
  }, [contextLimit])
  const [modelLabel, setModelLabel] = useState('')
  const [relation, setRelation] = useState(loadRelation)
  const [copiedMsgId, setCopiedMsgId] = useState<string | null>(null)
  const [modelOpen, setModelOpen] = useState(false)
  const [skillsOpen, setSkillsOpen] = useState(false)
  const [switchingId, setSwitchingId] = useState<string | null>(null)
  const [modelSwitchError, setModelSwitchError] = useState<string | null>(null)
  const [hoveredProvider, setHoveredProvider] = useState<string | null>(null)
  const [providerMenuPosition, setProviderMenuPosition] = useState<{
    top?: number
    bottom?: number
    left: number
    maxHeight?: number
  } | null>(null)
  const providerHoverTimer = useRef<number | null>(null)
  /** 模型管理弹窗（.cmd-modal-sm）容器：模型列表浮层以它为对齐基准与高度上限 */
  const modelModalRef = useRef<HTMLDivElement | null>(null)

  // ── Slash Commands ──
  const SLASH_ITEMS = useMemo(
    () => [
      { id: 'new-chat', label: '/new', desc: t('slash.new'), category: '' },
      { id: 'models', label: '/models', desc: t('slash.models'), category: '' },
      { id: 'themes', label: '/themes', desc: t('slash.themes'), category: '' },
      { id: 'security', label: '/security', desc: t('slash.security'), category: '' },
      { id: 'browser', label: '/browser', desc: t('slash.browser'), category: '' },
      { id: 'memories', label: '/memories', desc: t('slash.memories'), category: '' },
      { id: 'workflows', label: '/workflow', desc: t('slash.workflow'), category: '' },
      { id: 'skills', label: '/skills', desc: t('slash.skills'), category: '' },
      { id: 'knowledge', label: '/knowledge', desc: t('slash.knowledge'), category: '' },
      { id: 'soul', label: '/soul', desc: t('slash.soul'), category: '' },
      { id: 'force-reset', label: '/reset', desc: t('slash.reset'), category: '' },
      { id: 'help', label: '/help', desc: t('slash.help'), category: '' },
      { id: 'snake-game', label: '/snake', desc: t('cmd.snakeGameDesc'), category: '' },
    ],
    [t],
  )
  const SLASH_ICONS: Record<string, ReactNode> = {
    workflows: <IconWorkflow size={14} />,
    memories: <IconHistory size={14} />,
    skills: <IconWrench size={14} />,
    knowledge: <IconFile size={14} />,
    models: <IconBrain size={14} />,
    themes: <IconPalette size={14} />,
    security: <IconShield size={14} />,
    browser: <IconBrowser size={14} />,
    soul: <IconSparkles size={14} />,
    'new-chat': <IconSquare size={14} />,
    'force-reset': <IconX size={14} />,
    help: <IconCopy size={14} />,
    'snake-game': <IconGrid size={14} />,
  }
  const [cmdOpen, setCmdOpen] = useState(false)
  const [cmdQuery, setCmdQuery] = useState('')
  const [cmdIdx, setCmdIdx] = useState(0)
  // ── Resource picker (triggered by /skills, /knowledge, /workflow slash commands) ──
  const [resPickerOpen, setResPickerOpen] = useState(false)
  const [resPickerType, setResPickerType] = useState<ChatReference['type'] | null>(null)
  type ResItem = { id: string; label: string; desc?: string }
  const [resItems, setResItems] = useState<ResItem[]>([])
  const [resLoading, setResLoading] = useState(false)
  const [resError, setResError] = useState<string | null>(null)
  const [resIdx, setResIdx] = useState(0)
  const resItemRefs = useRef<(HTMLDivElement | null)[]>([])
  const filteredSlash = useMemo(
    () =>
      cmdQuery
        ? SLASH_ITEMS.filter(i => i.label.toLowerCase().includes(cmdQuery.toLowerCase()))
        : SLASH_ITEMS,
    [cmdQuery, SLASH_ITEMS],
  )
  const handleCommandWheel = useWheelSelection(steps => {
    setCmdIdx(i => Math.max(0, Math.min(i + steps, filteredSlash.length - 1)))
  })
  const handleResourceWheel = useWheelSelection(steps => {
    setResIdx(i => Math.max(0, Math.min(i + steps, resItems.length - 1)))
  })
  const cmdItemRefs = useRef<(HTMLDivElement | null)[]>([])
  useEffect(() => {
    const el = cmdItemRefs.current[cmdIdx]
    if (el) el.scrollIntoView({ block: 'nearest' })
  }, [cmdIdx])

  // ── Hints ──
  const HINTS = useMemo(
    () => [
      t('input.hint.shortcuts'),
      t('input.hint.commandsQueue'),
      t('input.hint.modes'),
      t('input.hint.desktop'),
    ],
    [t],
  )
  const [hintIndex, setHintIndex] = useState(0)
  const [hintFade, setHintFade] = useState(true)
  useEffect(() => {
    const t = setInterval(() => {
      setHintFade(false)
      setTimeout(() => {
        setHintIndex(i => (i + 1) % HINTS.length)
        setHintFade(true)
      }, 500)
    }, 30000)
    return () => clearInterval(t)
  }, [HINTS.length])

  // ── 当前工作目录（输入框 chip 的展示态）──
  // 数据源为后端配置（preferences 是当前目录与书签的单一事实源）；本组件只持有
  // chip 的展示目录，书签的消费方是会话工作台「项目」行与「创建项目」弹窗。
  const [projectDir, setProjectDir] = useState('')

  // 启动加载当前项目目录（输入框 chip 需在弹窗/切换前即可显示归属文件夹名）
  // + 一次性迁移旧版 localStorage 书签（旧版两套键互不相通 → 合并进后端）。
  // 书签本组件不持有：输入框 chip 已改为纯展示。
  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const [state, bookmarks] = await Promise.all([getProjectDir(), getProjectBookmarks()])
        if (cancelled) return
        setProjectDir(state.path)
        const merged = migrateLegacyProjectBookmarks(bookmarks)
        if (merged) await setProjectBookmarksCmd(merged)
      } catch {
        /* 启动读取失败：保持空态，不影响会话 */
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  /** 切换工作目录（落盘 + 后端向活跃会话注入变更提醒 + HUD 反馈）。
   *  返回 true = 已切到目标目录；会话工作台「组内新建对话 / 点击组内会话」据此判定
   *  是否需要继续（失败已由 HUD 反馈，不重复报错）。 */
  const switchProject = useCallback(async (path: string): Promise<boolean> => {
    try {
      const state = await setProjectDirCmd(path)
      setProjectDir(state.path)
      hudUpdate(`项目已切换：${state.name}`, 'info')
      return true
    } catch (e) {
      hudUpdate(friendlyIpcError(e, '切换项目失败'), 'warning')
      return false
    }
  }, [])

  const [savedConfigs, setSavedConfigs] = useState<
    { id: string; label: string; model: string; provider: string; baseUrl: string }[]
  >([])
  const [allProviders, setAllProviders] = useState<ProviderInfo[]>([])
  const [allModels, setAllModels] = useState<ModelInfo[]>([])
  // 当前推理深度（null = 提供商默认），provider 由当前配置解析（见 loadEffortContext）
  const [effort, setEffort] = useState<string | null>(null)
  const [currentProvider, setCurrentProvider] = useState('')
  const [showOnboarding, setShowOnboarding] = useState(false)
  const loadSavedConfigs = async () => {
    const providers = (await getSupportedProviders().catch(() => [])) || []
    setAllProviders(providers)

    const activeCfg = await getCurrentConfig().catch(() => null)
    // 所有在 config.toml 中有 API key 的 provider 列表
    const configuredProviders: string[] = activeCfg?.configured_providers || []

    const configs: {
      id: string
      label: string
      model: string
      provider: string
      baseUrl: string
    }[] = []
    const seen = new Set<string>()

    providers.forEach(p => {
      // 放行条件：官方/网关 provider 需已配 key；自定义实例（custom / custom-xxx）
      // 与本地服务地址用户自填（可能本就无鉴权），不要求 key。
      const isCustom = isCustomProviderId(p.id)
      const allowed = configuredProviders.includes(p.id) || isCustom || p.id === 'local'
      if (!allowed) return
      // 读取该 provider 持久化的当前 model。自定义实例即使还没有 currentModel
      // 也要出现在弹窗里（刚创建、尚未选模型的实例必须可见可选），model 空串即可。
      let currentModel = ''
      try {
        currentModel = localStorage.getItem(`nuphus_current_model_${p.id}`) || ''
      } catch {
        /* localStorage 读取失败按未配置处理 */
      }
      if (!currentModel && !isCustom) return
      const key = `${p.id}::${currentModel}`
      if (seen.has(key)) return
      seen.add(key)
      configs.push({
        id: key,
        // 显示名跟随用户设置（display_name 优先），不再用段名
        label: p.display_name || p.name,
        model: currentModel,
        provider: p.id,
        baseUrl: p.base_url,
      })
    })
    return configs
  }

  // ── 推理深度：解析当前 mode 生效模型的 provider 归属，加载已配置值 + 模型元数据（支持的级别）──
  // 注意：本函数只负责 provider/effort 上下文，禁止写 modelLabel——
  // modelLabel 唯一数据源是 getEffectiveModel(mode)（异步覆盖会把输入框显示打回
  // 默认模型）。provider 用 get_provider_context（与 getEffectiveModel 同一
  // effective_model 解析点，mode 感知 + 内存/磁盘/[last_model] 权威归属），不再用
  // getCurrentConfig——其 provider 是 runtime 全局模型，同 id 跨段时会定位错卡。
  const loadEffortContext = useCallback(async () => {
    const ctx = await getProviderContext(mode || 'leader').catch(() => null)
    const provider = ctx?.provider || ''
    setCurrentProvider(provider)
    if (provider) {
      try {
        const e = await getReasoningEffort(provider)
        setEffort(e ?? null)
      } catch {
        setEffort(null)
      }
    } else {
      setEffort(null)
    }
    try {
      const list = await listModels()
      if (Array.isArray(list)) setAllModels(list)
    } catch {
      /* 模型元数据加载失败时保持现状（入口隐藏） */
    }
  }, [mode])

  useEffect(() => {
    if (!modelOpen) {
      setHoveredProvider(null)
      setProviderMenuPosition(null)
      if (providerHoverTimer.current) window.clearTimeout(providerHoverTimer.current)
    }
    return () => {
      if (providerHoverTimer.current) window.clearTimeout(providerHoverTimer.current)
    }
  }, [modelOpen])

  const openProviderModels = useCallback((provider: string, anchor?: HTMLElement) => {
    if (providerHoverTimer.current) window.clearTimeout(providerHoverTimer.current)
    if (anchor) {
      const rect = anchor.getBoundingClientRect()
      const gap = 8
      const menuWidth = 236

      // 以「models 弹窗」为基准，而非 window：列表浮层是弹窗的附属层，
      // 上限与对齐都应相对弹窗。
      const modalEl = modelModalRef.current
      const modal = modalEl?.getBoundingClientRect()
      const boundTop = modal ? modal.top : 8
      const boundBottom = modal ? modal.bottom : window.innerHeight - 8
      const boundHeight = boundBottom - boundTop

      // 高度上限 = 弹窗高度的一半。这是**上限**，不是定高：
      // 浮层实际高度仍由内容决定（CSS max-height），1 个模型就只占 1 行高，
      // 不留空白。
      const maxHeight = Math.max(120, Math.floor(boundHeight / 2))

      // 横向：优先贴 provider 行右侧；右侧不够则翻到左侧
      const left =
        rect.right + gap + menuWidth <= window.innerWidth
          ? rect.right + gap
          : Math.max(8, rect.left - gap - menuWidth)

      // 纵向对齐规则：以弹窗中线为界
      //  - 行在**上半部** → 浮层顶部与行顶部对齐（top）
      //  - 行在**下半部** → 浮层底部与行底部对齐（bottom）
      //
      // 下半部必须用 bottom 定位而不是 top = rect.bottom - maxHeight：
      // 后者假定「浮层高度 = 上限」，模型只有 1-2 个时浮层远短于上限，
      // 按上限反推的 top 会让浮层**悬空**、脱离触发它那一行。
      // 用 bottom 则恒定让浮层底边贴住行底边，高度随内容自适应向上生长。
      const modalMid = boundTop + boundHeight / 2
      const alignTop = rect.top + rect.height / 2 <= modalMid

      if (alignTop) {
        setProviderMenuPosition({ top: rect.top, left, maxHeight })
      } else {
        // bottom 语义 = 距视口底的距离（fixed 定位）
        setProviderMenuPosition({
          bottom: window.innerHeight - rect.bottom,
          left,
          maxHeight,
        })
      }
    }
    setHoveredProvider(provider)
  }, [])

  const closeProviderModelsSoon = useCallback(() => {
    if (providerHoverTimer.current) window.clearTimeout(providerHoverTimer.current)
    providerHoverTimer.current = window.setTimeout(() => {
      setHoveredProvider(null)
      setProviderMenuPosition(null)
    }, 160)
  }, [])

  // ── 模型切换：点击外侧模型项 → 切换到该具体模型（切后自动关弹窗）──
  const switchConfig = useCallback(
    async (
      cfg: { id: string; label: string; model: string; provider: string; baseUrl: string },
      closeAfter: boolean,
    ) => {
      if (switchingId) return
      setSwitchingId(cfg.id)
      setModelSwitchError(null)
      let switched = false
      try {
        // 不下发内置默认地址：它对自定义/中转端点只是文档占位示例，会被后端当作
        // 「显式地址」顶掉 config.toml 里用户填写的真实地址。空值交给后端解析
        // （显式参数 → 已存配置 → 内置默认）。
        // context_window 兜底：从 list_models 磁盘元数据查该 (provider, model) 行，
        // 缺失时后端保持原值；显式传入避免切换后 runtime 窗口丢失。
        const ctxWin = allModels.find(
          m => m.id === cfg.model && m.provider === cfg.provider,
        )?.context_window
        await switchModel(cfg.model, cfg.provider, '', ctxWin, mode)
        switched = true
        playUiSound('switch')
        const limit = await getContextLimit()
        if (limit != null && limit > 0) setContextTotal(limit)
        onModelChanged?.()
        setModelLabel(cfg.model)
        // 同步持久化该 provider 的当前 model,保持与 /models 切换一致
        try {
          localStorage.setItem(`nuphus_current_model_${cfg.provider}`, cfg.model)
        } catch {
          /* localStorage 写入失败不阻塞切换流程 */
        }
        // 输入框弹窗子列表排序用：把这次选的模型置顶。只影响展示顺序，
        // 不参与模型解析/切换/生效判定（后者走 get_provider_context）。
        rememberRecentModel(localStorage, cfg.provider, cfg.model)
        // 本地同步 savedConfigs（切换后提供商项立即显示新模型名 + ✓）
        setSavedConfigs(prev =>
          prev.map(c =>
            c.provider === cfg.provider
              ? { ...c, model: cfg.model, id: `${cfg.provider}::${cfg.model}` }
              : c,
          ),
        )
        await new Promise(r => setTimeout(r, 300))
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        console.error('[Model] switch failed:', error)
        setModelSwitchError(`模型切换失败：${message}`)
      }
      setSwitchingId(null)
      if (closeAfter && switched) setModelOpen(false)
    },
    [switchingId, allProviders, allModels, mode, onModelChanged],
  )

  // 切换推理深度：写入 config.toml + 触发 Runtime 重建（后端已就绪，前端无需额外刷新）
  const handleEffortChange = useCallback(
    async (next: string | null) => {
      if (!currentProvider) return
      try {
        await setReasoningEffort(currentProvider, next)
        setEffort(next)
      } catch (e) {
        console.error('[Effort] set failed:', e)
      }
    },
    [currentProvider],
  )

  // 贴底跟随（useStickyScroll）：默认跟随流式输出贴底；用户鼠标上滚 / 拖拽滚动条即
  // 立即解锁，停在任意位置阅读不再被拽回。
  //
  // bottomZonePx = 80（对话窗宽容语义，2026-10-10 修回归）：严格版要求「4px 内 + 手势收尾
  // 瞬间」双条件才恢复跟随，在流式增长下几乎不可达（用户滚回底部松手时内容已又长高一截）
  // → 自动下拉永久丢失；且内容变矮（草稿被 progress 替换 / think 块剥离）时浏览器把
  // scrollTop 钳回底部，会被严格版误读成「用户上滚」而静默解锁。宽容语义下：
  // 「用户向下滚回区内」是唯一恢复入口（内容增长仍不恢复）、收缩钳位不再误解锁。
  // 执行追踪面板（ExecutionTraceFloating）不传该选项，保持原严格语义不变。
  // smoothPull = true（2026-10-11）：流式增长与回底按钮改走 rAF 追踪循环（每帧向底部
  // 收拢、目标每帧重测），「瞬移跳动」变连续滑移；程序性回填（followReset）保持瞬移。
  const { scrollRef, showJumpButton, onScroll, jumpToBottom, followReset, nudgeScrollTop } =
    useStickyScroll(messages, { bottomZonePx: 80, smoothPull: true })

  // ── Refine 折叠：refine 前历史消息默认折叠（小巧占位），解决多轮 refine 后
  // 前端历史气泡累积卡顿。点击折叠条展开/折叠。expandedRefineIds 记录展开的段。
  //
  // 两条体验铁律（大王 10-21）：
  // 1) refine 进行中（streaming）不折叠它前面的历史——刚触发提炼就把上文藏了，
  //    用户看不到提炼正在处理的上下文；等 refine 完成（completed）再折叠。
  // 2) 折叠/展开不跳顶：历史消息批量显隐会骤改滚动高度，浏览器把视口重定位到
  //    顶部。操作前记录 refine 分隔线的视口位置，DOM 提交后补偿 scrollTop；贴底
  //    跟随时直接回底（用户在底部看最新内容，不该被中间折叠条拽到上文去）。
  const [expandedRefineIds, setExpandedRefineIds] = useState<Set<string>>(new Set())
  const refineAnchorRef = useRef<{ id: string; atBottom: boolean; viewportY: number } | null>(null)
  const toggleRefineExpand = useCallback(
    (id: string) => {
      const container = scrollRef.current
      if (container) {
        const anchor = container.querySelector(`[data-refine-anchor="${id}"]`) as HTMLElement | null
        refineAnchorRef.current = {
          id,
          atBottom: !showJumpButton,
          viewportY: anchor
            ? anchor.getBoundingClientRect().top - container.getBoundingClientRect().top
            : 0,
        }
      }
      setExpandedRefineIds(prev => {
        const next = new Set(prev)
        if (next.has(id)) next.delete(id)
        else next.add(id)
        return next
      })
    },
    [showJumpButton],
  )
  // DOM 提交后（绘制前）恢复锚点：贴底则回底，否则把 refine 分隔线挪回原视口位置，
  // 抵消历史消息批量显隐造成的滚动高度跳变。
  useLayoutEffect(() => {
    const anchor = refineAnchorRef.current
    if (!anchor) return
    refineAnchorRef.current = null
    const container = scrollRef.current
    if (!container) return
    if (anchor.atBottom) {
      jumpToBottom()
      return
    }
    const el = container.querySelector(`[data-refine-anchor="${anchor.id}"]`) as HTMLElement | null
    if (!el) return
    const newViewportY = el.getBoundingClientRect().top - container.getBoundingClientRect().top
    const delta = newViewportY - anchor.viewportY
    if (delta !== 0) nudgeScrollTop(delta)
  }, [expandedRefineIds, nudgeScrollTop, jumpToBottom])
  // 按 refine 边界分段：每个 refine 消息前的连续消息为可折叠段
  const refineSegments = useMemo(() => {
    const segs: {
      refineId: string
      beforeStart: number
      beforeEnd: number
      count: number
      streaming: boolean
    }[] = []
    let segStart = 0
    messages.forEach((msg, idx) => {
      if (msg.role === 'refine') {
        segs.push({
          refineId: msg.id,
          beforeStart: segStart,
          beforeEnd: idx - 1,
          count: idx - segStart,
          streaming: msg.refineStatus === 'streaming',
        })
        segStart = idx + 1
      }
    })
    return segs
  }, [messages])
  // 折叠段内的消息索引（未展开的段）——map 时跳过，不渲染。
  // streaming 段不折叠：提炼进行中保留上文可见（见上方铁律 1）。
  const collapsedBeforeIds = useMemo(() => {
    const set = new Set<number>()
    refineSegments.forEach(seg => {
      if (!seg.streaming && !expandedRefineIds.has(seg.refineId) && seg.count > 0) {
        for (let i = seg.beforeStart; i <= seg.beforeEnd; i++) set.add(i)
      }
    })
    return set
  }, [refineSegments, expandedRefineIds])
  // followReset 回填：useEvents 在 App 层（本组件之上）监听 nuphus-event，
  // execution_started（新轮次恢复跟随后续流式）/ execution_completed（完成瞬间补拉）
  // 都要拉回底部 —— 函数不能经 props 上行，父层下发 followResetRef、本组件把最新闭包
  // 写进去（useEvents 事件时读 .current，与 useEvents 的 refineStateRef 同一思路）。
  // 子组件 effect 先于父组件 useEvents 的监听注册执行，挂载即就绪；卸载置空防陈旧调用。
  useEffect(() => {
    if (!followResetRef) return
    followResetRef.current = followReset
    return () => {
      followResetRef.current = null
    }
  }, [followResetRef, followReset])
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const overlayRef = useRef<HTMLDivElement>(null)
  // Reload relation config on mount
  useEffect(() => {
    setRelation(loadRelation())
    // 启动迁移：把历史用户 localStorage 里已有的称呼持久化到后端 relation.json，
    // 保证老用户升级后手机端 /identity 立即拿到用户定义的称呼（桌面端未发消息时也生效）。
    void persistRelationToBackend(loadRelation()).catch(() => {})
    // modelLabel 不在此初始化：统一由下方「mode 生效模型」effect 负责（mode 感知）
    loadSavedConfigs().then(setSavedConfigs)
    void loadEffortContext()
  }, [])

  // 首次启动引导检测
  useEffect(() => {
    const done = localStorage.getItem('nuphus_onboarding_done')
    if (done === 'true') return
    isLlmConfigured()
      .then(ok => {
        if (ok) {
          localStorage.setItem('nuphus_onboarding_done', 'true')
        } else {
          setShowOnboarding(true)
        }
      })
      .catch(() => setShowOnboarding(true))
  }, [])

  // ── modelLabel 唯一权威数据源：当前 mode 的生效模型（后端 effective_model 单点解析）──
  // 触发时机：挂载 / mode 切换 / session_info 推送（modelName 变化，如模型切换广播）。
  // 禁止直接用 modelName（全局 runtime 模型，非 leader mode 下与 mode 生效模型不一致）
  // 或 getCurrentConfig（config.toml 根模型锚点）写入 modelLabel。
  useEffect(() => {
    getEffectiveModel(mode || 'leader')
      .then(m => {
        if (m) setModelLabel(m)
      })
      .catch(() => {
        if (modelName) setModelLabel(modelName)
      })
    void loadEffortContext()
  }, [mode, modelName, loadEffortContext])

  // 当前模型声明的可选推理深度（来自内置 ModelDef 元数据；空 = 不支持配置，隐藏入口）
  // (provider, model) 二元组限定：同 id 跨段（deepseek-v4-flash 同时在 deepseek 官方段
  // 与 opencode-go 段；kimi-k3 / step-5-preview 同理）时，只按名称 find 会命中另一段的
  // 档位 → 显示的档位与 handleEffortChange 实际写入的段分叉。currentProvider 与
  // modelLabel 同出 effective_model 解析点（见下方弹窗勾选逻辑），天然同步；
  // currentProvider 未就绪时宁可暂无档位，也不错贴（宁缺勿错，同后端 list_models）。
  const currentModelEfforts = useMemo(() => {
    const id = modelLabel || modelName || ''
    if (!id || !currentProvider) return []
    return (
      allModels.find(m => m.id === id && m.provider === currentProvider)?.reasoning_efforts ?? []
    )
  }, [allModels, modelLabel, modelName, currentProvider])

  // 当前模型的默认推理强度（未配置时生效；null = 无声明，UI 显示「默认」）
  // 同上：(provider, model) 二元组限定，不做名称兜底。
  const currentModelDefaultEffort = useMemo(() => {
    const id = modelLabel || modelName || ''
    if (!id || !currentProvider) return null
    return (
      allModels.find(m => m.id === id && m.provider === currentProvider)?.default_effort ?? null
    )
  }, [allModels, modelLabel, modelName, currentProvider])

  // Reload latest config when quick-switch modal opens
  useEffect(() => {
    if (!modelOpen) return
    // 勾选态跟随当前 mode 的生效模型（非 config.toml 根模型）
    getEffectiveModel(mode || 'leader')
      .then(m => {
        if (m) setModelLabel(m)
      })
      .catch(() => {})
    loadSavedConfigs().then(setSavedConfigs)
  }, [modelOpen])

  // Focus input when focusSignal changes
  useEffect(() => {
    if (focusSignal && focusSignal > 0) {
      textareaRef.current?.focus()
    }
  }, [focusSignal])

  const autoResize = useCallback(() => {
    const ta = textareaRef.current
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = Math.min(ta.scrollHeight, 240) + 'px'
  }, [])

  // Listen for workflow export-to-chat events
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail
      const text = detail?.text
      const mode = detail?.mode
      if (text) {
        setInput(prev => (prev ? prev + '\n' + text : text))
        autoResize()
        // 追加后聚焦输入框并置光标到末尾（右键「问问 Nuphus」/工作流导出体验一致）。
        // 聚焦语义与引用入栏共用一份实现（inputFocus.ts）——两条链都是
        // 「内容进输入区后焦点交给输入框」。
        requestAnimationFrame(() => focusInputAtEnd(textareaRef.current))
      }
      if (mode && onSetMode) {
        onSetMode(mode)
      }
    }
    window.addEventListener('nuphus:append-to-chat', handler)
    return () => window.removeEventListener('nuphus:append-to-chat', handler)
  }, [])

  // Reset refining and selection state when refine modal closes
  useEffect(() => {
    if (!refineState) {
      setRefining?.(false)
      setRefineSelected(0)
    }
  }, [refineState, setRefining])

  // Refine modal keyboard nav: ↑↓ select + Enter confirm + Esc skip
  useEffect(() => {
    if (!refineState || refining) return
    const el = overlayRef.current
    if (!el) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setRefineSelected(prev => (prev > 0 ? prev - 1 : 1))
      } else if (e.key === 'ArrowDown') {
        e.preventDefault()
        setRefineSelected(prev => (prev < 1 ? prev + 1 : 0))
      } else if (e.key === 'Enter') {
        e.preventDefault()
        if (refineSelected === 0) {
          setRefining?.(true)
          onRefine?.()
        } else {
          onSkipRefine?.()
        }
      } else if (e.key === 'Escape') {
        e.preventDefault()
        onSkipRefine?.()
      }
    }
    el.addEventListener('keydown', handler)
    return () => el.removeEventListener('keydown', handler)
  }, [refineState, refining, onRefine, onSkipRefine, refineSelected, setRefining])

  const handleInputChange = useCallback((v: string) => {
    setInput(v)
    if (v.startsWith('/') && !v.includes(' ')) {
      setCmdQuery(v)
      setCmdOpen(true)
      setCmdIdx(0)
    } else {
      setCmdOpen(false)
      setResPickerOpen(false)
    }
  }, [])

  // SLASH_ITEMS id (plural) → ChatReference.type (singular)
  const RES_TYPE_MAP: Record<string, ChatReference['type']> = {
    skills: 'skill',
    knowledge: 'knowledge',
    workflows: 'workflow',
  }

  const executeSlash = (id: string) => {
    setCmdOpen(false)
    // Intercept resource commands → open resource picker
    if (id in RES_TYPE_MAP) {
      openResourcePicker(RES_TYPE_MAP[id])
      return
    }
    setInput('')
    // 外观面板在本组件内，就地展开（不再绕 App 层的模态入口）
    if (id === 'themes') {
      setShowAppearance(true)
      return
    }
    onCommand?.(id)
  }

  const executeSlashByIndex = useCallback(() => {
    const item = filteredSlash[cmdIdx]
    if (item) {
      setCmdOpen(false)
      const refType = RES_TYPE_MAP[item.id]
      if (refType) {
        openResourcePicker(refType)
      } else {
        setInput('')
        // 同 executeSlash：/themes 就地展开外观浮窗
        if (item.id === 'themes') {
          setShowAppearance(true)
          return
        }
        onCommand?.(item.id)
      }
    }
  }, [filteredSlash, cmdIdx, onCommand])

  useEffect(() => {
    if (!cmdOpen) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        setCmdOpen(false)
        return
      }
      // 空结果（如已有文字前加 "/" 无匹配）：不拦截键盘，放行正常输入/发送
      if (filteredSlash.length === 0) return
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setCmdIdx(i => Math.min(i + 1, filteredSlash.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setCmdIdx(i => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Enter') {
        e.preventDefault()
        executeSlashByIndex()
        return
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [cmdOpen, filteredSlash.length, executeSlashByIndex])

  const handleSubmit = () => {
    // 执行中发送 = 追加指令（与手机端一致）：不禁用。仅 refine 弹窗打开时拦截。
    if (!input.trim() || refineState) return
    playUiSound('send')
    // 将 pendingFiles 注入到 input 文本末尾
    let finalInput = input
    if (pendingFiles.length > 0) {
      const fileRefs = pendingFiles.map(f => `[附件: ${f.path}]`).join('\n')
      finalInput = input + '\n' + fileRefs
    }
    // 发送前快照：收尾期拒收时按原样退回（含图片/附件/引用，避免「只退回文字」的半丢失）
    const sentImageItems = pendingImages
    const sentRefs = pendingReferences
    const sentFiles = pendingFiles
    const pending = onSend(
      finalInput,
      sentImageItems.length > 0 ? sentImageItems.map(p => p.dataUrl) : undefined,
      sentRefs.length > 0 ? sentRefs : undefined,
    )
    setInput('')
    setPendingImages([])
    setPendingReferences([])
    setPendingFiles([])
    /* 收尾期拒收（后端 rejected="finalizing"）：主循环已退出、后端正在收尾，
       消息**未被受理**（未入队、未记去重基准）→ 原文原样退回输入框 + 提示稍后重发。
       既有入口保持「先清空再发送」的手感，这里按回执回填；条件回填：期间用户已开始
       输入新内容时不覆盖（不吞用户新打的字）。 */
    void Promise.resolve(pending)
      .then(outcome => {
        if (!outcome?.rejected) return
        setInput(prev => (prev.trim() ? prev : finalInput))
        setPendingImages(sentImageItems)
        setPendingReferences(sentRefs)
        setPendingFiles(sentFiles)
        hudUpdate(outcome.message || t('toast.finalizingPleaseResend'), 'warning')
      })
      .catch(() => {})
    requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto'
      }
    })
  }

  const handleRetry = (msg: ChatMessage) => {
    onRetry?.(msg.content, msg.id)
  }

  const handleCopy = async (msgId: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopiedMsgId(msgId)
      setTimeout(() => setCopiedMsgId(null), 1500)
    } catch {
      // fallback: select and exec copy
      const ta = document.createElement('textarea')
      ta.value = text
      document.body.appendChild(ta)
      ta.select()
      document.execCommand('copy')
      document.body.removeChild(ta)
      setCopiedMsgId(msgId)
      setTimeout(() => setCopiedMsgId(null), 1500)
    }
  }

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return

    // 与拖拽同一条通道：图片/文档一律进引用栏，不再把非图片内容读成文本塞进输入框。
    void (async () => {
      try {
        const { entries, failures } = await filesToAttachments([file])
        for (const entry of entries) {
          if (entry.kind === 'image') handleImageAttach(entry.image)
          else handleFileAttach(entry.file)
        }
        if (failures.length > 0) hudUpdate(failures.join('; '), 'error')
      } catch (e) {
        hudUpdate(e instanceof Error ? e.message : String(e), 'error')
      }
    })()
    e.target.value = ''
  }

  /** Handle image attached via drag-drop or paste */
  const handleImageAttach = (file: { name: string; dataUrl: string }) => {
    setPendingImages(prev => [...prev, { dataUrl: file.dataUrl, name: file.name }])
  }

  // ── Reference helpers ──
  const addReference = useCallback((ref: ChatReference) => {
    setPendingReferences(prev => {
      if (prev.some(r => r.type === ref.type && r.id === ref.id)) return prev
      return [...prev, ref]
    })
  }, [])

  const removeReference = useCallback((index: number) => {
    setPendingReferences(prev => prev.filter((_, i) => i !== index))
  }, [])

  /**
   * 引用入栏（统一入口）：加 chip + 聚焦输入框、光标置末尾。
   *
   * 两条来源共用：右键菜单「引用这段」（`nuphus:add-quote-reference` 事件）
   * 与预览浮层「发送给 Agent」（PreviewOverlay.onSendAnnotations）。
   * 聚焦是必需语义而非锦上添花——同族 append-to-chat 一直聚焦，
   * 引用通道漏了它会让用户入栏后必须再点一次输入框才能打字
   * （仓库 issue #98）。rAF 理由见 inputFocus.ts 头注。
   */
  const addQuoteReference = useCallback(
    (ref: ChatReference) => {
      addReference(ref)
      requestAnimationFrame(() => focusInputAtEnd(textareaRef.current))
    },
    [addReference],
  )

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ text?: string }>).detail
      const ref = buildQuoteRef(detail?.text ?? '')
      if (ref) addQuoteReference(ref)
    }
    window.addEventListener('nuphus:add-quote-reference', handler)
    return () => window.removeEventListener('nuphus:add-quote-reference', handler)
  }, [addQuoteReference])

  const removePendingImage = useCallback((index: number) => {
    setPendingImages(prev => prev.filter((_, i) => i !== index))
  }, [])

  const handleFileAttach = useCallback((file: PendingFile) => {
    setPendingFiles(prev => {
      if (prev.some(f => f.path === file.path)) return prev
      return [...prev, file]
    })
  }, [])

  const removePendingFile = useCallback((index: number) => {
    setPendingFiles(prev => prev.filter((_, i) => i !== index))
  }, [])

  // ── Capture result from DesktopToolbar (右侧竖条发起的截图) ──
  useEffect(() => {
    const handler = (e: Event) => {
      const { path, region, base64 } = (e as CustomEvent).detail as {
        path: string
        region: { x: number; y: number; width: number; height: number }
        base64?: string | null
      }
      if (!path) return
      const fileName = path.split(/[\\/]/).pop() || path
      addReference({
        type: 'capture',
        id: path,
        label: `${fileName} (${region.width}×${region.height})`,
        meta: { region, base64: base64 || undefined },
      })
    }
    window.addEventListener('nuphus:capture-result', handler)
    return () => window.removeEventListener('nuphus:capture-result', handler)
  }, [addReference])

  // ── Resource picker ──
  const openResourcePicker = useCallback(async (type: ChatReference['type']) => {
    setInput('') // clear slash command so stale text isn't sent
    setCmdQuery('')
    setCmdOpen(false)
    setResPickerOpen(true)
    setResPickerType(type)
    setResLoading(true)
    setResError(null)
    setResIdx(0)
    setResItems([])
    try {
      if (type === 'skill') {
        const { invoke } = await import('@tauri-apps/api/core')
        const list =
          await invoke<Array<{ name: string; display_name: string; description: string }>>(
            'skill_list',
          )
        setResItems(
          list.map(s => ({ id: s.name, label: s.display_name || s.name, desc: s.description })),
        )
      } else if (type === 'knowledge') {
        const { listKnowledge } = await import('../lib/api')
        const list = await listKnowledge()
        setResItems(
          (list ?? []).map(k => ({
            id: k.rel_path,
            label: k.title || k.rel_path,
            desc: k.snippet,
          })),
        )
      } else if (type === 'workflow') {
        const { listWorkflows } = await import('../lib/api')
        const list = await listWorkflows()
        setResItems((list ?? []).map(w => ({ id: w.id, label: w.title, desc: w.description })))
      }
    } catch (err: unknown) {
      setResError(err instanceof Error ? err.message : '加载失败')
    } finally {
      setResLoading(false)
    }
  }, [])

  const closeResourcePicker = useCallback(() => {
    setResPickerOpen(false)
    setResPickerType(null)
    setResItems([])
    setResError(null)
  }, [])

  const selectResource = useCallback(
    (item: ResItem) => {
      if (!resPickerType) return
      addReference({ type: resPickerType, id: item.id, label: item.label })
      closeResourcePicker()
    },
    [resPickerType, addReference, closeResourcePicker],
  )

  // ── Resource picker keyboard nav ──
  useEffect(() => {
    if (!resPickerOpen) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        closeResourcePicker()
        return
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setResIdx(i => Math.min(i + 1, resItems.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setResIdx(i => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Enter') {
        e.preventDefault()
        const item = resItems[resIdx]
        if (item) selectResource(item)
        return
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [resPickerOpen, resItems, resIdx, selectResource, closeResourcePicker])

  // Scroll selected resource item into view
  useEffect(() => {
    const el = resItemRefs.current[resIdx]
    if (el) el.scrollIntoView({ block: 'nearest' })
  }, [resIdx])

  /**
   * Ctrl/Cmd+Enter = 换行：受控 textarea 在 Ctrl+Enter 下没有原生换行行为，
   * 需按当前光标/选区手动插入 `\n`，并在重渲染后把光标落回换行符之后。
   * 高度自适应由 ChatInputBar 的 input 变化 effect 负责，这里不重复处理。
   */
  const insertLineBreakAtCaret = () => {
    const ta = textareaRef.current
    const start = ta?.selectionStart ?? input.length
    const end = ta?.selectionEnd ?? start
    handleInputChange(`${input.slice(0, start)}\n${input.slice(end)}`)
    requestAnimationFrame(() => {
      const el = textareaRef.current
      if (el) el.selectionStart = el.selectionEnd = start + 1
    })
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    // ── IME 组词态守卫（必须置于所有 Enter 分支之前）──
    // 中文/日文输入法组词时按 Enter = 确认候选词上屏，不是发送。WebView2(Chromium)
    // 在该态仍会派发 keydown（key='Enter'、isComposing=true，部分环境 keyCode=229），
    // 若无此守卫会被下方发送分支截获 → 候选词未上屏消息已飞出（用户反馈的"误发"）。
    // 组词态下 Ctrl/Cmd+Enter 与 Enter 一律让位给输入法：选词期间的组合键不该生效。
    // 同 Inspector.tsx / VariablePicker.tsx 的既有防护模式；mobile/components/Composer.tsx
    // 与 examples/agent-chat/index.html 有等价守卫，改动时三处同步。
    if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return
    // Ctrl/Cmd+Enter：换行（不再等同于发送；Shift+Enter 仍是原生换行）
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      insertLineBreakAtCaret()
      return
    }
    if (e.key === 'Enter' && !e.shiftKey && !cmdOpen && !resPickerOpen) {
      e.preventDefault()
      handleSubmit()
    }
  }

  return (
    <div className="chat-panel">
      {/* ── Session Rail：面板级左缘挂载（收起态 = 左缘一枚色块，点击滑出左侧抽屉）── */}
      {onChatReplaced && (
        <SessionRail
          onSessionChanged={onChatReplaced}
          onNewChat={onNewChat}
          onSwitchProjectDir={switchProject}
          onModeSwitched={onModeSwitched}
          // rail 锁定直接来自唯一执行态（Running ∨ Finalizing 都锁），
          // 不再与 can_switch 做 OR 派生（见 SessionRail.hardLocked）
          locked={backendLocked}
          mood={mood}
        />
      )}
      {/* ── Chat Header：右上角按钮列（设置 + 外观）──
          齿轮 = 全应用唯一设置入口（打开设置中心弹窗）；
          调色板 = 外观浮窗（主题 / 微调 / 我的主题 / 语言，非模态常驻）。 */}
      <div className="chat-header">
        {/* 左上角：App 层组合的常驻 chrome 槽位（Exec 执行指示器）。
            内部元素以 absolute 落位，故不会把 header 撑高（右侧三钮位置零变化）。 */}
        <div className="chat-header-left">{execIndicator}</div>
        {/* island 落点锚点：左右两组之间的中央留白（几何见 styles/app-pill.css 的
            .island-slot：绝对定位铺满 header 的 padding box 并 flex 居中，不参与
            space-between 排布 → 两侧内容位置不变）。
            AppIsland 经 portal 渲染到这里；锚点不存在时（聊天视图未挂载）岛按
            锚点优先级回落 —— 见 ui/islandChannel.ts 的「落点锚点」。 */}
        <div className="island-slot" ref={setIslandAnchor} />
        {/* 右侧按钮列：设置（齿轮）→ 外观（调色板）→ 浏览器（罗盘）纵向同一 DOM 排布；
            refine chip 有值时挂在这一列尾部自然下延 —— 它原先 absolute 在
            header 外（top:50px/right:16px/z-index:20），与 header 内的其它浮层
            互相抢层级；并入本列后改为 in-flow，z-index 竞争随之消失 */}
        <div className="chat-header-right">
          <button
            className="chat-header-settings-btn"
            aria-label={t('app.settings')}
            onClick={() => onOpenSettings?.()}
          >
            <IconSettings size={15} />
            <span className="chat-header-btn-label">{t('app.settings')}</span>
          </button>
          <button
            ref={appearanceToggleRef}
            className="chat-header-appearance-btn"
            aria-label={t('app.appearance')}
            onClick={() => setShowAppearance(o => !o)}
          >
            <IconPalette size={15} />
            <span className="chat-header-btn-label">{t('app.appearance')}</span>
          </button>
          {/* 浏览器窗口：配了指纹浏览器时是用户那个（attach_external，绝不自启
              自管 Chrome），未配时是 Nuphus 自管 CDP Chrome、独立 profile。这里
              只做「唤出/置前」，Agent 的浏览与自动化都在这个窗口内进行。启动中
              禁用（is-busy），避免连点把单槽串行的后端排成长队。 */}
          <button
            className={`chat-header-browser-btn${browserOpening ? ' is-busy' : ''}`}
            aria-label={t('browser.toggle')}
            aria-busy={browserOpening}
            disabled={browserOpening}
            onClick={showBrowserWindow}
          >
            <IconBrowser size={15} />
            <span className="chat-header-btn-label">{t('browser.toggleTitle')}</span>
          </button>
          {pendingRefine && !refineState && !refining && (
            <div className="refine-pending-area">
              <button
                className="refine-pending-btn"
                onClick={() => {
                  setShowRefineConfirm(true)
                  setForceError(null)
                }}
                title={`${t('refine.pendingBtn')} (${pendingRefine.usagePercent}%)`}
              >
                <IconChartColumn size={14} />
                <span className="refine-pending-pct">{pendingRefine.usagePercent}%</span>
                {pendingRefine.skippedTurns > 0 && (
                  <span className="refine-pending-badge">
                    {pendingRefine.skippedTurns > 99 ? '99+' : pendingRefine.skippedTurns}
                  </span>
                )}
              </button>

              {/* ── Confirm dialog when button clicked ── */}
              {showRefineConfirm && (
                <div className="refine-pending-confirm">
                  <div className="refine-confirm-body">
                    <div className="item-desc">
                      {t('refine.pendingDesc', String(pendingRefine.usagePercent))}
                    </div>
                    {/* 与主弹窗同一套规则：只有大窗口档给 slider。
                        跳过只是不再自动弹窗，调节强制线的能力必须保留。 */}
                    {pendingRefine.tier === 'large' && (
                      <div className="refine-force">
                        <div className="refine-force-head">
                          <span>{t('refine.forceLabel')}</span>
                          <span className="refine-force-value">
                            {Math.round((forceDraft ?? pendingRefine.forceThreshold) * 100)}%
                          </span>
                        </div>
                        <input
                          className="refine-force-slider"
                          type="range"
                          min={refineForceMinPct(pendingRefine)}
                          max={Math.round(pendingRefine.forceMax * 100)}
                          step={5}
                          value={Math.max(
                            refineForceMinPct(pendingRefine),
                            Math.round((forceDraft ?? pendingRefine.forceThreshold) * 100),
                          )}
                          aria-label={t('refine.forceLabel')}
                          onChange={e => {
                            const next = Number(e.target.value) / 100
                            setForceDraft?.(next)
                            setRefineForceThreshold(next).catch((err: unknown) => {
                              const msg = err instanceof Error ? err.message : String(err)
                              setForceError(`强制线设置失败：${msg}`)
                            })
                          }}
                        />
                        {forceError && <div className="refine-force-warn">{forceError}</div>}
                        {refineForceMinPct(pendingRefine) >
                          Math.round(pendingRefine.forceMin * 100) && (
                          <div className="refine-force-warn">
                            {t(
                              'refine.forceFloorRaised',
                              String(refineForceMinPct(pendingRefine)),
                              String(Math.round(pendingRefine.usagePercent)),
                            )}
                          </div>
                        )}
                        <div className="refine-force-hint">{t('refine.forceHint')}</div>
                      </div>
                    )}
                    <div className="refine-confirm-actions">
                      <button
                        className="refine-confirm-btn"
                        onClick={() => {
                          // 提炼执行中（refining 由另一端/本端刚触发）不重复触发：
                          // 关闭确认弹窗但不发起第二次 refine
                          if (refining) {
                            setShowRefineConfirm(false)
                            return
                          }
                          setShowRefineConfirm(false)
                          // 与 refine 弹窗路径一致：进入提炼中状态（全屏遮罩由全局
                          // refining 驱动，refine-pending-btn 路径同样触发）
                          setRefining?.(true)
                          onRefine?.()
                          setPendingRefine(null)
                        }}
                      >
                        {t('refine.action')}
                      </button>
                      <button
                        className="refine-confirm-cancel"
                        onClick={() => setShowRefineConfirm(false)}
                      >
                        {t('common.cancel') || 'Cancel'}
                      </button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
      {/* 外观浮窗：开关 state 在本组件内（保活不卸载），挂在 chat-header 之后、
          与 refine 列同级；面板自身 fixed，不受父级布局影响 */}
      <AppearancePanel
        open={showAppearance}
        onClose={closeAppearance}
        toggleRef={appearanceToggleRef}
      />
      {/* .chat-messages-wrap 是回底按钮的定位包含块：按钮必须浮在滚动视口上而不是
          钉进内容坐标（直接给 .chat-messages 加 position:relative 会随滚动消失，
          原理见 styles/chat-messages.css 的同名规则注释） */}
      <div className="chat-messages-wrap">
        <div
          className="chat-messages"
          ref={scrollRef}
          onScroll={() => {
            onScroll()
          }}
        >
          {messages.length === 0 ? (
            <WelcomeScreen
              onSend={onSend}
              startupStats={startupStats}
              onResume={onResumeLast ?? onChatReplaced}
            />
          ) : (
            <div className="chat-messages-inner">
              {(() => {
                return (
                  <>
                    {messages.map((msg, idx) => {
                      // Refine 折叠：未展开的 refine 前历史消息跳过渲染（小巧折叠条代替），
                      // 解决多轮 refine 后前端历史气泡累积卡顿。
                      if (collapsedBeforeIds.has(idx)) return null
                      // Refine message → render SessionDivider, skip normal message-row
                      if (msg.role === 'refine') {
                        const seg = refineSegments.find(s => s.refineId === msg.id)
                        const segExpanded = expandedRefineIds.has(msg.id)
                        return (
                          <React.Fragment key={msg.id}>
                            {seg && seg.count > 0 && !segExpanded && !seg.streaming && (
                              <RefineCollapseBar
                                count={seg.count}
                                expanded={false}
                                onToggle={() => toggleRefineExpand(msg.id)}
                              />
                            )}
                            <SessionDivider
                              summary={msg.refineStatus === 'completed' ? msg.content : ''}
                              messageCount={msg.messageCount ?? 0}
                              sessionId={msg.sessionId ?? ''}
                              anchorId={msg.id}
                              streamingContent={
                                msg.refineStatus === 'streaming' ? msg.content || null : null
                              }
                            />
                          </React.Fragment>
                        )
                      }
                      const isCurrentAgent = msg.role === 'assistant' && idx === messages.length - 1
                      // Avatar settings
                      const showAvatar = localStorage.getItem('nuphus_show_avatar') === 'true'
                      // 是否配了皮肤背景：气泡底色（--msg-*-bg）是不透明实色，背景图下
                      // 会盖住图；`with-skin` 让气泡改用 glass-bg 半透明，让背景透出来
                      // （见 chat-messages.css 的 .message-bubble.with-skin）。仅此一处消费。
                      const skinBg = localStorage.getItem('nuphus_skin_bg') || ''

                      // 自定义头像的**可渲染 URL** 由顶层预解析（avatarUrls）—— 异步，
                      // 不能在消息渲染热路径里同步取；无自定义头像时用字母头像
                      const AvatarComp =
                        msg.role === 'user' ? (
                          avatarUrls.user ? (
                            <img src={avatarUrls.user} alt="" className="msg-avatar-img" />
                          ) : (
                            <LetterAvatar letter="U" size={36} />
                          )
                        ) : avatarUrls.nuphus ? (
                          <img src={avatarUrls.nuphus} alt="" className="msg-avatar-img" />
                        ) : (
                          <LetterAvatar letter="A" size={36} />
                        )

                      return (
                        <React.Fragment key={`row-${msg.id}`}>
                          <div
                            key={msg.id}
                            id={msg.role === 'user' ? `turn-anchor-${msg.id}` : undefined}
                            className={`message-row ${msg.role} ${showAvatar ? 'with-avatar' : ''}`}
                          >
                            {msg.role === 'assistant' && showAvatar && (
                              <div className="message-avatar">{AvatarComp}</div>
                            )}
                            <div
                              className={`message-bubble ${msg.role} ${showAvatar ? 'with-avatar' : ''} ${skinBg ? 'with-skin' : ''}`}
                            >
                              <div className="message-header">
                                <span className={`message-label ${msg.role}`}>
                                  {msg.role === 'user' ? (
                                    relation.userLabel
                                  ) : msg.role === 'assistant' ? (
                                    relation.assistantName
                                  ) : (
                                    <span style={{ color: 'var(--warning)' }}>
                                      {t('chat.systemLabel')}
                                    </span>
                                  )}
                                </span>
                                {msg.sourceLabel && (
                                  <span
                                    className="message-source-badge"
                                    title={`来自插件 ${msg.sourceLabel}`}
                                  >
                                    {msg.sourceLabel}
                                  </span>
                                )}
                                <span className="message-time">
                                  {new Date(msg.timestamp).toLocaleTimeString()}
                                </span>
                              </div>
                              <div className={`message-content ${msg.role}`}>
                                {/* ── 图片附件 ── */}
                                {msg.images && msg.images.length > 0 && (
                                  <div className="msg-images">
                                    {msg.images.map((img, i) => (
                                      <img
                                        key={i}
                                        src={img}
                                        alt={`图片 ${i + 1}`}
                                        className="msg-image"
                                        onClick={() => setLightboxUrl(img)}
                                        onError={e => {
                                          ;(e.target as HTMLImageElement).style.display = 'none'
                                        }}
                                      />
                                    ))}
                                  </div>
                                )}
                                {/* ── 内容引用（quote/skill/knowledge/workflow）──
                                    正文上方展示，先看引用的哪段、再看提问，符合阅读顺序。
                                    quote 只截断显示、全文进 title：引文上限 2000 字符，
                                    在气泡内铺全文会撑爆布局（发送时后端注入的是 label 全文）。 */}
                                {msg.references &&
                                  msg.references.some(r => r.type !== 'capture') && (
                                    <div className="msg-refs">
                                      {msg.references
                                        .filter(r => r.type !== 'capture')
                                        .map((r, i) => {
                                          const short =
                                            r.label.length > 60
                                              ? r.label.slice(0, 60) + '…'
                                              : r.label
                                          return (
                                            <span
                                              key={`ref-${r.type}-${r.id}-${i}`}
                                              className={`msg-ref-chip msg-ref-chip--${r.type}`}
                                              title={r.label}
                                            >
                                              <span
                                                className="msg-ref-chip-icon"
                                                aria-hidden="true"
                                              >
                                                <RefIcon type={r.type} size={12} />
                                              </span>
                                              <span className="msg-ref-chip-label">{short}</span>
                                            </span>
                                          )
                                        })}
                                    </div>
                                  )}
                                {/* ── 截图引用（桌面工具条截图：本地文件路径经 asset 协议显示）── */}
                                {msg.references &&
                                  msg.references.some(r => r.type === 'capture') && (
                                    <div className="msg-images">
                                      {msg.references
                                        .filter(r => r.type === 'capture')
                                        .map((r, i) => {
                                          const src = r.meta?.base64 || toAssetUrl(r.id)
                                          if (!src) return null
                                          return (
                                            <img
                                              key={`cap-${i}`}
                                              src={src}
                                              alt={r.label || `截图 ${i + 1}`}
                                              className="msg-image"
                                              onClick={() => setLightboxUrl(src)}
                                              onError={e => {
                                                ;(e.target as HTMLImageElement).style.display =
                                                  'none'
                                              }}
                                            />
                                          )
                                        })}
                                    </div>
                                  )}
                                {/* ── 音频附件 ── */}
                                {msg.audio && msg.audio.length > 0 && (
                                  <div className="msg-audio-list">
                                    {msg.audio.map((aud, i) => (
                                      <audio
                                        key={i}
                                        controls
                                        className="msg-audio"
                                        preload="metadata"
                                      >
                                        <source src={aud} />
                                      </audio>
                                    ))}
                                  </div>
                                )}
                                {/* ── 文本内容 ── */}
                                {msg.role === 'assistant' ? (
                                  (() => {
                                    // Normal assistant message
                                    return isCurrentAgent && isProcessing ? (
                                      msg.content ? (
                                        <>
                                          <MarkdownContent
                                            content={msg.content}
                                            onFileClick={setPreviewPath}
                                            onUrlClick={openUrl}
                                          />
                                          <span className="message-thinking-cursor" />
                                        </>
                                      ) : (
                                        <span className="message-thinking-dots">
                                          <span className="mtd" />
                                          <span className="mtd" />
                                          <span className="mtd" />
                                        </span>
                                      )
                                    ) : (
                                      <MarkdownContent
                                        content={msg.content}
                                        onFileClick={setPreviewPath}
                                        onUrlClick={openUrl}
                                      />
                                    )
                                  })()
                                ) : msg.content ? (
                                  <span className="msg-plain-text">{msg.content}</span>
                                ) : null}
                              </div>
                              {msg.role === 'assistant' && (
                                <div className="message-footer-row">
                                  {/* ── 本轮元数据（耗时 / 令牌 / 步数）──
                                       常驻：不随 hover——执行中也能看到耗时/步数实时在涨。
                                       数据源 = ChatMessage.meta（后端 TurnMeta）：实时轮次由
                                       useEvents 写入、历史由 applyHistory 还原。
                                       空数据由组件判空后不渲染，不留空位。 */}
                                  <TurnMetaBar
                                    meta={msg.meta}
                                    activeTool={activeTool}
                                    className="turn-meta-bar--actions"
                                  />
                                  {/* 动作按钮（复制/点评/执行回溯）跟在执行数据后，同行显示
                                      （2026-10-24 大王定；执行完成才显示） */}
                                  {!(isCurrentAgent && isProcessing) && (
                                    <div className="message-actions">
                                      <IconButton
                                        variant="msg-action"
                                        label="复制"
                                        title={copiedMsgId === msg.id ? '已复制' : '复制内容'}
                                        onClick={() => handleCopy(msg.id, msg.content)}
                                      >
                                        {copiedMsgId === msg.id ? (
                                          <IconCheck size={14} />
                                        ) : (
                                          <IconCopy size={14} />
                                        )}
                                      </IconButton>
                                      <IconButton
                                        variant="msg-action"
                                        label="点评"
                                        title="点评"
                                        onClick={() => {
                                          const userMsg =
                                            [...messages.slice(0, idx)]
                                              .reverse()
                                              .find(m => m.role === 'user')?.content ?? ''
                                          setRatingMsg({
                                            id: msg.id,
                                            content: msg.content,
                                            userQuestion: userMsg,
                                          })
                                        }}
                                      >
                                        <IconStar size={14} />
                                      </IconButton>
                                      {msg.traceItems && msg.traceItems.length > 0 && (
                                        <IconButton
                                          variant="msg-action"
                                          label="执行回溯"
                                          title="查看该轮执行过程"
                                          onClick={() => onShowExecTrace?.(msg.traceItems!)}
                                        >
                                          <IconHistory size={14} />
                                        </IconButton>
                                      )}
                                    </div>
                                  )}
                                </div>
                              )}
                              {/* user 消息首轮 LLM 失败：hover 显示重试（优雅停止气泡无此标记） */}
                              {msg.role === 'user' && msg.failed && !isProcessing && onRetry && (
                                <div className="message-retry-user">
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => handleRetry(msg)}
                                  >
                                    {t('chat.retry')}
                                  </Button>
                                </div>
                              )}
                            </div>
                            {msg.role === 'user' && showAvatar && (
                              <div className="message-avatar user-side">{AvatarComp}</div>
                            )}
                          </div>
                        </React.Fragment>
                      )
                    })}
                  </>
                )
              })()}
            </div>
          )}
          {/* ── 回底按钮：冻结跟随时显示，点击立即滚底（定位几何见 .chat-messages-wrap）── */}
          {showJumpButton && (
            <button
              type="button"
              className="chat-jump-bottom"
              onClick={jumpToBottom}
              aria-label={t('chat.jumpToBottom')}
              title={t('chat.jumpToBottom')}
            >
              <IconChevronsDown size={16} />
            </button>
          )}
          {/* ── 轮次轨：消息流 ≥2 轮时渲染（组件内部控制），左侧 20px / 距底 188px，
               定位几何与显隐由 turn-rail.css 承担；锚点 = user 行的 turn-anchor id ── */}
          <TurnRail messages={messages} scrollRef={scrollRef} t={t} />
        </div>
      </div>

      {/* ── Pause Modal ── */}
      <PauseOverlay
        pauseState={showPauseLocal ? { actionId: 'local' } : (pauseState ?? null)}
        pauseMode={pauseMode}
        pauseActionBusy={pauseActionBusy}
        selectedOption={selectedOption}
        appendInput={appendInput}
        pauseSubmitting={pauseSubmitting}
        onPauseChoice={handlePauseChoice}
        onAppendInputChange={setAppendInput}
        onBackToMenu={() => setPauseMode('menu')}
        onSubmitAppend={handleSubmitAppend}
      />

      {/* ── Refine Modal ── */}
      {refineState &&
        createPortal(
          <div className="compact-overlay compact-overlay--high">
            <div
              className="compact-modal compact-modal--sm compact-modal--fit"
              onClick={e => e.stopPropagation()}
            >
              <div className="compact-header">
                <span className="compact-header-title">{t('refine.title')}</span>
                {/* 提炼中允许关闭：后端失败（key 失效/连不上）不再广播结束事件时，
                    用户不会被全屏弹窗困死（后台提炼继续，完成后照常落地） */}
                {refining && (
                  <button
                    className="compact-header-close"
                    title={t('refine.dismissHint')}
                    aria-label={t('refine.dismissHint')}
                    onClick={() => (onDismissRefine ? onDismissRefine() : setRefining?.(false))}
                  >
                    <IconX size={14} strokeWidth={2} />
                  </button>
                )}
              </div>
              <div className="compact-divider" />
              <div className="compact-body">
                {!refining ? (
                  <>
                    <div className="refine-modal-head">
                      <span className="badge badge-warning">{t('refine.suggest')}</span>
                      <span className="refine-usage-pct">
                        {Math.round(refineState.usagePercent)}%
                      </span>
                    </div>
                    <div className="item-desc">{t('refine.desc')}</div>
                    <div className="item-sub item-sub--mono refine-usage">
                      {t(
                        'refine.usage',
                        String(Math.round(refineState.usagePercent)),
                        formatTokens(
                          Math.round((refineState.totalLimit * refineState.usagePercent) / 100),
                        ),
                        formatTokens(refineState.totalLimit),
                      )}
                    </div>
                    <div
                      style={{
                        height: 3,
                        background: 'var(--glass-2)',
                        borderRadius: 2,
                        overflow: 'hidden',
                        marginBottom: 12,
                      }}
                    >
                      <div
                        style={{
                          height: '100%',
                          width: `${Math.min(refineState.usagePercent, 100)}%`,
                          background:
                            refineState.usagePercent > 80
                              ? 'var(--error)'
                              : refineState.usagePercent > 60
                                ? 'var(--warning)'
                                : 'var(--accent)',
                          borderRadius: 2,
                          transition: 'width .3s ease',
                        }}
                      />
                    </div>
                    {/* 主弹窗刻意不给强制线 slider（2026-10-08 大王定案）。
                        这个弹窗出现在「刚触发最低阈值提炼」的当口，用户此刻的目标
                        是看清发生了什么并决定是否继续，滑杆会把注意力从提示上拽走。
                        slider 属于「调参」行为，只在 pending 弹窗（右上角「可提炼 N%」
                        徽标 → 确认框）里给——那是用户主动逼近阈值、想自定义线位的地方。 */}
                  </>
                ) : (
                  <div
                    style={{
                      display: 'flex',
                      flexDirection: 'column',
                      alignItems: 'center',
                      gap: 12,
                      padding: '24px 0',
                    }}
                  >
                    <div
                      style={{
                        width: 28,
                        height: 28,
                        border: '3px solid var(--glass-1)',
                        borderTopColor: 'var(--accent)',
                        borderRadius: '50%',
                        animation: 'spin .8s linear infinite',
                      }}
                    />
                    <div style={{ fontSize: 14, color: 'var(--spark-primary)', fontWeight: 500 }}>
                      {t('refine.processing')}
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--spark-muted)' }}>
                      {t('refine.processingDesc')}
                    </div>
                  </div>
                )}
                {!refining && (
                  <div
                    style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 8 }}
                  >
                    <div
                      onClick={() => {
                        if (!refining) {
                          setRefining?.(true)
                          onRefine?.()
                        }
                      }}
                      className={refining ? 'refine-option refining' : 'refine-option'}
                      style={{
                        background: refineSelected === 0 ? 'var(--void-hover)' : 'transparent',
                      }}
                    >
                      <span
                        style={{
                          fontSize: 12,
                          color: 'var(--accent)',
                          width: 14,
                          flexShrink: 0,
                          fontFamily: 'var(--font-mono)',
                        }}
                      >
                        ▸
                      </span>
                      <div>
                        <div
                          style={{ fontSize: 13, color: 'var(--spark-primary)', fontWeight: 500 }}
                        >
                          {t('refine.action')}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--spark-muted)' }}>
                          {t('refine.processingAction')}
                        </div>
                      </div>
                    </div>
                    <div
                      onClick={() => onSkipRefine?.()}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        padding: '8px 10px',
                        borderRadius: 8,
                        cursor: 'pointer',
                        background: refineSelected === 1 ? 'var(--void-hover)' : 'transparent',
                      }}
                    >
                      <span
                        style={{
                          fontSize: 12,
                          color: 'var(--accent)',
                          width: 14,
                          flexShrink: 0,
                          fontFamily: 'var(--font-mono)',
                        }}
                      >
                        {refineSelected === 1 ? '▸' : ' '}
                      </span>
                      <div>
                        <div
                          style={{ fontSize: 13, color: 'var(--spark-primary)', fontWeight: 500 }}
                        >
                          {t('refine.skip')}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--spark-muted)' }}>
                          {t('refine.skipDesc')}
                        </div>
                      </div>
                    </div>
                  </div>
                )}
                <div
                  style={{
                    display: 'flex',
                    gap: 12,
                    fontSize: 10,
                    color: 'var(--spark-dim)',
                    fontFamily: 'var(--font-mono)',
                  }}
                >
                  <span>{refining ? t('refine.processing') : t('refine.hintSelect')}</span>
                  <span>{refining ? '' : t('refine.hintConfirm')}</span>
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* ── 提炼中全屏遮罩（refine-pending-btn 路径 / 弹窗已关闭但后端仍在提炼）──
          弹窗路径的提炼中状态内嵌在 refineState 弹窗里；pending 路径 refineState
          为 null，需独立遮罩。关闭按钮走 onDismissRefine（复位 UI + 提炼追踪
          refs）——后端失败/超时未发结束事件时可手动退出，不困在全屏遮罩里。 */}
      {refining &&
        !refineState &&
        !pendingRefine &&
        createPortal(
          <div className="compact-overlay compact-overlay--high">
            <div
              className="compact-modal compact-modal--sm compact-modal--fit"
              onClick={e => e.stopPropagation()}
            >
              <div className="compact-header">
                <span className="compact-header-title">{t('refine.title')}</span>
                <button
                  className="compact-header-close"
                  title={t('refine.dismissHint')}
                  aria-label={t('refine.dismissHint')}
                  onClick={() => (onDismissRefine ? onDismissRefine() : setRefining?.(false))}
                >
                  <IconX size={14} strokeWidth={2} />
                </button>
              </div>
              <div className="compact-divider" />
              <div className="compact-body">
                <div className="refine-processing-box">
                  <span className="refine-spinner" aria-hidden="true" />
                  <span className="refine-processing-text">{t('refine.processing')}</span>
                </div>
                <div className="item-sub item-sub--mono">{t('refine.processingDesc')}</div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* 新手引导弹窗 */}
      {showOnboarding && (
        <OnboardingModal
          onComplete={() => {
            setShowOnboarding(false)
            loadSavedConfigs().then(setSavedConfigs)
          }}
          onSkip={() => {
            setShowOnboarding(false)
            hudUpdate(modelSetupHint(mode ?? '', formatPrimaryShortcut('K')), 'warning')
          }}
        />
      )}

      {/* 输入区 dock：badge 与输入框共享同一定位几何，保证左边缘对齐 */}
      <div className="chat-input-dock">
        <VideoProgressBadge />
        {/* 外部 Agent 悬浮胶囊：输入框外层右上角（absolute 定位，不占文档流） */}
        <ExternalAgentsStatusBar
          onOpenConfig={onManageExternalAgents}
          onNotice={onExternalAgentNotice}
        />
        <ChatInputBar
          input={input}
          onInputChange={handleInputChange}
          onInputKeyDown={handleKeyDown}
          textareaRef={textareaRef}
          imageInputRef={fileInputRef}
          executionStage={executionStage}
          pauseState={pauseState ?? null}
          refineState={refineState ?? null}
          refining={refining}
          tokenUsage={tokenUsage || null}
          mainTokenUsage={mainTokenUsage || null}
          execTokenUsage={execTokenUsage || null}
          totalDurationMs={totalDurationMs}
          totalCalls={totalCalls}
          turnMeta={turnMeta}
          mood={mood || 'idle'}
          contextLimit={contextLimit}
          apiHealth={apiHealth}
          onApiHealthRead={onApiHealthRead}
          security={security ?? null}
          onApproveSecurity={onApproveSecurity}
          onRejectSecurity={onRejectSecurity}
          mode={mode}
          onSetMode={onSetMode}
          onManageCustomAgents={onManageCustomAgents}
          onToggleWorkAgentMode={onToggleWorkAgentMode}
          modelLabel={modelLabel}
          modelName={modelName}
          effort={effort}
          supportedEfforts={currentModelEfforts}
          defaultEffort={currentModelDefaultEffort}
          onEffortChange={handleEffortChange}
          onModelSwitch={() => {
            setModelOpen(true)
            // 打开「模型管理」时强制拉取最新模型/配置：后端可能刚被「模型设置」页改过
            //（保存密钥、切 provider），而本组件只在 mode/modelName 变化时才重载——
            // 「同 id 换 provider」（如 custom→local 都是 qwen38-27b-q8）两边都不变，
            // 卡片就会显示旧快照（曾表现为 Local 卡片显示 0 个模型）。
            void loadEffortContext()
            loadSavedConfigs().then(setSavedConfigs)
          }}
          onSend={handleSubmit}
          onInterrupt={onInterrupt}
          onGracefulStop={onGracefulStop}
          appendQueue={appendQueue}
          isWorkflowRunning={isWorkflowRunning}
          toolPermissions={toolPermissions}
          onFileSelect={handleFileSelect}
          onImageAttach={handleImageAttach}
          onFileAttach={handleFileAttach}
          projectDir={projectDir}
          onOpenPrinciples={onOpenPrinciples}
          onOpenAnnotations={onOpenAnnotations}
          hints={HINTS}
          hintIndex={hintIndex}
          hintFade={hintFade}
          pendingReferences={pendingReferences}
          pendingImages={pendingImages}
          pendingFiles={pendingFiles}
          onRemoveReference={removeReference}
          onRemoveImage={removePendingImage}
          onRemoveFile={removePendingFile}
          onPreviewFile={setPreviewPath}
        />
      </div>

      {/* ── Skills Manager Modal ── */}
      {skillsOpen &&
        createPortal(
          <div className="cmd-modal-overlay" onClick={() => setSkillsOpen(false)}>
            <div className="cmd-modal" onClick={e => e.stopPropagation()}>
              <div className="cmd-modal-header">
                <span className="cmd-modal-icon">◆</span>
                <span className="cmd-modal-title">Skills Manager</span>
                <IconButton variant="modal-close" label="关闭" onClick={() => setSkillsOpen(false)}>
                  <IconX size={14} />
                </IconButton>
              </div>
              <div className="cmd-modal-body">
                <div className="cmd-modal-section">
                  <div className="cmd-modal-subtitle">Installed Skills</div>
                  <div className="cmd-modal-empty">
                    No skill packages installed yet.
                    <div className="cmd-modal-empty-hint">
                      Skills extend Nuphus capabilities with domain-specific knowledge.
                    </div>
                  </div>
                </div>
                <div className="cmd-modal-section">
                  <div className="cmd-modal-subtitle">Install New Skill</div>
                  <div className="cmd-modal-input-row">
                    <input className="cmd-modal-input" placeholder="Skill name or git URL..." />
                    <Button variant="primary" size="sm">
                      Install
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* ── Model Manager Modal ── */}
      {modelOpen &&
        createPortal(
          <div className="cmd-modal-overlay" onClick={() => setModelOpen(false)}>
            <div
              ref={modelModalRef}
              className="cmd-modal cmd-modal-sm"
              onClick={e => e.stopPropagation()}
            >
              <div className="cmd-modal-header">
                <span className="cmd-modal-icon">⚙</span>
                <span className="cmd-modal-title">{t('modelManager.title')}</span>
                <IconButton variant="modal-close" label="关闭" onClick={() => setModelOpen(false)}>
                  <IconX size={14} />
                </IconButton>
              </div>
              <div className="cmd-modal-body">
                {modelSwitchError && (
                  <div className="cmd-item cmd-item--hint" style={{ color: 'var(--error)' }}>
                    {modelSwitchError}
                  </div>
                )}
                {savedConfigs.length === 0 ? (
                  <div className="cmd-modal-empty">
                    {t('modelManager.noConfigs')}
                    <div className="cmd-modal-empty-hint">{t('modelManager.noConfigsHint')}</div>
                  </div>
                ) : (
                  <div className="cmd-modal-list">
                    <div className="model-provider-picker">
                      <div className="model-provider-list">
                        {savedConfigs.map(cfg => {
                          // 子列表排序口径：收藏置顶 → 最近切换顺序 → 其余原序。
                          // 收藏（设置页卡片星形写的本地偏好）只影响顺序、不过滤——
                          // 没有收藏时行为与之前完全一致。均不参与模型解析/切换。
                          // 同一份集合同时驱动「名称后的收藏星标」，取一次即可。
                          const favorites = getProviderFavorites(localStorage, cfg.provider)
                          const providerModels = orderProviderModels(
                            allModels.filter(m => m.provider === cfg.provider),
                            readRecentModels(localStorage, cfg.provider),
                            favorites,
                          )
                          // 勾选态 = (provider, model) 双全等：官方厂商与 opencode-go 存在同 id
                          // 模型（deepseek-v4-flash 等），仅比 id 会让两 provider 卡片同时打勾。
                          // currentProvider 来自 get_provider_context（mode 感知的生效模型
                          // provider 归属，后端权威：内存 runtime → [last_model] → 候选回落），
                          // 与 modelLabel 同一 effective_model 解析点，两者天然同步。
                          const isActive =
                            !!currentProvider &&
                            cfg.provider === currentProvider &&
                            cfg.model === modelLabel
                          const isHovered = hoveredProvider === cfg.provider
                          return (
                            <div
                              key={cfg.provider}
                              className={`model-provider-option ${isHovered ? 'active' : ''}`}
                              style={
                                {
                                  '--provider-index': savedConfigs.indexOf(cfg),
                                } as React.CSSProperties
                              }
                              tabIndex={0}
                              aria-label={`${cfg.label} models`}
                              onMouseEnter={e => openProviderModels(cfg.provider, e.currentTarget)}
                              onMouseLeave={closeProviderModelsSoon}
                              onFocus={e => openProviderModels(cfg.provider, e.currentTarget)}
                              onBlur={closeProviderModelsSoon}
                            >
                              <span className="cmd-modal-provider-icon" aria-hidden="true">
                                {hasProviderIcon(cfg.provider) ? (
                                  <ProviderIcon provider={cfg.provider} size={18} />
                                ) : (
                                  (cfg.label || cfg.provider).charAt(0).toUpperCase()
                                )}
                              </span>
                              <span className="model-provider-option-body">
                                <span className="cmd-modal-card-name">{cfg.label}</span>
                                <span className="cmd-modal-card-meta">
                                  {/* 显示该 provider 最近使用的模型（localStorage 持久化）；
                                      仅当前生效项由右侧 ✓ 标识，不再用"暂无模型"掩盖其他项 */}
                                  {cfg.model || t('models.noModels')}
                                </span>
                              </span>
                              {isActive && (
                                <span className="cmd-modal-card-check" aria-label="当前模型">
                                  <IconCheck size={13} />
                                </span>
                              )}
                              {isHovered && (
                                <div
                                  className="model-provider-models"
                                  role="menu"
                                  aria-label={`${cfg.label} models`}
                                  style={
                                    providerMenuPosition
                                      ? {
                                          // top / bottom 二选一：同时设会把浮层拉成固定高度
                                          ...(providerMenuPosition.top != null
                                            ? { top: providerMenuPosition.top }
                                            : { bottom: providerMenuPosition.bottom }),
                                          left: providerMenuPosition.left,
                                          maxHeight: providerMenuPosition.maxHeight,
                                        }
                                      : undefined
                                  }
                                >
                                  <div className="model-provider-models-header">
                                    <span>{cfg.label}</span>
                                    <span>{providerModels.length}</span>
                                  </div>
                                  {providerModels.length === 0 ? (
                                    <div className="model-provider-empty">
                                      {t('models.noModels')}
                                    </div>
                                  ) : (
                                    providerModels.map(model => {
                                      // 同卡片：勾选须 provider+model 双全等，避免官方/GO 同 id
                                      // 模型在 hover 子菜单里互相打勾。currentProvider 空时保守不勾。
                                      const modelIsActive =
                                        !!currentProvider &&
                                        cfg.provider === currentProvider &&
                                        modelLabel === model.id
                                      return (
                                        <button
                                          key={model.id}
                                          type="button"
                                          className={`model-provider-model ${modelIsActive ? 'active' : ''} ${switchingId === `${cfg.provider}::${model.id}` ? 'switching' : ''}`}
                                          disabled={switchingId !== null}
                                          aria-busy={switchingId === `${cfg.provider}::${model.id}`}
                                          onClick={() =>
                                            switchConfig(
                                              {
                                                ...cfg,
                                                id: `${cfg.provider}::${model.id}`,
                                                model: model.id,
                                              },
                                              true,
                                            )
                                          }
                                        >
                                          <span className="model-provider-model-info">
                                            <span className="model-provider-model-name">
                                              {model.id}
                                              {cfg.provider === 'opencode-go' && (
                                                <span
                                                  className="model-go-badge"
                                                  title="OpenCode Go 网关"
                                                >
                                                  GO
                                                </span>
                                              )}
                                              {/* 收藏星标紧跟在名称后；没有收藏就不渲染
                                                  （不占位、不置灰——未收藏是默认态，
                                                  不需要一个空图标来「说明」它） */}
                                              {favorites?.includes(model.id) && (
                                                <span
                                                  className="model-popup-fav"
                                                  title={t('modelManager.favorited')}
                                                  aria-label={t('modelManager.favorited')}
                                                >
                                                  <IconStar size={10} />
                                                </span>
                                              )}
                                            </span>
                                            <span className="model-provider-model-meta">
                                              {model.supports_vision && (
                                                <span title="视觉能力" aria-label="视觉能力">
                                                  <IconEye size={11} /> 视觉
                                                </span>
                                              )}
                                              {model.supports_audio && (
                                                <span title="音频能力" aria-label="音频能力">
                                                  <IconMic size={11} /> 音频
                                                </span>
                                              )}
                                              {model.supports_image_generation && (
                                                <span title="图像生成" aria-label="图像生成">
                                                  <IconImage size={11} /> 图像
                                                </span>
                                              )}
                                              {model.supports_streaming && (
                                                <span title="流式输出" aria-label="流式输出">
                                                  <IconRadio size={11} /> 流式
                                                </span>
                                              )}
                                              <span
                                                className="model-provider-context"
                                                title={t('modelManager.unknownContext')}
                                              >
                                                {model.context_window
                                                  ? t(
                                                      'modelManager.contextUnit',
                                                      model.context_window.toLocaleString(),
                                                    )
                                                  : t('modelManager.unknownContext')}
                                              </span>
                                              {model.reasoning_efforts.length > 0 && (
                                                <span title="推理强度" aria-label="推理强度">
                                                  {t(
                                                    'modelManager.reasoning',
                                                    model.reasoning_efforts.join(' / '),
                                                  )}
                                                </span>
                                              )}
                                            </span>
                                          </span>
                                          {modelIsActive && (
                                            <span
                                              className="cmd-modal-card-check"
                                              aria-label="当前模型"
                                            >
                                              <IconCheck size={13} />
                                            </span>
                                          )}
                                        </button>
                                      )
                                    })
                                  )}
                                </div>
                              )}
                            </div>
                          )
                        })}
                      </div>
                    </div>
                  </div>
                )}
                <div className="cmd-modal-footer-hint">{t('modelManager.switchTip')}</div>
              </div>
            </div>
          </div>,
          document.body,
        )}

      {/* 空结果（输入框已有文字前加 "/" 无匹配）时不渲染——避免空 palette 的
          border/shadow 显示为长条黑块（用户实测） */}
      {cmdOpen &&
        filteredSlash.length > 0 &&
        createPortal(
          <div className="cmd-overlay" onClick={() => setCmdOpen(false)}>
            <div
              className="cmd-palette"
              onClick={e => e.stopPropagation()}
              onWheel={handleCommandWheel}
            >
              {filteredSlash.map((item, i) => (
                <div
                  key={item.id}
                  ref={el => {
                    cmdItemRefs.current[i] = el
                  }}
                  className={`cmd-item ${i === cmdIdx ? 'selected' : ''}`}
                  onClick={() => executeSlash(item.id)}
                  onMouseEnter={() => setCmdIdx(i)}
                >
                  {SLASH_ICONS[item.id] && <span className="cmd-icon">{SLASH_ICONS[item.id]}</span>}
                  <span className="cmd-label">{item.label}</span>
                  <span className="cmd-desc">{item.desc}</span>
                  {i === cmdIdx && <span className="cmd-arrow">↵</span>}
                </div>
              ))}
            </div>
          </div>,
          document.body,
        )}

      {/* ── Resource picker (skill/knowledge/workflow selection) ── */}
      {resPickerOpen &&
        createPortal(
          <div className="cmd-overlay" onClick={closeResourcePicker}>
            <div
              className="cmd-palette"
              onClick={e => e.stopPropagation()}
              onWheel={handleResourceWheel}
            >
              <div className="cmd-palette-header">
                <span className="cmd-palette-title">
                  {resPickerType === 'skill'
                    ? '选择 Skill'
                    : resPickerType === 'knowledge'
                      ? '选择知识库'
                      : '选择工作流'}
                </span>
              </div>
              {resLoading && <div className="cmd-item cmd-item--hint">加载中...</div>}
              {resError && (
                <div className="cmd-item cmd-item--hint" style={{ color: 'var(--error)' }}>
                  {resError}
                </div>
              )}
              {!resLoading && !resError && resItems.length === 0 && (
                <div className="cmd-item cmd-item--hint">无可用项</div>
              )}
              {!resLoading &&
                resItems.map((item, i) => (
                  <div
                    key={item.id}
                    ref={el => {
                      resItemRefs.current[i] = el
                    }}
                    className={`cmd-item ${i === resIdx ? 'selected' : ''}`}
                    onClick={() => selectResource(item)}
                    onMouseEnter={() => setResIdx(i)}
                  >
                    <span className="cmd-label">{item.label}</span>
                    {item.desc && <span className="cmd-desc">{item.desc}</span>}
                    {i === resIdx && <span className="cmd-arrow">↵</span>}
                  </div>
                ))}
            </div>
          </div>,
          document.body,
        )}

      {/* ── 点评弹窗 ── */}
      {ratingMsg && (
        <RatingModal
          goal={ratingMsg.content.slice(0, 80)}
          toolCalls={[]}
          totalMs={0}
          onClose={() => setRatingMsg(null)}
          onSubmit={(name, rating, comment, saveAsStrategy) => {
            onRate?.(
              name,
              rating,
              comment,
              saveAsStrategy,
              ratingMsg.userQuestion,
              ratingMsg.content,
            )
            setRatingMsg(null)
          }}
        />
      )}
      {/* Lightbox overlay for image click-to-zoom
          ⚠️ 必须 portal 到 body：留在 .chat-panel 内会被 .chat-area（z-index:1 的
          层叠上下文）整体压在 TitleBar（.app-shell 直接子级、DOM 靠后）之下——
          用户会看到「最小化/还原/关闭」浮在预览遮罩上，把应用关闭钮当成预览
          关闭钮点（真实事故）。portal 到根层后 z-index 才由本层真实生效。
          Esc 关闭：预览打开时唯一的活动覆盖层，键盘用户的退出路径。 */}
      {lightboxUrl &&
        createPortal(
          <div
            className="msg-lightbox-overlay"
            onClick={() => setLightboxUrl(null)}
            onKeyDown={e => {
              if (e.key === 'Escape') setLightboxUrl(null)
            }}
          >
            <button
              type="button"
              className="msg-lightbox-close"
              aria-label="关闭预览"
              title="关闭 (Esc)"
              autoFocus
              onClick={() => setLightboxUrl(null)}
            >
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                aria-hidden="true"
              >
                <path d="M18 6 6 18M6 6l12 12" />
              </svg>
            </button>
            <img
              src={lightboxUrl}
              alt="放大预览"
              className="msg-lightbox-image"
              onClick={e => e.stopPropagation()}
            />
          </div>,
          document.body,
        )}

      {/* ── 文件预览覆盖层（AI 回复路径点击，全屏对齐画布范式） ── */}
      {previewPath && (
        <PreviewOverlay
          path={previewPath}
          onClose={() => setPreviewPath(null)}
          onSendAnnotations={addQuoteReference}
        />
      )}
    </div>
  )
}
