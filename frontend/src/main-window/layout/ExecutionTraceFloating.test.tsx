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

describe('history folding by step', () => {
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

  function makeCalls(n: number): TimelineEntry[] {
    return Array.from({ length: n }, (_, i) => ({
      id: `call-${i + 1}`,
      kind: 'tool_call' as const,
      toolName: `tool_${i + 1}`,
      status: 'success' as const,
    }))
  }

  /** 混合流：每步前挂一段文本 —— 验证切点落在 tool_call 起点、文本整段不劈半 */
  function makeMixedCalls(n: number): TimelineEntry[] {
    const out: TimelineEntry[] = []
    for (let i = 1; i <= n; i++) {
      out.push({ id: `text-${i}`, kind: 'text', text: `段落 ${i}` })
      out.push({
        id: `call-${i}`,
        kind: 'tool_call',
        toolName: `tool_${i}`,
        status: 'success',
      })
    }
    return out
  }

  const panel = (entries: TimelineEntry[]) => (
    <ExecutionTraceFloating
      timeline={entries}
      stepIndex={entries.length}
      progress={{ iteration: 1, max: 200, calls: entries.length }}
      isProcessing={false}
      completed={false}
      expandedCalls={new Set()}
      onToggleExpand={vi.fn()}
      visible
    />
  )

  function renderTimeline(entries: TimelineEntry[]) {
    return render(panel(entries))
  }

  const indicators = (container: HTMLElement) =>
    Array.from(container.querySelectorAll('.tc-option-indicator')).map(el => el.textContent)

  /**
   * 卡片模式滚动容器。jsdom 无布局：scrollHeight 按「已渲染步数 × px」现算 ——
   * 于是「展开前后的高度差」可被精确断言（锚点补偿量的唯一依据）。
   */
  function scrollBodyOf(container: HTMLElement, pxPerStep = 100): HTMLDivElement {
    const el = container.querySelector('.execution-trace-body') as HTMLDivElement
    if (!el) throw new Error('.execution-trace-body 未渲染')
    Object.defineProperty(el, 'scrollHeight', {
      configurable: true,
      get: () => container.querySelectorAll('.tc-option').length * pxPerStep,
    })
    Object.defineProperty(el, 'clientHeight', { configurable: true, value: 400 })
    return el
  }

  /** 模拟「用户上滚到折叠条」：先落远处建立方向，再落进触发带（≤32px） */
  function scrollUpToFoldBar(el: HTMLDivElement, from: number, to: number) {
    el.scrollTop = from
    fireEvent.scroll(el)
    el.scrollTop = to
    fireEvent.scroll(el)
  }

  it('默认档只渲染最新 30 步，折叠条文案以「步」计', () => {
    const { container } = renderTimeline(makeCalls(90))
    expect(screen.getByText('已折叠更早的 60 步 · 上滚展开')).toBeInTheDocument()
    expect(container.querySelectorAll('.tc-option')).toHaveLength(30)
    expect(screen.queryByText('tool_1')).not.toBeInTheDocument()
    expect(screen.getByText('tool_90')).toBeInTheDocument()
  })

  it('折叠态的序号接着数，不从 1 重来', () => {
    const { container } = renderTimeline(makeCalls(90))
    const nums = indicators(container)
    expect(nums).toHaveLength(30)
    expect(nums[0]).toBe('61')
    expect(nums[29]).toBe('90')
  })

  it('切点落在 tool_call 起点：整段文本整体隐藏，不劈半', () => {
    // 40 步 → 默认档 30 步 → 收起最前 10 步；切点 = 第 11 条 tool_call
    const { container } = renderTimeline(makeMixedCalls(40))
    const body = container.querySelector('.execution-trace-body') as HTMLElement
    expect(screen.getByText('已折叠更早的 10 步 · 上滚展开')).toBeInTheDocument()
    expect(container.querySelectorAll('.tc-option')).toHaveLength(30)
    // 折叠条之后的**第一个内容节点**就是 tool_call 卡片 —— 切点落在步骤起点上
    const [, firstContent] = Array.from(body.children)
    expect(firstContent.classList.contains('tc-option')).toBe(true)
    expect(indicators(container)[0]).toBe('11')
    // 折叠区内的文本整段隐藏（不是被截半），可见区内的文本整段保留
    expect(screen.queryByText('段落 11')).not.toBeInTheDocument()
    expect(screen.getByText('段落 12')).toBeInTheDocument()
  })

  it('折叠条点击可展开一档、再点收起回默认档', () => {
    const { container } = renderTimeline(makeCalls(90))
    fireEvent.click(screen.getByText('已折叠更早的 60 步 · 上滚展开'))
    expect(container.querySelectorAll('.tc-option')).toHaveLength(60)
    expect(screen.getByText('收起更早的 60 步')).toBeInTheDocument()
    fireEvent.click(screen.getByText('收起更早的 60 步'))
    expect(container.querySelectorAll('.tc-option')).toHaveLength(30)
    expect(screen.queryByText('tool_1')).not.toBeInTheDocument()
  })

  it('上滚到折叠条自动 +30 步，并按新增高度补偿 scrollTop（视野不跳）', () => {
    const { container } = renderTimeline(makeCalls(90))
    const body = scrollBodyOf(container)
    expect(container.querySelectorAll('.tc-option')).toHaveLength(30)

    scrollUpToFoldBar(body, 300, 10)

    expect(container.querySelectorAll('.tc-option')).toHaveLength(60)
    expect(screen.getByText('收起更早的 60 步')).toBeInTheDocument()
    // 新增 30 步 × 100px = 3000px 插在折叠条之后：scrollTop 同步 +3000，视野停在原处
    expect(body.scrollTop).toBe(10 + 3000)
    // 序号续数：可见区首条是第 31 步
    expect(indicators(container)[0]).toBe('31')
  })

  it('展开补偿产生的程序滚动不连环触发；再上滚才再放一档', () => {
    const { container } = renderTimeline(makeCalls(90))
    const body = scrollBodyOf(container)
    scrollUpToFoldBar(body, 300, 10)
    expect(container.querySelectorAll('.tc-option')).toHaveLength(60)

    // 补偿把 scrollTop 推到带外，浏览器为此派发的 scroll（向下）不得触发续展
    fireEvent.scroll(body)
    // 仍在带外但方向为上：折叠条未贴顶，同样不触发
    body.scrollTop = 100
    fireEvent.scroll(body)
    expect(container.querySelectorAll('.tc-option')).toHaveLength(60)

    // 重新上滚到折叠条 → 再 +30；全部放出后折叠条消失（无更早内容）
    scrollUpToFoldBar(body, 100, 5)
    expect(container.querySelectorAll('.tc-option')).toHaveLength(90)
    expect(screen.queryByText(/已折叠更早的/)).not.toBeInTheDocument()
    expect(screen.queryByText(/收起更早的/)).not.toBeInTheDocument()
  })

  it('步数未超默认档时不出现折叠条，上滚也不续展', () => {
    const { container } = renderTimeline(makeCalls(30))
    expect(screen.queryByText(/已折叠更早的/)).not.toBeInTheDocument()
    const body = scrollBodyOf(container)
    scrollUpToFoldBar(body, 300, 10)
    expect(container.querySelectorAll('.tc-option')).toHaveLength(30)
  })

  it('新一轮开始（步数回落默认档以内）自动收回默认档', () => {
    const utils = renderTimeline(makeCalls(90))
    fireEvent.click(screen.getByText('已折叠更早的 60 步 · 上滚展开'))
    expect(utils.container.querySelectorAll('.tc-option')).toHaveLength(60)

    // 新一轮：timeline 回落到 5 步
    utils.rerender(panel(makeCalls(5)))
    expect(utils.container.querySelectorAll('.tc-option')).toHaveLength(5)
    expect(screen.queryByText(/已折叠更早的/)).not.toBeInTheDocument()
    expect(screen.queryByText(/收起更早的/)).not.toBeInTheDocument()
    expect(indicators(utils.container)).toEqual(['1', '2', '3', '4', '5'])
  })

  it('新一轮即使步数不减少，也收回默认档（换轮次信号）', () => {
    const utils = renderTimeline(makeCalls(90))
    fireEvent.click(screen.getByText('已折叠更早的 60 步 · 上滚展开'))
    expect(utils.container.querySelectorAll('.tc-option')).toHaveLength(60)

    // 新一轮：头部条目 id 变化（call-1 → round2-call-1），步数同为 90
    const nextRound = makeCalls(90).map(e => ({ ...e, id: `round2-${e.id}` }))
    utils.rerender(panel(nextRound))
    expect(utils.container.querySelectorAll('.tc-option')).toHaveLength(30)
    expect(screen.getByText('已折叠更早的 60 步 · 上滚展开')).toBeInTheDocument()
  })

  it('终端模式与卡片模式共用同一折叠条（文案/交互一致）', () => {
    const { container } = renderTimeline(makeCalls(90))
    expect(screen.getByText('已折叠更早的 60 步 · 上滚展开')).toBeInTheDocument()

    fireEvent.click(screen.getByTitle('终端模式'))
    expect(container.querySelector('.execution-terminal-body')).not.toBeNull()
    // 同一节点、同一文案、同一续展逻辑：终端模式下点一下同样放出 30 步
    fireEvent.click(screen.getByText('已折叠更早的 60 步 · 上滚展开'))
    expect(screen.getByText('收起更早的 60 步')).toBeInTheDocument()
    expect(container.querySelectorAll('.term-cmd')).toHaveLength(60)
  })
})
