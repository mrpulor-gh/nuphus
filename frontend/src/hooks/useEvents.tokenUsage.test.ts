/**
 * useEvents × token_usage 累加口径单测
 *
 * 钉死：本轮 turnMeta 的 token 维度**只累加单次调用用量源**（exec / workflow）。
 * main 源是「主上下文占用」快照（input=会话规模、cache=哨兵 0xffffffff、tps/ttft=None），
 * 旧实现把它一并累加，造成三处读数污染：
 *   ① 上下文规模被翻倍计进本轮 inputTokens
 *   ② 哨兵 4294967295 被计进 cacheHitTokens
 *   ③ exec 刚写进的 genTps / ttftMs 被 main 的 undefined 覆盖（ctx 弹窗速度行消失）
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import type { NuphusEvent, TurnMeta } from '../core/types'
import type { EventHandlers } from './useEvents'
import { useEvents } from './useEvents'

const { listenMock } = vi.hoisted(() => ({ listenMock: vi.fn() }))
vi.mock('../core/bridge', () => ({
  invoke: vi.fn(),
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

/** main 源哨兵：后端「本事件只承载上下文占用、不带缓存读数」的标记 */
const SENTINEL = 0xffffffff

function makeHandlers() {
  const handlers = {
    refs: {
      streamingMsgId: { current: 'msg-1' },
      lastStreamingMsgId: { current: null },
      executionActiveRef: { current: true },
      processingRef: { current: false },
      turnMetaTokensRef: { current: {} as TurnMeta },
      interruptedRef: { current: false },
      stickyFollowResetRef: { current: vi.fn() },
    },
    setApiHealth: vi.fn(),
    setExecutionStage: vi.fn(),
    setDismissThinking: vi.fn(),
    setRefineState: vi.fn(),
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
  return handlers
}

function driveEvent(event: NuphusEvent, seq = 1): void {
  const call = listenMock.mock.calls.find(c => c[0] === 'nuphus-event')
  if (!call) throw new Error('nuphus-event listener 未注册')
  act(() => {
    ;(call[1] as (payload: unknown) => void)({ seq, event })
  })
}

/** 按 React setState 语义把 updater 依次应用到本地状态上 */
function foldTurnMeta(handlers: EventHandlers, initial: TurnMeta | null = null): TurnMeta | null {
  let state = initial
  for (const [arg] of vi.mocked(handlers.setTurnMeta).mock.calls) {
    state = typeof arg === 'function' ? (arg as (prev: TurnMeta | null) => TurnMeta)(state) : arg
  }
  return state
}

describe('useEvents token_usage 累加口径', () => {
  beforeEach(() => {
    listenMock.mockClear()
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('① exec 源（单次调用用量）累加进本轮 turnMeta', () => {
    const handlers = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent(
      {
        type: 'token_usage',
        input_tokens: 130801,
        output_tokens: 199,
        cache_hit_tokens: 51200,
        source: 'exec',
        gen_tps: 86.6,
        ttft_ms: 3616,
      },
      1,
    )
    driveEvent(
      {
        type: 'token_usage',
        input_tokens: 132000,
        output_tokens: 120,
        cache_hit_tokens: 60000,
        source: 'exec',
        gen_tps: 90.1,
        ttft_ms: 3200,
      },
      2,
    )

    const meta = foldTurnMeta(handlers)
    expect(meta?.inputTokens).toBe(130801 + 132000)
    expect(meta?.outputTokens).toBe(199 + 120)
    expect(meta?.cacheHitTokens).toBe(51200 + 60000)
    // 后一次调用的速度覆盖前一次（取最后一次为代表）
    expect(meta?.genTps).toBe(90.1)
    expect(meta?.ttftMs).toBe(3200)
    // 同步可读基准一致（气泡分发依赖它）
    expect(handlers.refs.turnMetaTokensRef.current.inputTokens).toBe(130801 + 132000)
  })

  it('② main 源（上下文占用 + 哨兵）不计入本轮 turnMeta', () => {
    const handlers = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent(
      {
        type: 'token_usage',
        input_tokens: 130801,
        output_tokens: 199,
        cache_hit_tokens: 51200,
        source: 'exec',
        gen_tps: 86.6,
        ttft_ms: 3616,
      },
      1,
    )
    driveEvent(
      {
        type: 'token_usage',
        input_tokens: 133000,
        output_tokens: 0,
        cache_hit_tokens: SENTINEL,
        source: 'main',
      },
      2,
    )

    const meta = foldTurnMeta(handlers)
    // 上下文规模不得翻倍计进本轮；哨兵不得计进 cacheHit；tps/ttft 不被 undefined 覆盖
    expect(meta?.inputTokens).toBe(130801)
    expect(meta?.cacheHitTokens).toBe(51200)
    expect(meta?.genTps).toBe(86.6)
    expect(meta?.ttftMs).toBe(3616)
    // main 事件仍归 main 槽（上下文进度条需要它）
    expect(handlers.setMainTokenUsage).toHaveBeenCalledTimes(1)
    expect(handlers.setExecTokenUsage).toHaveBeenCalledTimes(1)
  })

  it('③ 空闲态（executionActiveRef=false）事件不计入本轮', () => {
    const handlers = makeHandlers()
    handlers.refs.executionActiveRef.current = false
    renderHook(() => useEvents(handlers))

    driveEvent({
      type: 'token_usage',
      input_tokens: 999,
      output_tokens: 9,
      cache_hit_tokens: 100,
      source: 'exec',
    })

    expect(handlers.setTurnMeta).not.toHaveBeenCalled()
    expect(handlers.refs.turnMetaTokensRef.current).toEqual({})
  })
})
