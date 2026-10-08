import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TimelineEntry } from '../../core/types'
import { ExecutionTraceFloating } from './ExecutionTraceFloating'

vi.mock('../chat/MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div>{content}</div>,
}))
vi.mock('../../ui/NuphusAvatar', () => ({ NuphusAvatar: () => <span /> }))

function renderAction(overrides: Partial<TimelineEntry> = {}) {
  const entry: TimelineEntry = {
    id: 'desktop-action',
    kind: 'tool_call',
    toolName: 'desktop_semantic_execute',
    status: 'success',
    output: JSON.stringify({
      dispatch_state: 'sent',
      effect: 'unverifiable',
      business_goal_confirmed: false,
    }),
    ...overrides,
  }
  return render(
    <ExecutionTraceFloating
      timeline={[entry]}
      stepIndex={1}
      progress={{ iteration: 1, max: 20, calls: 1 }}
      isProcessing={false}
      completed={false}
      expandedCalls={new Set()}
      onToggleExpand={vi.fn()}
      visible
    />,
  )
}

describe('desktop action execution trace', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'requestAnimationFrame',
      vi.fn(() => 1),
    )
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('shows delivery and uncertain effect instead of a completed chip', () => {
    const { container } = renderAction()
    expect(screen.getByText('已发送 · 效果未确认')).toHaveAttribute(
      'data-action-state',
      'unverifiable',
    )
    expect(container.querySelector('.tc-status-chip')).not.toBeInTheDocument()
    expect(screen.queryByText('完成', { exact: true })).not.toBeInTheDocument()
  })

  it('terminal mode does not display exit 0 as proof of action effect', () => {
    renderAction()
    fireEvent.click(screen.getByTitle('终端模式'))
    expect(screen.getByText('已发送 · 效果未确认')).toBeInTheDocument()
    expect(screen.queryByText('exit 0')).not.toBeInTheDocument()
  })

  it('clarifies that confirmed UI state is not whole-task completion', () => {
    renderAction({ output: '{"dispatch_state":"sent","effect":"confirmed"}' })
    expect(screen.getByText('已发送 · 状态已确认')).toHaveAttribute(
      'title',
      '已确认指定界面状态，不代表整个业务任务已完成。',
    )
  })

  it('preserves a partial native failure instead of a generic error chip', () => {
    renderAction({
      status: 'error',
      output: 'desktop_action_result:{"dispatch_state":"partial","effect":"partial"}',
    })
    expect(screen.getByText('部分发送')).toHaveAttribute('data-dispatch-state', 'partial')
    expect(screen.queryByText('失败', { exact: true })).not.toBeInTheDocument()
  })

  it('does not report completion when an action preview is truncated', () => {
    renderAction({ output: '{"dispatch_state":"sent","receipt":', isTruncated: true })
    expect(screen.getByText('发送状态不明')).toBeInTheDocument()
    expect(screen.queryByText('完成', { exact: true })).not.toBeInTheDocument()
  })

  it('leaves running tools and observation tools on their existing status path', () => {
    const running = renderAction({ status: 'running' })
    expect(running.container.querySelector('.desktop-action-status')).not.toBeInTheDocument()
    expect(running.container.querySelector('.tc-status-chip.running')).toBeInTheDocument()
    running.unmount()
    const observation = renderAction({ toolName: 'desktop_windows_list', output: '[]' })
    expect(observation.container.querySelector('.desktop-action-status')).not.toBeInTheDocument()
    expect(observation.container.querySelector('.tc-status-chip.success')).toBeInTheDocument()
  })
})

/**
 * 贴底跟随（useStickyScroll 复用）—— 面板侧契约：
 *  ① 执行中上翻冻结 + 新步骤到达不闪回（旧「3s debounce 强制滚底」残骸已删），
 *     静默满 15s 宽限才恢复滚底（面板无回底按钮，宽限封顶 15s，不得加大）；
 *  ② 空闲态（isProcessing=false）上翻不排恢复计时：再久也不闪回。
 *
 * jsdom 局限（盲区）：无布局，scrollHeight/clientHeight/scrollTop 全注入；smooth
 * 动画逐帧过程不模拟，断言的是判定逻辑（冻结 / 恢复 / 滚底目标值）。
 */
