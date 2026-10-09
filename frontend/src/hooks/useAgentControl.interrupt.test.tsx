import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useAgentControl, type AgentControlDeps } from './useAgentControl'
import { countRetainedBackgroundTasks, interrupt } from '../main-window/lib/api'
import { subscribeBackgroundTasksChanged } from '../main-window/lib/backgroundTasks'

vi.mock('../main-window/lib/api', () => ({
  interrupt: vi.fn(async () => ''),
  countRetainedBackgroundTasks: vi.fn(async () => 0),
  isLlmConfigured: vi.fn(async () => true),
  getExecutionState: vi.fn(async () => ({ busy: false })),
  gracefulStop: vi.fn(async () => ''),
  pauseExecution: vi.fn(async () => ''),
  continueExecution: vi.fn(async () => ''),
  appendInstruction: vi.fn(async () => ''),
  terminateExecution: vi.fn(async () => ''),
  forceReset: vi.fn(async () => ''),
  setMode: vi.fn(async () => ''),
  retryAgent: vi.fn(async () => null),
  getToolPermissions: vi.fn(async () => ({})),
  setToolPermissions: vi.fn(async () => ''),
  wfPause: vi.fn(async () => ''),
  wfResume: vi.fn(async () => ''),
  submitExecutionRating: vi.fn(async () => ''),
  backendErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}))

/** 只补齐类型，不给行为——本文件只关心 handleInterrupt 一条路径 */
function makeDeps(over: Partial<AgentControlDeps> = {}): AgentControlDeps {
  const noop = () => {}
  return {
    isProcessing: false,
    sessionId: 's1',
    mode: 'leader',
    execPhase: 'idle',
    timeline: [],
    workflowRunId: null,
    hasWorkflowActivity: false,
    setHasWorkflowActivity: noop,
    setExecutionStage: noop,
    setCompleted: noop,
    setGoal: noop,
    setExecPhase: noop,
    setTimeline: noop,
    setStepIndex: noop,
    setPlanData: noop,
    setSecurity: noop,
    setRefineState: noop,
    setPendingRefine: noop,
    setWorkflowRunSteps: noop,
    setIsWorkflowPaused: noop,
    setShowWorkflowPermConfirm: noop,
    setShowWorkflowExitConfirm: noop,
    setPauseState: noop,
    setMode: noop,
    messagesRef: { current: [] },
    streamingMsgId: { current: null },
    lastStreamingMsgId: { current: null },
    executionActiveRef: { current: false },
    interruptedRef: { current: false },
    showToast: vi.fn(),
    setMood: noop,
    removeRetryErrorBubble: noop,
    ...over,
  }
}

function setup(over: Partial<AgentControlDeps> = {}) {
  const deps = makeDeps(over)
  const view = renderHook(() => useAgentControl(deps))
  return { deps, showToast: deps.showToast as ReturnType<typeof vi.fn>, ...view }
}

describe('handleInterrupt — 必须把后端的事实汇报透给用户', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })
  afterEach(() => {
    cleanup()
  })

  it('正常中断：info 级提示，且带上后端返回的事实句原文', async () => {
    const report = 'Task interrupted: 无活动工作流；已终止 1 个前台进程（0 个自行退出）'
    vi.mocked(interrupt).mockResolvedValue(report)
    vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(0)
    const showToast = vi.fn()

    const { result } = setup({ showToast })
    await act(async () => {
      await result.current.handleInterrupt()
    })

    expect(interrupt).toHaveBeenCalledTimes(1)
    expect(showToast).toHaveBeenCalledTimes(1)
    const [message, type] = showToast.mock.calls[0]
    expect(type).toBe('info')
    expect(message).toContain(report)
  })

  it('汇报含「未能终止」时升级为 warning 并原样带出明细', async () => {
    const report =
      'Task interrupted: 无活动工作流；已终止 0 个前台进程（1 个自行退出），另有 2 个前台进程未能终止（已如实记录）'
    vi.mocked(interrupt).mockResolvedValue(report)
    vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(0)
    const showToast = vi.fn()

    const { result } = setup({ showToast })
    await act(async () => {
      await result.current.handleInterrupt()
    })

    expect(showToast).toHaveBeenCalledTimes(1)
    const [message, type] = showToast.mock.calls[0]
    expect(type).toBe('warning')
    expect(message).toContain('未能终止')
    expect(message).toContain('另有 2 个前台进程未能终止（已如实记录）')
  })

  it('保留项 > 0：追加 warning 提示，并通知面板刷新（入口因此自动出现）', async () => {
    vi.mocked(interrupt).mockResolvedValue(
      'Task interrupted: 无活动工作流；已终止 1 个前台进程（0 个自行退出），保留 2 个后台任务仍在运行',
    )
    vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(2)
    const showToast = vi.fn()
    const notified = vi.fn()
    const unsubscribe = subscribeBackgroundTasksChanged(notified)

    const { result } = setup({ showToast })
    await act(async () => {
      await result.current.handleInterrupt()
    })
    unsubscribe()

    expect(countRetainedBackgroundTasks).toHaveBeenCalledTimes(1)
    const warnings = showToast.mock.calls.filter(c => c[1] === 'warning')
    expect(warnings).toHaveLength(1)
    expect(warnings[0][0]).toContain('2')
    expect(warnings[0][0]).toContain('后台任务')
    expect(notified).toHaveBeenCalledTimes(1)
  })

  it('保留项为 0 时不再多嘴（只报一次事实，不叠加保留提示）', async () => {
    vi.mocked(interrupt).mockResolvedValue(
      'Task interrupted: 无活动工作流；已终止 1 个前台进程（0 个自行退出）',
    )
    vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(0)
    const showToast = vi.fn()

    const { result } = setup({ showToast })
    await act(async () => {
      await result.current.handleInterrupt()
    })

    expect(showToast).toHaveBeenCalledTimes(1)
    expect(showToast.mock.calls[0][1]).toBe('info')
  })

  it('后端没给出汇报（旧后端 / IPC 返回 null）时宁可不报，也不编「已停止」', async () => {
    vi.mocked(interrupt).mockResolvedValue(null)
    vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(0)
    const showToast = vi.fn()

    const { result } = setup({ showToast })
    await act(async () => {
      await result.current.handleInterrupt()
    })

    expect(showToast).not.toHaveBeenCalled()
  })

  it('保留项计数失败不吞掉已发出的事实汇报，也不阻断中断', async () => {
    vi.mocked(interrupt).mockResolvedValue(
      'Task interrupted: 无活动工作流；已终止 1 个前台进程（0 个自行退出）',
    )
    vi.mocked(countRetainedBackgroundTasks).mockRejectedValue(new Error('IPC down'))
    const showToast = vi.fn()

    const { result } = setup({ showToast })
    await act(async () => {
      await result.current.handleInterrupt()
    })

    expect(showToast).toHaveBeenCalledTimes(1)
    expect(showToast.mock.calls[0][1]).toBe('info')
  })

  it('中断自身的失败仍按原语义上抛（不吞、不改执行语义）', async () => {
    vi.mocked(interrupt).mockRejectedValue(new Error('IPC down'))
    const showToast = vi.fn()

    const { result } = setup({ showToast })
    await act(async () => {
      await expect(result.current.handleInterrupt()).rejects.toThrow('IPC down')
    })
    expect(showToast).not.toHaveBeenCalled()
  })
})
