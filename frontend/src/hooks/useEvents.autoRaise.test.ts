/**
 * useEvents × refine_prompt → autoRaiseForceThreshold 接线单测
 *
 * 钉死 usagePercent 的**全部**写入点（事件驱动，无轮询）：正常弹窗
 * （setRefineState）与 pendingRefine（跳过常驻）在拿到含新 usagePercent 的
 * 完整 RefineState 后都触发一次后端强制线 reconcile——usage 把滑块下限
 * 顶高时，已生效配置同步抬升，消除「滑块显示 70% / 后端还是 0.55」的
 * 显示撒谎；阈值已不低于下限则不写盘。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import type { NuphusEvent } from '../core/types'
import type { EventHandlers } from './useEvents'
import { useEvents } from './useEvents'

const { listenMock, invokeMock } = vi.hoisted(() => ({
  listenMock: vi.fn(),
  invokeMock: vi.fn(),
}))
vi.mock('../core/bridge', () => ({
  invoke: invokeMock,
  listen: (name: string, handler: (payload: unknown) => void) => {
    listenMock(name, handler)
    return Promise.resolve(() => {})
  },
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ setFocus: vi.fn(), setAlwaysOnTop: vi.fn(), minimize: vi.fn() }),
}))
vi.mock('../ui/sound', () => ({ playUiSound: vi.fn() }))
vi.mock('../ui/islandChannel', () => ({
  showAppFeedbackByHudPhase: vi.fn(),
  showAppFeedback: vi.fn(),
}))

/** 跳过常驻数据（pendingRefine）：与 useExecutionUI state 同形 */
const PENDING = {
  usagePercent: 50,
  totalLimit: 1000,
  tier: 'large' as const,
  forceThreshold: 0.55,
  forceMin: 0.5,
  forceMax: 0.8,
  skippedTurns: 1,
}

function makeHandlers(pendingRefine?: unknown) {
  // vi.fn() 单独持有：handlers 经 `as unknown as EventHandlers` 后 setter 类型
  // 变回真实 useState dispatcher，断言只能走这个引用（同 followReset.test 的
  // followReset 替身模式）。
  const setPendingRefine = vi.fn()
  const handlers = {
    refs: {
      streamingMsgId: { current: 'msg-1' },
      lastStreamingMsgId: { current: null },
      executionActiveRef: { current: false },
      processingRef: { current: false },
      turnMetaTokensRef: { current: {} },
      interruptedRef: { current: false },
      stickyFollowResetRef: { current: vi.fn() },
    },
    refineState: null,
    pendingRefine: pendingRefine ?? null,
    modelName: 'm-1',
    setApiHealth: vi.fn(),
    setExecutionStage: vi.fn(),
    setDismissThinking: vi.fn(),
    setRefineState: vi.fn(),
    setPendingRefine,
    setRefining: vi.fn(),
    setExecutionCounter: vi.fn(),
    setStepIndex: vi.fn(),
    setGoal: vi.fn(),
    setTimeline: vi.fn(),
    setTaskRuns: vi.fn(),
    setCompleted: vi.fn(),
    setTotalDurationMs: vi.fn(),
    setTotalCalls: vi.fn(),
    setTurnMeta: vi.fn(),
    setLiveTurnToolCalls: vi.fn(),
    setExecTokenUsage: vi.fn(),
    setMainTokenUsage: vi.fn(),
    setExecPhase: vi.fn(),
    setMessages: vi.fn(),
    setProgress: vi.fn(),
    setPauseState: vi.fn(),
    setMood: vi.fn(),
    addMessage: vi.fn(),
  } as unknown as EventHandlers
  return { handlers, setPendingRefine }
}

/** 经 listen mock 拿到 nuphus-event handler，按后端帧格式（{ seq, event }）驱动 */
function driveEvent(event: NuphusEvent, seq = 1): void {
  const call = listenMock.mock.calls.find(c => c[0] === 'nuphus-event')
  if (!call) throw new Error('nuphus-event listener 未注册')
  act(() => {
    ;(call[1] as (payload: unknown) => void)({ seq, event })
  })
}

