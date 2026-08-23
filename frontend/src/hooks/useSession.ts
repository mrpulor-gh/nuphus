// useSession — 会话状态管理（组合器）
// 将状态分为：useModals / useExecutionUI / useAgentControl / useInit
// handleSend / handleNewChat 保留在此（紧密耦合 messages）

import { useState, useCallback, useEffect, useRef, useMemo } from 'react'
import { invoke } from '../core/bridge'
import type {
  ChatMessage,
  ChatReference,
  ToolSchema,
  TimelineEntry,
  PlanData,
  PlanTask,
  TaskStatus,
  TaskPriority,
  WorkflowRunStep,
} from '../core/types'
import type { MoodState } from '../ui/MoodFace'
import {
  processInput,
  isLlmConfigured,
  getContextLimit,
  getCurrentConfig,
  isBusy,
  getChatHistory,
} from '../main-window/lib/api'
import { foldHistoryAssistants, toTimelineEntry } from './useInit'
import { loadRelation } from '../main-window/lib/relation'
import { useLanguage } from '../locales'

import { useModals } from './useModals'
import { useExecutionUI } from './useExecutionUI'
import { useAgentControl } from './useAgentControl'
import { useInit } from './useInit'
import type { Toast } from './useInit'

export type { Toast }

// ════════════════════════════════════════════════════════════
// Types
// ════════════════════════════════════════════════════════════

type InitStatus = 'pending' | 'loading' | 'done' | 'error'

export interface SessionAPI {
  // ── App lifecycle ──
  appState: 'loading' | 'ready' | 'error'
  initError: { kind: string; message: string; detail?: string } | null
  initItems: { key: string; label: string; status: InitStatus }[]
  fadeOut: boolean
  startupStats: { tools: number; memories: number }
  setAppState: (v: 'loading' | 'ready' | 'error') => void
  setInitError: (v: { kind: string; message: string; detail?: string } | null) => void
  setInitItems: (v: { key: string; label: string; status: InitStatus }[]) => void
  setTools: (v: ToolSchema[]) => void
  setModelName: (v: string) => void
  setContextLimit: (v: number) => void

  // ── Toast ──
  showToast: (message: string, type?: Toast['type']) => void

  // ── Core state ──
  messages: ChatMessage[]
  isProcessing: boolean
  status: 'idle' | 'running' | 'error'
  modelName: string
  sessionId: string
  setSessionId: (v: string) => void
  tools: ToolSchema[]

  // ── Mode ──
  mode: string
  /** useEvents 的 mode_changed 广播回调（手机端 /switch-mode 同步桌面输入框 mode） */
  setMode: (v: string) => void

  // ── AI Mood ──
  mood: MoodState
  setMood: (m: MoodState) => void

  // ── Modal navigation ──
  showWorkflow: boolean
  setShowWorkflow: (v: boolean) => void
  showMemories: boolean
  setShowMemories: (v: boolean) => void
  showSkills: boolean
  setShowSkills: (v: boolean) => void
  showKnowledge: boolean
  setShowKnowledge: (v: boolean) => void
  showThemes: boolean
  setShowThemes: (v: boolean) => void
  showProject: boolean
  setShowProject: (v: boolean) => void
  showSecurity: boolean
  setShowSecurity: (v: boolean) => void
  showBrowser: boolean
  setShowBrowser: (v: boolean) => void
  showSoul: boolean
  setShowSoul: (v: boolean) => void
  showModels: boolean
  setShowModels: (v: boolean) => void
  showMcp: boolean
  setShowMcp: (v: boolean) => void
  showHelp: boolean
  setShowHelp: (v: boolean) => void
  showSnakeGame: boolean
  setShowSnakeGame: (v: boolean) => void
  showMobile: boolean
  setShowMobile: (v: boolean) => void
  showCustomAgents: boolean
  setShowCustomAgents: (v: boolean) => void
  showExternalAgents: boolean
  setShowExternalAgents: (v: boolean) => void
  showPlugins: boolean
  setShowPlugins: (v: boolean) => void
  showPluginDev: boolean
  setShowPluginDev: (v: boolean) => void

