// useExecutionUI — 执行追踪、上下文精炼、工作流运行时、Planner/Approval 状态
import { useState, useCallback, useEffect, useRef } from 'react'
import { invoke, listen } from '../core/bridge'
import type {
  SecurityCheck,
  UserInputRequest,
  TimelineEntry,
  PlanData,
  PlanTask,
  TaskStatus,
  TaskPriority,
  WorkflowRunStep,
  ApiHealthState,
} from '../core/types'
import { executeSessionRefine, refineSkip } from '../main-window/lib/api'
import type { Toast } from './useInit'
import { initialApiHealthState } from '../main-window/chat/ApiHealthBadge'

export function useExecutionUI(showToast: (msg: string, type?: Toast['type']) => void) {
  // ── Execution trace visibility ──
  const [showExecTrace, setShowExecTrace] = useState(false)
  const [dismissThinking, setDismissThinking] = useState(false)
  /** 气泡执行回溯：非空时执行面板显示该轮历史执行过程（覆盖全局 timeline），
   *  关闭执行面板时由 App 层清除 */
  const [execTraceOverride, setExecTraceOverride] = useState<TimelineEntry[] | null>(null)

  // ── Execution state ──
  const [stepIndex, setStepIndex] = useState(0)
  const [goal, setGoal] = useState('')
  const [progress, setProgress] = useState({ iteration: 0, max: 0, calls: 0 })
  const [execPhase, setExecPhase] = useState<
    'understanding' | 'executing' | 'recording' | 'workflow' | 'retrying' | ''
  >('')
  const [security, setSecurity] = useState<SecurityCheck | null>(null)
  const [userInputRequest, setUserInputRequest] = useState<UserInputRequest | null>(null)
  const [pauseState, setPauseState] = useState<{ actionId: string } | null>(null)
  const [appendQueue, setAppendQueue] = useState<string[]>([])
  const [completed, setCompleted] = useState(false)
  const [timeline, setTimeline] = useState<TimelineEntry[]>([])
  const [apiHealth, setApiHealth] = useState<ApiHealthState>(initialApiHealthState)

  const [goalType, setGoalType] = useState<{
    type: string
    label: string
    confidence: number
  } | null>(null)

  const [mainTokenUsage, setMainTokenUsage] = useState<{
    inputTokens: number
    outputTokens: number
    cacheHitTokens: number
  } | null>(null)

  const [execTokenUsage, setExecTokenUsage] = useState<{
    inputTokens: number
    outputTokens: number
    cacheHitTokens: number
  } | null>(null)

  const [totalDurationMs, setTotalDurationMs] = useState(0)
  const [totalCalls, setTotalCalls] = useState(0)
  const [contextLimit, setContextLimit] = useState<number>(0)
  const [currentTaskDesc, setCurrentTaskDesc] = useState('')

  // ── 上下文提炼状态 ──
  const [refineState, setRefineState] = useState<{
    usagePercent: number
    totalLimit: number
  } | null>(null)
  /** 提炼执行中（全局）：驱动「提炼中」全屏遮罩（弹窗路径与 refine-pending-btn
   *  路径统一）。handleRefine 成功/失败 finally 恢复；useEvents 事件兜底恢复。 */
  const [refining, setRefining] = useState(false)

  const [pendingRefine, setPendingRefine] = useState<{
    usagePercent: number
    totalLimit: number
    skippedTurns: number
  } | null>(null)

  // ── 工作流运行状态 ──
  const [workflowRunSteps, setWorkflowRunSteps] = useState<WorkflowRunStep[]>([])
  const [workflowRunId, setWorkflowRunId] = useState<string | null>(null)
  // 最近一次运行的 workflow id（run 结束后保留，供步骤面板加载参数定义）
  const [lastWorkflowId, setLastWorkflowId] = useState<string | null>(null)
  // WorkflowAgent 是否有过任何执行活动（用于退出 Workflow 模式时的确认弹窗，一旦设 true 永不自动清除）
  const [hasWorkflowActivity, setHasWorkflowActivity] = useState(false)
  const [isWorkflowPaused, setIsWorkflowPaused] = useState(false)
  // 步骤面板被用户**收起**（≠ 清空数据）。面板可见性原本直接等于 workflowRunSteps 非空，
  // 而 ✕ 又直接清空该数组，于是误关一次就既丢数据、又没有任何入口能把它找回来。
  const [workflowPanelDismissed, setWorkflowPanelDismissed] = useState(false)
  const [showWorkflowPermConfirm, setShowWorkflowPermConfirm] = useState(false)
  const [showWorkflowExitConfirm, setShowWorkflowExitConfirm] = useState(false)

  // ── Planner Modal state ──
  const [showPlannerModal, setShowPlannerModal] = useState(false)
  const [planData, setPlanData] = useState<PlanData | null>(null)
  const [showReview, setShowReview] = useState(false)

  // ── Approval Modal state ──
  const [approvalState, setApprovalState] = useState<{
    open: boolean
    kind: string
    title: string
    content: string
    actionId: string
    tenetCount: number
  }>({ open: false, kind: '', title: '', content: '', actionId: '', tenetCount: 0 })

  // ── TaskBubble state ──
  const [taskBubbleVisible, setTaskBubbleVisible] = useState(false)

  // ── Planner task helpers ──
  const transitionTask = useCallback((taskId: number, status: TaskStatus) => {
    setPlanData(prev => {
      if (!prev) return prev
      return {
        ...prev,
        tasks: prev.tasks.map(t => (t.id === taskId ? { ...t, status } : t)),
      }
    })
  }, [])
  const setTaskPriority = useCallback((_taskId: number, _priority: TaskPriority) => {}, [])
  const reorderTask = useCallback((_taskId: number, _direction: -1 | 1) => {}, [])

  // ── Refine handlers ──
  // in-flight 锁：executeSessionRefine 是 await 整轮提炼的 invoke（90s 级），期间
  // refining=true。双击/重复触发会在 refining state 渲染生效前再进 handleRefine，
  // 第二发 invoke 被后端防重拒绝 → catch/finally 提前 setRefining(false)，
  // 破坏第一发提炼执行的 UI 锁（大王实测：提炼执行中 refine 入口可再触发）。
  // 用 ref 锁（不依赖 state 渲染时机）挡住并发重入；finally 统一释放。
  const refineInvokeLockRef = useRef(false)
  const handleRefine = useCallback(async () => {
    if (refineInvokeLockRef.current) return
    refineInvokeLockRef.current = true
    showToast('Refining session context...', 'info')
    setRefining(true)
    // 后端防重拒绝时是否已置 true（另一端真实提炼中）：不释放锁，等事件收敛
    let unlock = true
    try {
      await executeSessionRefine()
    } catch (e: any) {
      const msg = e?.message ?? String(e ?? '')
      if (msg.includes('提炼进行中')) {
        // 另一端已开始提炼（本端点击时 refine_active=true）：refining 保持 true
        // 由 RefineExecuting/session_refined/refine_failed 事件权威收敛——此处
        // 释放会让弹窗/入口在真实提炼执行中重新可触发（大王实测）。
        unlock = false
      } else {
        // 其余拒绝都是「本次未发射任何事件」的前置失败：refine_active 抢占失败（另一端
        // 已在提炼）或 busy 抢占失败（主流程仍在收尾，refine.rs 文案「当前轮次仍在收尾」）。
        // 复位 refining 并提示原因——手动路径不重试，用户需要立刻知道「现在不能提炼」，
        // 而不是静默等待（静默并发执行是本 bug 的根因，必须显式拒绝）。
        showToast('Refine failed: ' + msg, 'error')
        // Reset refine state so modal doesn't stay stuck in loading
        setRefineState(null)
      }
    } finally {
      // 成功/失败都恢复提炼中遮罩——失败不留全屏遮罩（否则用户无法回到原有界面）
      if (unlock) setRefining(false)
      refineInvokeLockRef.current = false
    }
  }, [showToast, setRefineState, setRefining])

  const handleSkipRefine = useCallback(() => {
    if (refineState) {
      setPendingRefine({
        usagePercent: refineState.usagePercent,
        totalLimit: refineState.totalLimit,
        skippedTurns: 0,
      })
    }
    setRefineState(null)
    // 通知后端广播 RefineSkipped——手机端弹窗同步关闭（双端状态一致）
    refineSkip().catch(() => {})
  }, [refineState])

  // ── Workflow event listener ──
  useEffect(() => {
    let cancelled = false
    const unlisten = listen<Record<string, unknown>>('workflow-event', event => {
      if (cancelled) return
      const payload: Record<string, unknown> =
        (event.payload as Record<string, unknown> | undefined) ||
        (event as unknown as Record<string, unknown>)
      const str = (v: unknown, d = '') => (typeof v === 'string' ? v : d)
      const eventType = str(payload.event)

      switch (eventType) {
        case 'run_started': {
          const data = payload
          setWorkflowRunSteps([])
          setWorkflowRunId(str(data.workflow_id) || null)
          setLastWorkflowId(str(data.workflow_id) || null)
          setHasWorkflowActivity(true) // 持久标记：WorkflowAgent 确实执行了内容
          setWorkflowPanelDismissed(false) // 新一轮运行：面板自动展开（上一轮的收起状态不该继承）
          break
        }
        case 'step_run_started': {
          const data = payload
          setWorkflowRunSteps((prev: WorkflowRunStep[]) => {
            if (prev.some(s => s.id === str(data.step_id))) {
              return prev.map(s => (s.id === str(data.step_id) ? { ...s, status: 'running' } : s))
            }
            return [
              ...prev,
              {
                id: str(data.step_id),
                name: str(data.step_name, '未知步骤'),
                status: 'running',
                depth: typeof data.depth === 'number' ? data.depth : 0,
                kind: str(data.kind, 'tool'),
              },
            ]
          })
          break
        }
        case 'step_run_completed': {
          const data = payload
          // 失败判定：StepRunStatus 外部标记枚举，Error(String) → {"Error": "..."}。
          // 旧实现只比对 'Failed' 字符串（枚举无此变体）→ 永假，失败步骤被误收敛为绿色。
          const rawStatus = data.status
          const failed =
            (typeof rawStatus === 'string' && rawStatus === 'Failed') ||
            (typeof rawStatus === 'object' && rawStatus !== null && 'Error' in rawStatus)
          setWorkflowRunSteps((prev: WorkflowRunStep[]) =>
            prev.map(s =>
              s.id === str(data.step_id) ? { ...s, status: failed ? 'failed' : 'completed' } : s,
            ),
          )
          break
        }
        case 'step_run_paused': {
          setIsWorkflowPaused(true)
          const data = payload
          setWorkflowRunSteps((prev: WorkflowRunStep[]) =>
            prev.map(s => (s.id === str(data.step_id) ? { ...s, status: 'paused' as const } : s)),
          )
          break
        }
        case 'run_completed': {
          setIsWorkflowPaused(false)
          setWorkflowRunSteps((prev: WorkflowRunStep[]) =>
            prev.map(s =>
              s.status === 'running' || s.status === 'pending' ? { ...s, status: 'completed' } : s,
            ),
          )
          setWorkflowRunId(null)
          break
        }
      }
    })

    return () => {
      unlisten.then(fn => fn())
    }
  }, [])

  // 工作流步骤面板的收起 / 展开。
  // 收起**只隐藏 UI，不清空 workflowRunSteps**：这是「关了就再也打不开」的修复点——
  // 数组是全仓唯一的面板可见性来源，且只由运行事件写入，清空后除非有新的 step_run_started
  // 否则永远回不来（面板里的暂停/终止/紧急停止入口也跟着一起丢）。
  const dismissWorkflowPanel = useCallback(() => setWorkflowPanelDismissed(true), [])
  const showWorkflowPanel = useCallback(() => setWorkflowPanelDismissed(false), [])

  return {
    // Execution trace UI
    showExecTrace,
    setShowExecTrace,
    dismissThinking,
    setDismissThinking,
    execTraceOverride,
    setExecTraceOverride,

    // Execution state
    stepIndex,
    setStepIndex,
    goal,
    setGoal,
    progress,
    setProgress,
    execPhase,
    setExecPhase,
    security,
    setSecurity,
    userInputRequest,
    setUserInputRequest,
    pauseState,
    setPauseState,
    appendQueue,
    setAppendQueue,
    completed,
    setCompleted,
    timeline,
    setTimeline,
    apiHealth,
    setApiHealth,
    goalType,
    setGoalType,

    // Token usage
    mainTokenUsage,
    setMainTokenUsage,
    execTokenUsage,
    setExecTokenUsage,
    totalDurationMs,
    setTotalDurationMs,
    totalCalls,
    setTotalCalls,
    contextLimit,
    setContextLimit,
    currentTaskDesc,
    setCurrentTaskDesc,

    // Refine
    refineState,
    setRefineState,
    pendingRefine,
    setPendingRefine,
    refining,
    setRefining,
    handleRefine,
    handleSkipRefine,

    // Workflow run
    workflowRunSteps,
    setWorkflowRunSteps,
    workflowRunId,
    setWorkflowRunId,
    lastWorkflowId,
    setLastWorkflowId,
    isWorkflowPaused,
    setIsWorkflowPaused,
    workflowPanelDismissed,
    dismissWorkflowPanel,
    showWorkflowPanel,
    showWorkflowPermConfirm,
    setShowWorkflowPermConfirm,
    showWorkflowExitConfirm,
    setShowWorkflowExitConfirm,
    hasWorkflowActivity,
    setHasWorkflowActivity,

    // Planner / Approval / TaskBubble
    showPlannerModal,
    setShowPlannerModal,
    planData,
    setPlanData,
    showReview,
    setShowReview,
    approvalState,
    setApprovalState,
    taskBubbleVisible,
    setTaskBubbleVisible,
    transitionTask,
    setTaskPriority,
    reorderTask,
  }
}