/** refine_prompt 帧：win=1000 / usage=70% / large 档 / 阈值可调 50~80 */
function refinePrompt(forceThreshold: number): NuphusEvent {
  return {
    type: 'refine_prompt',
    current_tokens: 700,
    refine_limit: 0,
    force_limit: 0,
    threshold: 0.8,
    context_window: 1000,
    tier: 'large',
    force_threshold: forceThreshold,
    force_min: 0.5,
    force_max: 0.8,
    forced: false,
  }
}

describe('useEvents refine_prompt → autoRaiseForceThreshold', () => {
  beforeEach(() => {
    listenMock.mockClear()
    invokeMock.mockClear()
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('① 正常弹窗分支：usage 70% 顶低下限至 70，旧阈值 0.55 → 抬到 0.70', () => {
    const { handlers } = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent(refinePrompt(0.55))

    expect(handlers.setRefineState).toHaveBeenCalledWith(
      expect.objectContaining({ usagePercent: 70, forceThreshold: 0.55 }),
    )
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('set_session_refine_config', { forceThreshold: 0.7 })
  })

  it('② pendingRefine 分支：跳过常驻数据同样 reconcile（usagePercent 70 → 0.70）', () => {
    const { handlers, setPendingRefine } = makeHandlers(PENDING)
    renderHook(() => useEvents(handlers))

    driveEvent(refinePrompt(0.55))

    expect(setPendingRefine).toHaveBeenCalledTimes(1)
    const updater = setPendingRefine.mock.calls[0][0] as (
      prev: typeof PENDING | null,
    ) => typeof PENDING | null
    const next = updater(PENDING)
    expect(next).toMatchObject({ usagePercent: 70, skippedTurns: 2 })
    expect(invokeMock).toHaveBeenCalledWith('set_session_refine_config', { forceThreshold: 0.7 })
  })

  it('③ 阈值 0.75 已不低于下限 70 → 不写盘（无谓 invoke）', () => {
    const { handlers } = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent(refinePrompt(0.75))

    expect(handlers.setRefineState).toHaveBeenCalledTimes(1)
    expect(invokeMock).not.toHaveBeenCalled()
  })

  // 回归：pending 分支曾用「跳过那一刻」的旧 forceThreshold 快照去 reconcile，
  // 而 autoRaiseForceThreshold 以 `forceThreshold >= minPct` 判「后端已够高」。
  // 用户把线调到 0.80 后快照仍停在 0.50，下一轮 usage 把下限顶到 70% →
  // 误判需要抬升 → 把后端从 0.80 静默改写成 0.70，用户设置被降级。
  it('④ pending 分支以事件权威值为准，不被旧快照带偏（用户设 0.80 → 不降级到 0.70）', () => {
    // PENDING 快照停留在 0.55（= 用户跳过那一刻的值），事件权威值为 0.80
    const { handlers, setPendingRefine } = makeHandlers(PENDING)
    renderHook(() => useEvents(handlers))

    driveEvent(refinePrompt(0.8))

    const updater = setPendingRefine.mock.calls[0][0] as (
      prev: typeof PENDING | null,
    ) => typeof PENDING | null
    const next = updater(PENDING)
    // pendingRefine 必须被刷成后端权威值，否则显示继续停留在旧快照
    expect(next).toMatchObject({ usagePercent: 70, forceThreshold: 0.8 })
    // 权威值 0.80 已高于下限 70 → 无需抬升，绝不能把 0.80 改小
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('⑤ pending 分支：权威值确实低于下限时才抬升（0.50 + usage 70% → 0.70）', () => {
    const { handlers } = makeHandlers(PENDING)
    renderHook(() => useEvents(handlers))

    driveEvent(refinePrompt(0.5))

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('set_session_refine_config', { forceThreshold: 0.7 })
  })
})