  // ── Execution ──
  showExecTrace: boolean
  setShowExecTrace: (v: boolean) => void
  execTraceOverride: TimelineEntry[] | null
  setExecTraceOverride: (v: TimelineEntry[] | null) => void
  dismissThinking: boolean
  setDismissThinking: (v: boolean) => void
  stepIndex: number
  goal: string
  progress: { iteration: number; max: number; calls: number }
  execPhase: 'understanding' | 'executing' | 'recording' | 'workflow' | 'retrying' | ''
  security: import('../core/types').SecurityCheck | null
  userInputRequest: import('../core/types').UserInputRequest | null
  pauseState: { actionId: string } | null
  completed: boolean
  timeline: TimelineEntry[]
  goalType: { type: string; label: string; confidence: number } | null

  // ── Workflow run ──
  workflowRunSteps: WorkflowRunStep[]
  setWorkflowRunSteps: (
    v: WorkflowRunStep[] | ((prev: WorkflowRunStep[]) => WorkflowRunStep[]),
  ) => void
  workflowRunId: string | null
  setWorkflowRunId: (v: string | null) => void
  /** 最近一次运行的 workflow id（run 结束后保留，供步骤参数查看） */
  lastWorkflowId: string | null
  isWorkflowPaused: boolean
  handleWfPause: () => Promise<void>
  handleWfResume: () => Promise<void>
  handleWorkflowPermCancel: () => void
  handleWorkflowPermConfirm: () => Promise<void>
  handleWorkflowExitCancel: () => void
  handleWorkflowExitConfirm: () => Promise<void>
  showWorkflowPermConfirm: boolean
  setShowWorkflowPermConfirm: (v: boolean) => void
  showWorkflowExitConfirm: boolean
  setShowWorkflowExitConfirm: (v: boolean) => void
  hasWorkflowActivity: boolean
  setHasWorkflowActivity: (v: boolean) => void

  // ── Token ──
  mainTokenUsage: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  execTokenUsage: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  totalDurationMs: number
  totalCalls: number
  contextLimit: number
  currentTaskDesc: string

