// useAgentControl — Agent 控制函数（pause/continue/interrupt/stop/terminate/retry/reset/mode/rate/wf）
import { useCallback, useRef } from 'react'
import { invoke } from '../core/bridge'
import type { MoodState } from '../ui/MoodFace'
import type { TimelineEntry } from '../core/types'
import {
  isLlmConfigured,
  isBusy,
  interrupt,
  gracefulStop,
  pauseExecution,
  continueExecution,
  appendInstruction,
  terminateExecution,
  forceReset as apiForceReset,
  setMode as apiSetMode,
  retryAgent,
  getToolPermissions,
  setToolPermissions,
  wfPause,
  wfResume,
  submitExecutionRating,
} from '../main-window/lib/api'
import type { Toast } from './useInit'

export interface AgentControlDeps {
  // State (read)
  isProcessing: boolean
  sessionId: string
  mode: string
  execPhase: string
  timeline: TimelineEntry[]
  workflowRunId: string | null
  hasWorkflowActivity: boolean
  setHasWorkflowActivity: (v: boolean) => void
  // Setters (using React.Dispatch where applicable)
  setIsProcessing: (v: boolean) => void
  setCompleted: (v: boolean) => void
  setGoal: (v: string) => void
  setExecPhase: React.Dispatch<React.SetStateAction<any>>
  setTimeline: React.Dispatch<React.SetStateAction<TimelineEntry[]>>
  setStepIndex: (v: number) => void
  setPlanData: React.Dispatch<React.SetStateAction<any>>
  setSecurity: React.Dispatch<React.SetStateAction<any>>
  setRefineState: React.Dispatch<React.SetStateAction<any>>
  setPendingRefine: React.Dispatch<React.SetStateAction<any>>
  setWorkflowRunSteps: React.Dispatch<React.SetStateAction<any>>
  setIsWorkflowPaused: (v: boolean) => void
  setShowWorkflowPermConfirm: (v: boolean) => void
  setShowWorkflowExitConfirm: (v: boolean) => void
  setPauseState: React.Dispatch<React.SetStateAction<{ actionId: string } | null>>
  setMode: (v: string) => void
  // Refs
  messagesRef: React.MutableRefObject<any[]>
  streamingMsgId: React.MutableRefObject<string | null>
  lastStreamingMsgId: React.MutableRefObject<string | null>
  executionActiveRef: React.MutableRefObject<boolean>
  // Callbacks
  showToast: (msg: string, type?: Toast['type']) => void
  setMood: (m: MoodState) => void
  /** 重试开始前移除失败回合的错误气泡（回合内重来，错误内容不残留） */
  removeRetryErrorBubble: () => void
}

