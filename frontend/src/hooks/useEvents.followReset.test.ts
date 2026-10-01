/**
 * useEvents × 贴底跟随 followReset 接线单测
 *
 * 钉死两个调用点（任务验收项，useStickyScroll 的 hook 侧语义另见 useStickyScroll.test）：
 *  ① execution_started（新轮次，非 refine 路径）→ stickyFollowResetRef.current() 被调
 *     一次：恢复跟随后续流式（用户上一轮的上翻冻结位置不延续到新轮次）；同轮重复事件
 *     被 executionActiveRef 守卫去重，不重复拉；
 *  ② execution_completed（非 refine 路径）→ 补拉一次：完成任务瞬间下拉展示成果
 *     （非执行态唯一的自动拉 —— 用户上翻 + 空闲不会被拉，二者互不打扰）。
 *
 * 其余事件行为不在本文件范围：这里只驱动 NuphusEvent，断言经 ref 到达 ChatPanel 内
 * useStickyScroll.followReset 的调用次数（App → useEvents → ref → ChatPanel 全链路）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import type { NuphusEvent } from '../core/types'
import type { EventHandlers } from './useEvents'
import { useEvents } from './useEvents'

// listen mock 需在 vi.mock 工厂（提升执行）里引用 —— vi.hoisted 保证先于工厂求值
const { listenMock } = vi.hoisted(() => ({ listenMock: vi.fn() }))
vi.mock('../core/bridge', () => ({
  invoke: vi.fn(),
  listen: (name: string, handler: (payload: unknown) => void) => {
    listenMock(name, handler)
    return Promise.resolve(() => {})
  },
}))
// jsdom 无 Tauri 运行时 / 音视频通道：按路径 mock，避免 import 期或执行期触碰真实 API
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ setFocus: vi.fn(), setAlwaysOnTop: vi.fn(), minimize: vi.fn() }),
}))
vi.mock('../ui/sound', () => ({ playUiSound: vi.fn() }))
vi.mock('../ui/islandChannel', () => ({
  showAppFeedbackByHudPhase: vi.fn(),
  showAppFeedback: vi.fn(),
}))

/** followReset 的替身：断言「事件 → ChatPanel 贴底跟随」直连 ref 的调用次数 */
function makeHandlers() {
  const followReset = vi.fn()
  const handlers = {
    refs: {
      streamingMsgId: { current: 'msg-1' },
      lastStreamingMsgId: { current: null },
      executionActiveRef: { current: false },
      processingRef: { current: false },
      turnMetaTokensRef: { current: {} },
      interruptedRef: { current: false },
      stickyFollowResetRef: { current: followReset },
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
    setExecPhase: vi.fn(),
    setMessages: vi.fn(),
    setProgress: vi.fn(),
    setPauseState: vi.fn(),
    setMood: vi.fn(),
    addMessage: vi.fn(),
  } as unknown as EventHandlers
  return { handlers, followReset }
}

/** 经 listen mock 拿到 nuphus-event handler，按后端帧格式（{ seq, event }）驱动 */
function driveEvent(event: NuphusEvent, seq = 1): void {
  const call = listenMock.mock.calls.find(c => c[0] === 'nuphus-event')
  if (!call) throw new Error('nuphus-event listener 未注册')
  act(() => {
    ;(call[1] as (payload: unknown) => void)({ seq, event })
  })
}

describe('useEvents followReset 接线', () => {
  beforeEach(() => {
    listenMock.mockClear()
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('① execution_started → followReset 一次（新轮次恢复跟随后续流式），同轮重复事件去重', () => {
    const { handlers, followReset } = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent({
      type: 'execution_started',
      step_index: 1,
      goal: '整理会议纪要',
      tools: [],
      source: 'user',
    })

    expect(followReset).toHaveBeenCalledTimes(1)
    // executionActiveRef 守卫：同一轮的迟到重复 started 不再触发（防流式期反复拉底）
    driveEvent(
      {
        type: 'execution_started',
        step_index: 1,
        goal: '整理会议纪要',
        tools: [],
        source: 'user',
      },
      2,
    )
    expect(followReset).toHaveBeenCalledTimes(1)
  })

  it('② execution_completed → followReset 一次（完成任务瞬间补拉展示成果）', () => {
    const { handlers, followReset } = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent({
      type: 'execution_completed',
      step_index: 1,
      output: {
        step_index: 1,
        result_message: '已完成',
        artifacts: [],
        tool_calls_count: 0,
      },
      total_duration_ms: 1200,
      total_calls: 1,
    })

    expect(followReset).toHaveBeenCalledTimes(1)
    // 补齐断言：本次事件同时把执行态收敛为 finalizing（followReset 紧邻其放置）
    expect(handlers.setExecutionStage).toHaveBeenCalledWith('finalizing')
  })

  it('③ 其它事件不触发 followReset（只有新轮次 / 完成两个时机拉）', () => {
    const { handlers, followReset } = makeHandlers()
    renderHook(() => useEvents(handlers))

    driveEvent({ type: 'llm_text_delta', text: 'hi', is_thinking: false, from_task: false }, 1)
    driveEvent(
      { type: 'execution_progress', iteration: 1, max_iterations: 20, tool_calls_so_far: 1 },
      2,
    )
    driveEvent({ type: 'execution_paused', action_id: 'a-1' }, 3)

    expect(followReset).not.toHaveBeenCalled()
  })
})