  // ── Setters (for useEvents) ──
  setMessages: (v: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void
  setIsProcessing: (v: boolean) => void
  setCompleted: (v: boolean) => void
  setStepIndex: React.Dispatch<React.SetStateAction<number>>
  setGoal: (v: string) => void
  setProgress: (
    v:
      | { iteration: number; max: number; calls: number }
      | ((prev: { iteration: number; max: number; calls: number }) => {
          iteration: number
          max: number
          calls: number
        }),
  ) => void
  setExecPhase: React.Dispatch<
    React.SetStateAction<'understanding' | 'executing' | 'recording' | 'workflow' | 'retrying' | ''>
  >
  setSecurity: React.Dispatch<React.SetStateAction<import('../core/types').SecurityCheck | null>>
  setUserInputRequest: (v: import('../core/types').UserInputRequest | null) => void
  setPauseState: React.Dispatch<React.SetStateAction<{ actionId: string } | null>>
  setTimeline: (v: TimelineEntry[] | ((prev: TimelineEntry[]) => TimelineEntry[])) => void
  setGoalType: React.Dispatch<
    React.SetStateAction<{ type: string; label: string; confidence: number } | null>
  >
  setMainTokenUsage: React.Dispatch<
    React.SetStateAction<{
      inputTokens: number
      outputTokens: number
      cacheHitTokens: number
    } | null>
  >
  setExecTokenUsage: React.Dispatch<
    React.SetStateAction<{
      inputTokens: number
      outputTokens: number
      cacheHitTokens: number
    } | null>
  >
  setTotalDurationMs: (v: number) => void
  setTotalCalls: (v: number) => void
  executionCounter: number
  setExecutionCounter: React.Dispatch<React.SetStateAction<number>>
  setCurrentTaskDesc: (v: string) => void

  // ── Refine ──
  refineState: { usagePercent: number; totalLimit: number } | null
  setRefineState: React.Dispatch<
    React.SetStateAction<{ usagePercent: number; totalLimit: number } | null>
  >
  refining: boolean
  setRefining: (v: boolean) => void
  pendingRefine: { usagePercent: number; totalLimit: number; skippedTurns: number } | null
  setPendingRefine: React.Dispatch<
    React.SetStateAction<{
      usagePercent: number
      totalLimit: number
      skippedTurns: number
    } | null>
  >

  // ── Planner / Approval / TaskBubble ──
  showPlannerModal: boolean
  planData: PlanData | null
  showReview: boolean
  approvalState: {
    open: boolean
    kind: string
    title: string
    content: string
    actionId: string
    tenetCount: number
  }
  taskBubbleVisible: boolean
  setShowPlannerModal: (v: boolean) => void
  setPlanData: React.Dispatch<React.SetStateAction<PlanData | null>>
  setShowReview: (v: boolean) => void
  setApprovalState: React.Dispatch<
    React.SetStateAction<{
      open: boolean
      kind: string
      title: string
      content: string
      actionId: string
      tenetCount: number
    }>
  >
  setTaskBubbleVisible: (v: boolean) => void
  transitionTask: (taskId: number, status: TaskStatus) => void
  setTaskPriority: (taskId: number, priority: TaskPriority) => void
  reorderTask: (taskId: number, direction: -1 | 1) => void

  // ── Command palette & keyboard ──
  cmdPaletteOpen: boolean
  focusSignal: number
  showDesktopToolbar: boolean
  regionPickerMode: 'picker' | 'capture' | 'ocr' | null
  expandedCalls: Set<string>
  setCmdPaletteOpen: (v: boolean | ((prev: boolean) => boolean)) => void
  setFocusSignal: (v: number | ((prev: number) => number)) => void
  setShowDesktopToolbar: (v: boolean | ((prev: boolean) => boolean)) => void
  setRegionPickerMode: (v: 'picker' | 'capture' | 'ocr' | null) => void
  setExpandedCalls: (v: Set<string> | ((prev: Set<string>) => Set<string>)) => void

  // ── Refs (for useEvents) ──
  refs: {
    streamingMsgId: React.MutableRefObject<string | null>
    lastStreamingMsgId: React.MutableRefObject<string | null>
    executionActiveRef: React.MutableRefObject<boolean>
    processingRef: React.MutableRefObject<boolean>
    lastSentRef: React.MutableRefObject<{ content: string; time: number } | null>
    sendSeqRef: React.MutableRefObject<number>
    messagesRef: React.MutableRefObject<ChatMessage[]>
    toolCallCountRef: React.MutableRefObject<number>
    messagesRestoredRef: React.MutableRefObject<boolean>
  }

  // ── Computed ──
  cmdItems: { id: string; label: string; desc: string; category: string; action: () => void }[]
  thinkingStep: string
  displayTokenUsage: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  liveCalls: number

  // ── Handlers ──
  handleSend: (
    input: string,
    images?: string[],
    forceMode?: string,
    refs?: import('../core/types').ChatReference[],
  ) => Promise<void>
  handleNewChat: () => void
  reloadChatFromBackend: () => Promise<void>
  handleRetryAgent: (input: string) => Promise<void>
  handlePause: () => Promise<void>
  handleContinue: (actionId: string) => Promise<void>
  handleInterrupt: () => Promise<void>
  handleGracefulStop: () => Promise<void>
  handleAppendInstruction: (actionId: string, instruction: string) => Promise<void>
  handleTerminate: (actionId: string) => Promise<void>
  handleSetMode: (newMode: string) => Promise<void>
  toggleWorkAgentMode: () => Promise<void>
  forceReset: () => Promise<void>
  handleRefine: () => Promise<void>
  handleSkipRefine: () => void
  handleRate: (
    name: string,
    rating: number,
    comment: string,
    saveAsStrategy: boolean,
    userQuestion?: string,
    assistantContent?: string,
  ) => Promise<void>
  toggleExpand: (id: string) => void
  addMessage: (msg: ChatMessage) => void

  runInitialization: () => Promise<void>
  refreshModelInfo: () => Promise<void>
}

// ════════════════════════════════════════════════════════════
// Hook
// ════════════════════════════════════════════════════════════

export function useSession(): SessionAPI {
  const { t } = useLanguage()

  // ── Core state ──
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [isProcessing, setIsProcessing] = useState(false)
  const [status, setStatus] = useState<'idle' | 'running' | 'error'>('idle')
  const [modelName, setModelName] = useState('')
  const [sessionId, setSessionId] = useState('')
  const [tools, setTools] = useState<ToolSchema[]>([])
  const [currentQuery, setCurrentQuery] = useState('')

  const [mode, setModeState] = useState('leader')

  // ── AI Mood ──
  const [mood, setMood] = useState<MoodState>('idle')

  // ── Refs ──
  const streamingMsgId = useRef<string | null>(null)
  const lastStreamingMsgId = useRef<string | null>(null)
  const executionActiveRef = useRef(false)
  const messagesRestoredRef = useRef(false)
  const processingRef = useRef(false)
  const lastSentRef = useRef<{ content: string; time: number } | null>(null)
  const sendSeqRef = useRef(0)
  const messagesRef = useRef<ChatMessage[]>(messages)
  messagesRef.current = messages
  const toolCallCountRef = useRef(0)

  const [expandedCalls, setExpandedCalls] = useState<Set<string>>(new Set())

  // ── Sub-hooks ──
  const modals = useModals()

  const {
    showToast,
    appState,
    initError,
    initItems,
    fadeOut,
    startupStats,
    setAppState,
    setInitError,
    setInitItems,
    runInitialization,
  } = useInit({
    setMessages,
    setModelName,
    setSessionId,
    messagesRestoredRef,
  })

  const execUI = useExecutionUI(showToast)

  // 重试时移除失败回合的错误气泡（assistant 含「LLM请求失败」/ system 以「错误」开头）
  const removeRetryErrorBubble = useCallback(() => {
    setMessages(prev => {
      const last = prev[prev.length - 1]
      const isErr =
        (last?.role === 'assistant' && last.content.includes('LLM请求失败')) ||
        (last?.role === 'system' && last.content.startsWith('错误'))
      return isErr ? prev.slice(0, -1) : prev
    })
  }, [])

  const agentControl = useAgentControl({
    isProcessing,
    sessionId,
    mode,
    execPhase: execUI.execPhase,
    timeline: execUI.timeline,
    workflowRunId: execUI.workflowRunId,
    hasWorkflowActivity: execUI.hasWorkflowActivity,
    setHasWorkflowActivity: execUI.setHasWorkflowActivity,
    setIsProcessing,
    setCompleted: execUI.setCompleted,
    setGoal: execUI.setGoal,
    setExecPhase: execUI.setExecPhase,
    setTimeline: execUI.setTimeline,
    setStepIndex: execUI.setStepIndex,
    setPlanData: execUI.setPlanData,
    setSecurity: execUI.setSecurity,
    setRefineState: execUI.setRefineState,
    setPendingRefine: execUI.setPendingRefine,
    setWorkflowRunSteps: execUI.setWorkflowRunSteps,
    setIsWorkflowPaused: execUI.setIsWorkflowPaused,
    setShowWorkflowPermConfirm: execUI.setShowWorkflowPermConfirm,
    setShowWorkflowExitConfirm: execUI.setShowWorkflowExitConfirm,
    setPauseState: execUI.setPauseState,
    setMode: setModeState,
    messagesRef,
    streamingMsgId,
    lastStreamingMsgId,
    executionActiveRef,
    showToast,
    setMood,
    removeRetryErrorBubble,
  })

  // ── handleSend (kept in useSession due to tight coupling with messages) ──
  const handleSend = useCallback(
    async (input: string, images?: string[], forceMode?: string, refs?: ChatReference[]) => {
      if (!(await agentControl.checkBackendReady())) return

      let backendBusy = true
      if (isProcessing) {
        // 执行中（后端 busy）发送 = 追加指令：不清执行态，交给后端入队注入。
        // 仅当后端已空闲（UI 残留 processing 状态）时才复位，避免新执行态被吞。
        try {
          backendBusy = (await isBusy()) ?? true
        } catch {
          backendBusy = true
        }
        if (!backendBusy) {
          setIsProcessing(false)
          executionActiveRef.current = false
        }
      }

      const configured = await isLlmConfigured()
      if (!configured) {
        invoke('hud_update', { text: 'Please configure API Key first', phase: 'error' })
        return
      }

      // 执行中（后端 busy）发送 = 追加指令：不创建独立 user 气泡。
      // 追加消息只入后端队列注入执行——显示新气泡会让前端以为要开启新回合，
      // 并覆盖 streamingMsgId 导致 agent 当前气泡瞬间封闭、最终回复丢失。
      const isAppendAttempt = isProcessing && backendBusy

      const msg: ChatMessage = {
        id: crypto.randomUUID(),
        role: 'user',
        content: input,
        images: images && images.length > 0 ? images : undefined,
        references: refs && refs.length > 0 ? refs : undefined,
        timestamp: Date.now(),
      }
      if (!isAppendAttempt) {
        setMessages(prev => [...prev, msg])
      }
      setCurrentQuery('')
      setIsProcessing(true)
      // Workflow 模式下发送消息 = WorkflowAgent 将执行内容，标记活动
      if ((forceMode || mode) === 'workflow') {
        execUI.setHasWorkflowActivity(true)
      }
      if (!isAppendAttempt) {
        execUI.setCompleted(false)
        execUI.setExecPhase('understanding')
        execUI.setTimeline([])
        execUI.setStepIndex(0)
        execUI.setPlanData(null)
        execUI.setGoal(input.slice(0, 120))
      }
      const sendId = crypto.randomUUID()
      // ⚠️ 追加指令（执行中发送）绝不覆盖 streamingMsgId：它指向正在流式的
      // agent 气泡，覆盖后 llm_text_delta / execution_completed 找不到目标，
      // agent 最终回复会凭空消失（气泡被"划开"）。仅新执行才重置。
      if (!isAppendAttempt) {
        streamingMsgId.current = sendId
      }
      let isAppend = false
      try {
        const history = messagesRef.current.map(m => ({ role: m.role, content: m.content }))
        const relation = loadRelation()
        const result = await processInput(
          input,
          history,
          relation,
          sendId,
          forceMode || mode,
          images,
          refs,
        )
        if (result === null) {
          invoke('hud_update', {
            text: 'Connection lost - please try again',
            phase: 'error',
          })
          return
        }
        if (result.appended) {
          // 执行中发送被接受为追加指令：不开启新执行、不清除执行态。
          // 追加消息不显示为独立气泡——撤销可能已 push 的 msg（前端 isProcessing
          // 与后端 busy 状态不同步时兜底），仅弹窗提示消息内容本身。
          isAppend = true
          setMessages(prev => prev.filter(m => m.id !== msg.id))
          showToast(result.message || input, 'info')
          return
        }
        // 图片降级警告：主模型与视觉模型都不支持视觉，图片已降级发送但 AI 无法查看。
        // 弹窗提示，不阻塞消息流（后端已正常处理）。
        if (result.image_warning) {
          showToast(result.image_warning, 'warning')
        }
        if (result.success === false) {
          invoke('hud_update', {
            text: result.message || '发送失败，请重试',
            phase: 'error',
          })
          return
        }
      } catch (e: any) {
        invoke('hud_update', { text: 'Request failed: ' + (e.message || e), phase: 'error' })
      } finally {
        // 追加指令不改变执行态（仍在执行中）；只有真正开启新执行才清理
        if (!isAppend) {
          setIsProcessing(false)
          streamingMsgId.current = null
          executionActiveRef.current = false
        }
      }
    },
    [mode, isProcessing, agentControl.checkBackendReady],
  )

  // ── handleNewChat (kept in useSession due to tight coupling with messages) ──
  const resetTransientUI = useCallback(() => {
    setMessages([])
    setIsProcessing(false)
    execUI.setCompleted(false)
    execUI.setGoal('')
    execUI.setTimeline([])
    execUI.setStepIndex(0)
    execUI.setExecPhase('')
    execUI.setPlanData(null)
    execUI.setSecurity(null)
    setCurrentQuery('')
    execUI.setLastWorkflowId(null)
    execUI.setHasWorkflowActivity(false)
    execUI.setRefineState(null)
    execUI.setPendingRefine(null)
    streamingMsgId.current = null
    lastStreamingMsgId.current = null
  }, [])

  const handleNewChat = useCallback(() => {
    resetTransientUI()
  }, [resetTransientUI])

  /**
   * Session Shelf 切换/新建后：从后端重拉当前会话历史并整体替换气泡。
   * 映射逻辑与 useInit 启动恢复完全一致（fold 连续 assistant + traceItems）。
   */
  const reloadChatFromBackend = useCallback(async () => {
    resetTransientUI()
    try {
      const history = await getChatHistory()
      if (history && history.length > 0) {
        const folded = foldHistoryAssistants(history)
        setMessages(
          folded.map(h => ({
            id: crypto.randomUUID(),
            role: h.role as ChatMessage['role'],
            content: h.content,
            images: h.images && h.images.length > 0 ? h.images : undefined,
            audio: h.audio && h.audio.length > 0 ? h.audio : undefined,
            ...(h.role === 'refine' ? { refineStatus: 'completed' as const } : {}),
            timestamp: h.timestamp ?? Date.now(),
            ...(h.traceItems && h.traceItems.length > 0
              ? { traceItems: h.traceItems.map(toTimelineEntry) }
              : {}),
          })),
        )
        messagesRestoredRef.current = true
      }
    } catch {}
  }, [resetTransientUI])

  // ── addMessage ──
  const addMessage = useCallback((msg: ChatMessage) => {
    setMessages(prev => [...prev, msg])
  }, [])

  // ── toggleExpand ──
  const toggleExpand = useCallback((id: string) => {
    setExpandedCalls(prev => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }, [])

  // ── refreshModelInfo (with contextLimit from execUI) ──
  const refreshModelInfoFinal = useCallback(async () => {
    try {
      const [cfg, limit] = await Promise.all([getCurrentConfig(), getContextLimit()])
      if (cfg?.model) setModelName(cfg.model)
      if (limit !== null) execUI.setContextLimit(limit)
    } catch {}
  }, [])

  // ── Fetch context limit on mount ──
  useEffect(() => {
    refreshModelInfoFinal()
  }, [refreshModelInfoFinal])

  // ── Command palette items ──
  const cmdItems = useMemo(
    () => [
      {
        id: 'new-chat',
        label: t('cmd.newChat'),
        desc: t('cmd.newChatDesc'),
        category: t('cmd.category.actions'),
        action: handleNewChat,
      },
      {
        id: 'memories',
        label: t('cmd.memories'),
        desc: t('cmd.memoriesDesc'),
        category: t('cmd.category.browse'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowMemories(true)
        },
      },
      {
        id: 'workflows',
        label: t('cmd.workflows'),
        desc: t('cmd.workflowsDesc'),
        category: t('cmd.category.browse'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowWorkflow(true)
        },
      },
      {
        id: 'skills',
        label: t('cmd.skills'),
        desc: t('cmd.skillsDesc'),
        category: t('cmd.category.browse'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowSkills(true)
        },
      },
      {
        id: 'knowledge',
        label: t('cmd.knowledge'),
        desc: t('cmd.knowledgeDesc'),
        category: t('cmd.category.browse'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowKnowledge(true)
        },
      },
      {
        id: 'mcp',
        label: t('cmd.mcp'),
        desc: t('cmd.mcpDesc'),
        category: t('cmd.category.browse'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowMcp(true)
        },
      },
      {
        id: 'plugins',
        label: t('cmd.plugins'),
        desc: t('cmd.pluginsDesc'),
        category: t('cmd.category.browse'),
        action: () => {
          setCmdPaletteOpen(false)
          // 与开发者中心互斥（全窗口覆盖层不双层堆叠）
          modals.setShowPluginDev(false)
          modals.setShowPlugins(true)
        },
      },
      {
        id: 'models',
        label: t('cmd.models'),
        desc: t('cmd.modelsDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowModels(true)
        },
      },
      {
        id: 'themes',
        label: t('cmd.themes'),
        desc: t('cmd.themesDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowThemes(true)
        },
      },
      {
        id: 'external-agents',
        label: t('cmd.externalAgents'),
        desc: t('cmd.externalAgentsDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowExternalAgents(true)
        },
      },
      {
        id: 'project',
        label: t('cmd.project'),
        desc: t('cmd.projectDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowProject(true)
        },
      },
      {
        id: 'browser',
        label: t('cmd.browser'),
        desc: t('cmd.browserDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowBrowser(true)
        },
      },
      {
        id: 'mobile',
        label: t('cmd.mobile'),
        desc: t('cmd.mobileDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowMobile(true)
        },
      },
      {
        id: 'soul',
        label: t('cmd.soul'),
        desc: t('cmd.soulDesc'),
        category: t('cmd.category.settings'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowSoul(true)
        },
      },
      {
        id: 'security',
        label: t('cmd.security'),
        desc: t('cmd.securityDesc'),
        category: t('cmd.category.management'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowSecurity(true)
        },
      },
      {
        id: 'force-reset',
        label: t('cmd.forceReset'),
        desc: t('cmd.forceResetDesc'),
        category: t('cmd.category.management'),
        action: agentControl.forceReset,
      },
      {
        id: 'snake-game',
        label: t('cmd.snakeGame'),
        desc: t('cmd.snakeGameDesc'),
        category: t('cmd.category.fun'),
        action: () => {
          setCmdPaletteOpen(false)
          modals.setShowSnakeGame(true)
        },
      },
    ],
    [t, handleNewChat, agentControl.forceReset],
  )

  // ── Derived / computed ──
  const thinkingStep = useMemo(() => {
    const timeline = execUI.timeline
    if (timeline.length === 0) return ''
    // 显示优先级：agent 正文输出（text）> 思考（thinking）> 工具调用。
    // bar 语义是「agent 现在在产出什么」——agent 输出正文时显示正文，
    // 否则显示思考过程；仅当完全没有 agent 文本（思考/正文都无）时才回退
    // 显示当前 running 工具调用（tool_call 优先会导致思考与工具交替时
    // 在两个文案间每帧跳变（闪抖））。
    const lastText = timeline.filter(t => t.kind === 'text').pop() as
      { kind: 'text'; text: string } | undefined
    const lastThinking = timeline.filter(t => t.kind === 'thinking').pop() as
      { kind: 'thinking'; text: string } | undefined
    const text = lastText?.text || lastThinking?.text || ''
    if (text && text.trim().length >= 2) {
      const firstPara = text.split('\n\n')[0]?.trim() || ''
      if (firstPara.length <= 200) return firstPara
      const firstSentence = firstPara.split(/[。]|[.]\s/)[0]
      if (firstSentence && firstSentence.length <= 180)
        return firstSentence + (firstSentence.length < firstPara.length ? '…' : '')
      return firstPara.slice(0, 160) + '…'
    }
    // 无 agent 文本 → 显示当前正在执行的工具（仅 running 态；成功/失败不产生
    // "执行中"文案，否则执行完成后标签永远停留在 xxx executing... 无法闭合）
    const last = timeline[timeline.length - 1]
    if (last.kind === 'tool_call' && last.toolName && last.status === 'running') {
      return last.toolName + ' executing...'
    }
    if (execUI.currentTaskDesc) return execUI.currentTaskDesc
    return ''
  }, [execUI.timeline, execUI.currentTaskDesc])

  const displayTokenUsage = useMemo(() => execUI.mainTokenUsage, [execUI.mainTokenUsage])
  const liveCalls = useMemo(
    () => execUI.timeline.filter(t => t.kind === 'tool_call').length,
    [execUI.timeline],
  )

  // ── UI flag state ──
  const [cmdPaletteOpen, setCmdPaletteOpen] = useState(false)
  const [focusSignal, setFocusSignal] = useState(0)
  const [executionCounter, setExecutionCounter] = useState(0)
  const [showDesktopToolbar, setShowDesktopToolbar] = useState(false)
  const [regionPickerMode, setRegionPickerMode] = useState<'picker' | 'capture' | 'ocr' | null>(
    null,
  )

  // ── Return ──
  return {
    // Init
    appState,
    initError,
    initItems,
    fadeOut,
    startupStats,
    setAppState,
    setInitError,
    setInitItems,
    setTools,
    setModelName,
    setContextLimit: execUI.setContextLimit,
    showToast,

    // Core
    messages,
    isProcessing,
    status,
    modelName,
    sessionId,
    setSessionId,
    tools,

    // Mode
    mode,
    // useEvents 的 mode_changed 广播依赖此 setter（手机端 /switch-mode 同步桌面）。
    // 缺失会导致 h.setMode 为 undefined，mode_changed 被静默丢弃——桌面自己切换
    // 走 useAgentControl.handleSetMode（内部直接 setModeState）不受影响，但手机端
    // 切换后桌面输入框 mode 不更新（实测根因）。
    setMode: setModeState,

    // Mood
    mood,
    setMood,

    // Modals
    ...modals,

    // Execution UI
    showExecTrace: execUI.showExecTrace,
    setShowExecTrace: execUI.setShowExecTrace,
    execTraceOverride: execUI.execTraceOverride,
    setExecTraceOverride: execUI.setExecTraceOverride,
    dismissThinking: execUI.dismissThinking,
    setDismissThinking: execUI.setDismissThinking,
    stepIndex: execUI.stepIndex,
    goal: execUI.goal,
    progress: execUI.progress,
    execPhase: execUI.execPhase,
    security: execUI.security,
    userInputRequest: execUI.userInputRequest,
    pauseState: execUI.pauseState,
    completed: execUI.completed,
    timeline: execUI.timeline,
    goalType: execUI.goalType,

    // Workflow run
    workflowRunSteps: execUI.workflowRunSteps,
    setWorkflowRunSteps: execUI.setWorkflowRunSteps,
    workflowRunId: execUI.workflowRunId,
    setWorkflowRunId: execUI.setWorkflowRunId,
    lastWorkflowId: execUI.lastWorkflowId,
    isWorkflowPaused: execUI.isWorkflowPaused,
    handleWfPause: agentControl.handleWfPause,
    handleWfResume: agentControl.handleWfResume,
    handleWorkflowPermCancel: agentControl.handleWorkflowPermCancel,
    handleWorkflowPermConfirm: agentControl.handleWorkflowPermConfirm,
    handleWorkflowExitCancel: agentControl.handleWorkflowExitCancel,
    handleWorkflowExitConfirm: agentControl.handleWorkflowExitConfirm,
    showWorkflowPermConfirm: execUI.showWorkflowPermConfirm,
    setShowWorkflowPermConfirm: execUI.setShowWorkflowPermConfirm,
    showWorkflowExitConfirm: execUI.showWorkflowExitConfirm,
    setShowWorkflowExitConfirm: execUI.setShowWorkflowExitConfirm,
    hasWorkflowActivity: execUI.hasWorkflowActivity,
    setHasWorkflowActivity: execUI.setHasWorkflowActivity,

    // Token usage
    mainTokenUsage: execUI.mainTokenUsage,
    execTokenUsage: execUI.execTokenUsage,
    totalDurationMs: execUI.totalDurationMs,
    totalCalls: execUI.totalCalls,
    contextLimit: execUI.contextLimit,
    currentTaskDesc: execUI.currentTaskDesc,

    // Setters (for useEvents)
    setMessages,
    setIsProcessing,
    setCompleted: execUI.setCompleted,
    setStepIndex: execUI.setStepIndex,
    setGoal: execUI.setGoal,
    setProgress: execUI.setProgress,
    setExecPhase: execUI.setExecPhase,
    setSecurity: execUI.setSecurity,
    setUserInputRequest: execUI.setUserInputRequest,
    setPauseState: execUI.setPauseState,
    setTimeline: execUI.setTimeline,
    setGoalType: execUI.setGoalType,
    setMainTokenUsage: execUI.setMainTokenUsage,
    setExecTokenUsage: execUI.setExecTokenUsage,
    setTotalDurationMs: execUI.setTotalDurationMs,
    setTotalCalls: execUI.setTotalCalls,
    executionCounter,
    setExecutionCounter,
    setCurrentTaskDesc: execUI.setCurrentTaskDesc,

    // Refine
    refineState: execUI.refineState,
    setRefineState: execUI.setRefineState,
    refining: execUI.refining,
    setRefining: execUI.setRefining,
    pendingRefine: execUI.pendingRefine,
    setPendingRefine: execUI.setPendingRefine,

    // Planner / Approval / TaskBubble
    showPlannerModal: execUI.showPlannerModal,
    planData: execUI.planData,
    showReview: execUI.showReview,
    approvalState: execUI.approvalState,
    taskBubbleVisible: execUI.taskBubbleVisible,
    setShowPlannerModal: execUI.setShowPlannerModal,
    setPlanData: execUI.setPlanData,
    setShowReview: execUI.setShowReview,
    setApprovalState: execUI.setApprovalState,
    setTaskBubbleVisible: execUI.setTaskBubbleVisible,
    transitionTask: execUI.transitionTask,
    setTaskPriority: execUI.setTaskPriority,
    reorderTask: execUI.reorderTask,

    // Command palette & keyboard
    cmdPaletteOpen,
    focusSignal,
    showDesktopToolbar,
    regionPickerMode,
    expandedCalls,
    setCmdPaletteOpen,
    setFocusSignal,
    setShowDesktopToolbar,
    setRegionPickerMode,
    setExpandedCalls,

    // Refs
    refs: {
      streamingMsgId,
      lastStreamingMsgId,
      executionActiveRef,
      processingRef,
      lastSentRef,
      sendSeqRef,
      messagesRef,
      toolCallCountRef,
      messagesRestoredRef,
    },

    // Computed
    cmdItems,
    thinkingStep,
    displayTokenUsage,
    liveCalls,

    // Handlers
    handleSend,
    handleNewChat,
    reloadChatFromBackend,
    handleRetryAgent: agentControl.handleRetryAgent,
    handlePause: agentControl.handlePause,
    handleContinue: agentControl.handleContinue,
    handleInterrupt: agentControl.handleInterrupt,
    handleGracefulStop: agentControl.handleGracefulStop,
    handleAppendInstruction: agentControl.handleAppendInstruction,
    handleTerminate: agentControl.handleTerminate,
    handleSetMode: agentControl.handleSetMode,
    toggleWorkAgentMode: agentControl.toggleWorkAgentMode,
    forceReset: agentControl.forceReset,
    handleRefine: execUI.handleRefine,
    handleSkipRefine: execUI.handleSkipRefine,
    handleRate: agentControl.handleRate,
    toggleExpand,
    addMessage,

    runInitialization,
    refreshModelInfo: refreshModelInfoFinal,
  }
}