export function useAgentControl(deps: AgentControlDeps) {
  const {
    isProcessing,
    sessionId,
    mode,
    execPhase,
    timeline,
    workflowRunId,
    hasWorkflowActivity,
    setHasWorkflowActivity,
    setIsProcessing,
    setCompleted,
    setGoal,
    setExecPhase,
    setTimeline,
    setStepIndex,
    setPlanData,
    setSecurity,
    setRefineState,
    setPendingRefine,
    setWorkflowRunSteps,
    setIsWorkflowPaused,
    setShowWorkflowPermConfirm,
    setShowWorkflowExitConfirm,
    setPauseState,
    setMode: setModeState,
    messagesRef,
    streamingMsgId,
    lastStreamingMsgId,
    executionActiveRef,
    showToast,
    setMood,
    removeRetryErrorBubble,
  } = deps

  const previousModeRef = useRef<string>('leader')
  const pendingExitModeRef = useRef<string>('leader')

  // ── Backend ready check ──
  // 只检查后端连通性，不再检查 busy：
  // 执行中发送 = 追加指令（与移动端一致），busy 时后端会走 mobile_append
  // 入队并返回 appended=true，前端据此提示"已作为追加指令插入"。
  // 若在这里因 busy 直接 return false，桌面端将无法执行中追加——双端行为不一致。
  const checkBackendReady = useCallback(async (): Promise<boolean> => {
    try {
      await isBusy()
      return true
    } catch {
      invoke('hud_update', { text: '无法连接后端', phase: 'error' })
      return false
    }
  }, [])

  // ── handleRetryAgent ──
  const handleRetryAgent = useCallback(
    async (input: string) => {
      if (isProcessing) {
        try {
          const busy = await isBusy()
          if (!busy) {
            setIsProcessing(false)
            executionActiveRef.current = false
          } else {
            invoke('hud_update', { text: 'Already processing, please wait', phase: 'warning' })
            return
          }
        } catch {
          invoke('hud_update', { text: 'Already processing, please wait', phase: 'warning' })
          return
        }
      }
      const configured = await isLlmConfigured()
      if (!configured) {
        invoke('hud_update', { text: 'Please configure API Key first', phase: 'error' })
        return
      }
      // 回合内重来：移除失败回合的错误气泡，新回复流式输出到干净的新气泡
      removeRetryErrorBubble()
      setIsProcessing(true)
      setCompleted(false)
      setExecPhase('retrying')
      setTimeline([])
      setStepIndex(0)
      setPlanData(null)
      setGoal(input.slice(0, 120))
      try {
        const result = await retryAgent()
        if (result === null) {
          invoke('hud_update', { text: 'Connection lost - please try again', phase: 'error' })
        }
      } catch (e: any) {
        invoke('hud_update', { text: 'Retry failed: ' + (e.message || e), phase: 'error' })
      } finally {
        setIsProcessing(false)
        executionActiveRef.current = false
      }
    },
    [isProcessing],
  )

  // ── toggleWorkAgentMode ──
  const toggleWorkAgentMode = useCallback(async () => {
    if (mode !== 'workflow') {
      const perms = await getToolPermissions()
      let parsed: { file_access: boolean; web_search: boolean; system_automation: boolean } | null =
        null
      if (perms && typeof perms === 'string') {
        try {
          parsed = JSON.parse(perms)
        } catch {}
      }
      const allGranted =
        parsed && parsed.file_access && parsed.web_search && parsed.system_automation

      if (!allGranted) {
        setShowWorkflowPermConfirm(true)
        return
      }

      previousModeRef.current = mode
      apiSetMode('workflow')
      setModeState('workflow')
      setExecPhase('workflow')
    } else {
      const prev = previousModeRef.current
      apiSetMode(prev)
      setModeState(prev)
      setExecPhase('')
    }
  }, [mode, workflowRunId])

  // ── handleSetMode ──
  const handleSetMode = useCallback(
    async (newMode: string) => {
      if (newMode !== mode) {
        previousModeRef.current = mode
      }
      await apiSetMode(newMode)
      setModeState(newMode)
      if (newMode === 'workflow') {
        setExecPhase('workflow')
      } else if (execPhase === 'workflow') {
        setExecPhase('')
      }
    },
    [mode, execPhase, setModeState],
  )

  // ── Workflow confirm/cancel ──
  const handleWorkflowPermCancel = useCallback(() => {
    setShowWorkflowPermConfirm(false)
  }, [])

  const handleWorkflowPermConfirm = useCallback(async () => {
    setShowWorkflowPermConfirm(false)
    await setToolPermissions(true, true, true)
    previousModeRef.current = mode
    apiSetMode('workflow')
    setModeState('workflow')
    setExecPhase('workflow')
  }, [mode, setModeState])

  const handleWorkflowExitCancel = useCallback(() => {
    setShowWorkflowExitConfirm(false)
  }, [])

  const handleWorkflowExitConfirm = useCallback(async () => {
    setShowWorkflowExitConfirm(false)
    const targetMode = pendingExitModeRef.current
    previousModeRef.current = mode
    await apiSetMode(targetMode)
    setModeState(targetMode)
    if (targetMode === 'workflow') {
      setExecPhase('workflow')
    } else {
      setExecPhase('')
    }
  }, [mode, setModeState])

  // ── handlePause ──
  const handlePause = useCallback(async () => {
    await pauseExecution()
  }, [])

  // ── handleContinue ──
  const handleContinue = useCallback(async (actionId: string) => {
    await continueExecution(actionId)
    setPauseState(null)
  }, [])

  // ── handleAppendInstruction ──
  const handleAppendInstruction = useCallback(async (actionId: string, instruction: string) => {
    await appendInstruction(actionId, instruction)
    setPauseState(null)
  }, [])

  // ── handleTerminate ──
  const handleTerminate = useCallback(async (actionId: string) => {
    await terminateExecution(actionId)
    setIsProcessing(false)
    setCompleted(true)
    setPauseState(null)
  }, [])

  // ── handleGracefulStop ──
  const handleGracefulStop = useCallback(async () => {
    await gracefulStop()
    showToast('Graceful stop requested', 'info')
  }, [showToast])

  // ── handleInterrupt ──
  const handleInterrupt = useCallback(async () => {
    await interrupt()
    showToast('Interrupted', 'info')
    setIsProcessing(false)
    setCompleted(true)
  }, [showToast])

  // ── handleWfPause ──
  const handleWfPause = useCallback(async () => {
    if (workflowRunId) {
      await wfPause(workflowRunId)
      setIsWorkflowPaused(true)
      setWorkflowRunSteps((prev: any[]) =>
        prev.map((s: any) => (s.status === 'running' ? { ...s, status: 'paused' } : s)),
      )
    }
  }, [workflowRunId])

  // ── handleWfResume ──
  const handleWfResume = useCallback(async () => {
    if (workflowRunId) {
      await wfResume(workflowRunId)
      setIsWorkflowPaused(false)
    }
  }, [workflowRunId])

  // ── forceReset ──
  const forceReset = useCallback(async () => {
    await apiForceReset()
    messagesRef.current = []
    setIsProcessing(false)
    setCompleted(false)
    setGoal('')
    setTimeline([])
    setExecPhase('')
    setPlanData(null)
    setSecurity(null)
    setRefineState(null)
    setPendingRefine(null)
    setHasWorkflowActivity(false)
    streamingMsgId.current = null
    lastStreamingMsgId.current = null
    showToast('State reset', 'info')
  }, [showToast])

  // ── handleRate ──
  const handleRate = useCallback(
    async (
      name: string,
      rating: number,
      comment: string,
      _saveAsStrategy: boolean,
      userQuestion?: string,
      assistantContent?: string,
    ) => {
      try {
        const steps = timeline
          .filter(t => t.kind === 'tool_call')
          .map(t => ({
            tool: t.toolName || '',
            durationMs: t.durationMs || 0,
            success: t.status === 'success',
          }))
        const goal = userQuestion || name
        const fullComment = assistantContent
          ? `${comment}\n\n---\n相关回复: ${assistantContent.slice(0, 300)}`
          : comment
        await submitExecutionRating(goal, rating, fullComment, steps, sessionId)
        showToast('Rating saved', 'success')
      } catch (e: any) {
        showToast('Failed to save rating: ' + (e.message || e), 'error')
      }
    },
    [timeline, sessionId, showToast],
  )

  return {
    previousModeRef,
    pendingExitModeRef,
    checkBackendReady,
    handleRetryAgent,
    toggleWorkAgentMode,
    handleSetMode,
    handleWorkflowPermCancel,
    handleWorkflowPermConfirm,
    handleWorkflowExitCancel,
    handleWorkflowExitConfirm,
    handlePause,
    handleContinue,
    handleAppendInstruction,
    handleTerminate,
    handleGracefulStop,
    handleInterrupt,
    handleWfPause,
    handleWfResume,
    forceReset,
    handleRate,
  }
}