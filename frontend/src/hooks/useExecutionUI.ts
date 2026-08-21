// useExecutionUI — 执行追踪、上下文精炼、工作流运行时、Planner/Approval 状态
import { useState, useCallback, useEffect } from 'react'
import { invoke, listen } from '../core/bridge'
import type { SecurityCheck, UserInputRequest, TimelineEntry, PlanData, PlanTask, TaskStatus, TaskPriority, WorkflowRunStep } from '../core/types'
import { executeSessionRefine, refineSkip } from '../main-window/lib/api'
import type { Toast } from './useInit'

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
  const [completed, setCompleted] = useState(false)
  const [timeline, setTimeline] = useState<TimelineEntry[]>([])

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
  const handleRefine = useCallback(async () => {
    showToast('Refining session context...', 'info')
    setRefining(true)
    try {
      await executeSessionRefine()
    } catch (e: any) {
      showToast('Refine failed: ' + (e.message || e), 'error')
      // Reset refine state so modal doesn't stay stuck in loading
      setRefineState(null)
    } finally {
      // 成功/失败都恢复提炼中遮罩——失败不留全屏遮罩（否则用户无法回到原有界面）
      setRefining(false)
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
            (typeof rawStatus === 'object' &&
              rawStatus !== null &&
              'Error' in rawStatus)
          setWorkflowRunSteps((prev: WorkflowRunStep[]) =>
            prev.map(s =>
              s.id === str(data.step_id)
                ? { ...s, status: failed ? 'failed' : 'completed' }
                : s,
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
    completed,
    setCompleted,
    timeline,
    setTimeline,
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