describe('execution trace sticky scroll (useStickyScroll)', () => {
  let scrollToSpy: ReturnType<typeof vi.fn>

  /** 卡片模式的步骤树滚动容器：按需覆写成贴底 600（距底 0）*/
  function bodyOf(container: HTMLElement): HTMLDivElement {
    const el = bodyElOf(container)
    // jsdom 无布局：距底 0px（1000 - 600 - 400）视为贴底
    Object.defineProperty(el, 'scrollTop', {
      value: 600,
      writable: true,
      configurable: true,
    })
    return el
  }

  /** 只取滚动容器、不覆写几何：保留挂载时的 scrollTop（200 = 距底 400px，未到底）。
   *  进场自动下拉的用例必须用它 —— 覆写成贴底就等于没有「下拉」可等。 */
  function bodyElOf(container: HTMLElement): HTMLDivElement {
    const el = container.querySelector('.execution-trace-body') as HTMLDivElement
    if (!el) throw new Error('.execution-trace-body 未渲染')
    return el
  }

  function renderPanel(timeline: TimelineEntry[], isProcessing: boolean) {
    return render(
      <ExecutionTraceFloating
        timeline={timeline}
        stepIndex={1}
        progress={{ iteration: 1, max: 20, calls: 1 }}
        isProcessing={isProcessing}
        completed={false}
        expandedCalls={new Set()}
        onToggleExpand={vi.fn()}
        visible
      />,
    )
  }

  beforeEach(() => {
    vi.useFakeTimers()
    scrollToSpy = vi.fn()
    Object.defineProperty(window.Element.prototype, 'scrollTo', {
      value: scrollToSpy,
      writable: true,
      configurable: true,
    })
    // 挂载即有几何（否则 enterPanel 在 effect 里量到 scrollHeight=0 → 判「已在底部」，
    // 永远进不了「自动下拉未完成」态，③/③b 就失去被测对象）。scrollTop 设在 200
    // = 距底 400px（未到底），bodyOf 仍按需覆写成贴底 600。
    Object.defineProperty(HTMLDivElement.prototype, 'scrollHeight', {
      value: 1000,
      configurable: true,
    })
    Object.defineProperty(HTMLDivElement.prototype, 'clientHeight', {
      value: 400,
      configurable: true,
    })
    Object.defineProperty(HTMLDivElement.prototype, 'scrollTop', {
      value: 200,
      writable: true,
      configurable: true,
    })
    // rAF 经 setTimeout 驱动：fake timers 下 advanceTimersByTime 即可冲刷
    vi.stubGlobal(
      'requestAnimationFrame',
      (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0) as unknown as number,
    )
    vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  })
  afterEach(() => {
    cleanup()
    vi.useRealTimers()
    vi.unstubAllGlobals()
    delete (window.Element.prototype as unknown as Record<string, unknown>).scrollTo
    delete (HTMLDivElement.prototype as unknown as Record<string, unknown>).scrollHeight
    delete (HTMLDivElement.prototype as unknown as Record<string, unknown>).clientHeight
    delete (HTMLDivElement.prototype as unknown as Record<string, unknown>).scrollTop
  })

  it('① 执行中上翻 + 新步骤不闪回（旧 3s debounce 已删），静默满 15s 宽限才恢复滚底', () => {
    const entry: TimelineEntry = {
      id: 'e1',
      kind: 'tool_call',
      toolName: 'Read',
      status: 'success',
      output: 'ok',
    }
    const utils = renderPanel([entry], true)
    const bodyEl = bodyOf(utils.container)
    // 挂载即跟随 + 模式切换兜底滚底 + enterPanel 进场自动滚底：全部落定后再清零
    // 计数，只看用户交互后的行为（进场自动滚底完成即释放，用户输入随时可介入）
    act(() => {
      vi.advanceTimersByTime(1_200)
    })
    expect(scrollToSpy).toHaveBeenCalled()
    scrollToSpy.mockClear()

    // 用户上翻（距底 400px）：冻结跟随，宽限计时起排
    bodyEl.scrollTop = 200
    fireEvent.scroll(bodyEl)
    act(() => {
      vi.advanceTimersByTime(200)
    })

    // 流式新步骤到达：冻结态不拽回 —— 旧 debounce 会在停手 3s 后强制滚底（闪回）
    const entry2: TimelineEntry = { ...entry, id: 'e2', toolName: 'Write' }
    utils.rerender(
      <ExecutionTraceFloating
        timeline={[entry, entry2]}
        stepIndex={1}
        progress={{ iteration: 1, max: 20, calls: 2 }}
        isProcessing
        completed={false}
        expandedCalls={new Set()}
        onToggleExpand={vi.fn()}
        visible
      />,
    )
    act(() => {
      vi.advanceTimersByTime(5_000) // 早超过旧 debounce 的 3s
    })
    expect(scrollToSpy).not.toHaveBeenCalled() // 不闪回

    act(() => {
      vi.advanceTimersByTime(10_000) // 距上次用户滚动累计 15s
    })
    act(() => {
      vi.advanceTimersByTime(20) // 恢复滚底的 rAF
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(1)
    expect(scrollToSpy).toHaveBeenLastCalledWith({ top: 1000, behavior: 'smooth' })
  })

  it('② 空闲态（isProcessing=false）上翻不排恢复计时：30s 也不闪回', () => {
    const entry: TimelineEntry = {
      id: 'e1',
      kind: 'tool_call',
      toolName: 'Read',
      status: 'success',
      output: 'ok',
    }
    const utils = renderPanel([entry], false)
    const bodyEl = bodyOf(utils.container)
    // 同 ①：等进场宽限（3s）落定再注入用户滚动
    act(() => {
      vi.advanceTimersByTime(3_100)
    })
    scrollToSpy.mockClear()

    bodyEl.scrollTop = 200
    fireEvent.scroll(bodyEl)
    const entry2: TimelineEntry = { ...entry, id: 'e2', toolName: 'Write' }
    utils.rerender(
      <ExecutionTraceFloating
        timeline={[entry, entry2]}
        stepIndex={1}
        progress={{ iteration: 1, max: 20, calls: 2 }}
        isProcessing={false}
        completed={false}
        expandedCalls={new Set()}
        onToggleExpand={vi.fn()}
        visible
      />,
    )
    act(() => {
      vi.advanceTimersByTime(30_000) // 远超 15s 宽限：空闲态根本不排计时
    })
    expect(scrollToSpy).not.toHaveBeenCalled()
  })

  it('③ 进场自动下拉：完成即释放，用户随时可抢控制（无 3s 硬宽限）', () => {
    const entry: TimelineEntry = {
      id: 'e1',
      kind: 'tool_call',
      toolName: 'Read',
      status: 'success',
      output: 'ok',
    }
    // 面板常驻、visible 受控（App.tsx 的 showExecTrace）：先关（return null，无滚动
    // 容器），再开 —— 模拟「打开执行追踪」，isVisible false→true 即 open 来源
    const panel = (visible: boolean, timeline: TimelineEntry[]) => (
      <ExecutionTraceFloating
        timeline={timeline}
        stepIndex={1}
        progress={{ iteration: 1, max: 20, calls: 1 }}
        isProcessing
        completed={false}
        expandedCalls={new Set()}
        onToggleExpand={vi.fn()}
        visible={visible}
      />
    )
    const utils = render(panel(false, [entry]))
    expect(utils.container.querySelector('.execution-trace-body')).toBeNull()

    // 打开：enterPanel —— followReset 立即滚底（展示最新执行态）。
    // 挂载几何就是「距底 400px」，故此刻确实处于「自动下拉未完成」态
    utils.rerender(panel(true, [entry]))
    const bodyEl = bodyElOf(utils.container)
    act(() => {
      vi.advanceTimersByTime(100) // enterPanel 的 rAF + 模式切换兜底的 50ms
    })
    expect(scrollToSpy).toHaveBeenLastCalledWith({ top: 1000, behavior: 'smooth' })
    scrollToSpy.mockClear()

    // 自动下拉中途（向下 delta、未到底）= smooth 动画中间态 → 不判定
    bodyEl.scrollTop = 500
    fireEvent.scroll(bodyEl)
    const entry2: TimelineEntry = { ...entry, id: 'e2', toolName: 'Write' }
    utils.rerender(panel(true, [entry, entry2]))
    act(() => {
      vi.advanceTimersByTime(20)
    })
    expect(scrollToSpy).toHaveBeenCalled() // 新步骤跟随滚底：未被进场滚动误判冻结

    // 下拉完成（滚回 80px 容差内：1000 − 600 − 400 = 0）→ 立即释放判定，不等任何时长
    bodyEl.scrollTop = 600
    fireEvent.scroll(bodyEl)
    // 释放后用户上滚立即冻结：新步骤不再拽回（15s 静默宽限重排）
    bodyEl.scrollTop = 100
    fireEvent.scroll(bodyEl)
    scrollToSpy.mockClear()
    const entry3: TimelineEntry = { ...entry, id: 'e3', toolName: 'Edit' }
    utils.rerender(panel(true, [entry, entry2, entry3]))
    act(() => {
      vi.advanceTimersByTime(2_000)
    })
    expect(scrollToSpy).not.toHaveBeenCalled() // 冻结态不闪回：判定已交还用户
  })

  it('③b 自动下拉期间用户上滚抢控制：立即冻结（不吞用户输入、不等宽限）', () => {
    const entry: TimelineEntry = {
      id: 'e1',
      kind: 'tool_call',
      toolName: 'Read',
      status: 'success',
      output: 'ok',
    }
    const panel = (visible: boolean, timeline: TimelineEntry[]) => (
      <ExecutionTraceFloating
        timeline={timeline}
        stepIndex={1}
        progress={{ iteration: 1, max: 20, calls: 1 }}
        isProcessing
        completed={false}
        expandedCalls={new Set()}
        onToggleExpand={vi.fn()}
        visible={visible}
      />
    )
    const utils = render(panel(false, [entry]))
    utils.rerender(panel(true, [entry]))
    const bodyEl = bodyElOf(utils.container)
    act(() => {
      vi.advanceTimersByTime(100)
    })
    scrollToSpy.mockClear()

    // 进场自动滚底还没完成，用户就向上滚（读历史）→ 立刻冻结，无需等任何时间窗
    // （旧实现这里会等满 3s 硬宽限，用户的滚动被整段吞掉）
    bodyEl.scrollTop = 150
    fireEvent.scroll(bodyEl)
    bodyEl.scrollTop = 100
    fireEvent.scroll(bodyEl)
    const entry2: TimelineEntry = { ...entry, id: 'e2', toolName: 'Write' }
    utils.rerender(panel(true, [entry, entry2]))
    act(() => {
      vi.advanceTimersByTime(100)
    })
    expect(scrollToSpy).not.toHaveBeenCalled() // 用户已接管：不拽回
  })
})

describe('history folding', () => {
  beforeEach(() => {
    vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1))
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  function makeCalls(n: number): TimelineEntry[] {
    return Array.from({ length: n }, (_, i) => ({
      id: `call-${i + 1}`,
      kind: 'tool_call' as const,
      toolName: `tool_${i + 1}`,
      status: 'success' as const,
    }))
  }

  function renderTimeline(entries: TimelineEntry[]) {
    return render(
      <ExecutionTraceFloating
        timeline={entries}
        stepIndex={entries.length}
        progress={{ iteration: 1, max: 200, calls: entries.length }}
        isProcessing={false}
        completed={false}
        expandedCalls={new Set()}
        onToggleExpand={vi.fn()}
        visible
      />,
    )
  }

  const indicators = (container: HTMLElement) =>
    Array.from(container.querySelectorAll('.tc-option-indicator')).map(el => el.textContent)

  it('超阈值时只渲染最近 40 条，更早的收进一行摘要', () => {
    const { container } = renderTimeline(makeCalls(60))
    expect(screen.getByText('已折叠更早的 20 项 · 点击展开')).toBeInTheDocument()
    expect(container.querySelectorAll('.tc-option')).toHaveLength(40)
    expect(screen.queryByText('tool_1')).not.toBeInTheDocument()
    expect(screen.getByText('tool_60')).toBeInTheDocument()
  })

  it('折叠态的序号接着数，不从 1 重来', () => {
    const { container } = renderTimeline(makeCalls(60))
    const nums = indicators(container)
    expect(nums).toHaveLength(40)
    expect(nums[0]).toBe('21')
    expect(nums[39]).toBe('60')
  })

  it('展开后全部条目回归，序号仍连续', () => {
    const { container } = renderTimeline(makeCalls(60))
    fireEvent.click(screen.getByText('已折叠更早的 20 项 · 点击展开'))
    expect(container.querySelectorAll('.tc-option')).toHaveLength(60)
    expect(screen.getByText('tool_1')).toBeInTheDocument()
    const nums = indicators(container)
    expect(nums[0]).toBe('1')
    expect(nums[40]).toBe('41')
    expect(nums[59]).toBe('60')
  })

  it('展开后可再收起，回到折叠态', () => {
    const { container } = renderTimeline(makeCalls(60))
    fireEvent.click(screen.getByText('已折叠更早的 20 项 · 点击展开'))
    fireEvent.click(screen.getByText('收起更早的 20 项'))
    expect(container.querySelectorAll('.tc-option')).toHaveLength(40)
    expect(screen.queryByText('tool_1')).not.toBeInTheDocument()
  })

  it('条目未超阈值时不出现折叠行', () => {
    renderTimeline(makeCalls(40))
    expect(screen.queryByText(/已折叠更早的/)).not.toBeInTheDocument()
  })
})
