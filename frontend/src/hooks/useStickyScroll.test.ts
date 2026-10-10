/**
 * useStickyScroll 单测 —— 手势驱动的「贴底跟随」滚动语义（2026-10-09 重写）
 *
 * 行为契约（与任务 B1–B6 一一对应）：
 *  B1 跟随态不由「滚动条位置 / 距底百分比」判定：内容增长（scrollHeight 变大、
 *     scrollTop 不变）派发的 scroll 帧既不解锁也不会误回归；位置变化（哪怕落回贴底）
 *     也不能重新开启跟随。
 *  B2 鼠标滚轮向上 → 立即解锁：只发 wheel、连 scroll 事件都未派发时状态就已改写。
 *  B3 拖拽滚动条 → 解锁（pointerdown 落在滚动条槽上）；且「原生滚动条拖拽不派发
 *     pointer 事件」时有兜底：未对冲的 scrollTop 下降归因用户 → 解锁。
 *  B4 只有「手势结束」且落点贴底才回归：滚轮/键盘 = 末次手势后 120ms 无新手势；
 *     指针 = pointerup。停在中途松手不回归。
 *  B5 流式把滚动条顶起来不得被读成「用户到底」：跟随态不因增长解锁；解锁态不因位置
 *     落回底部而回归。
 *  B6 上滚解锁后可停留在任意位置阅读：后续新增内容不拽回，且无任何「静默 N 秒恢复」。
 *  ⑦ onScroll 返回值语义（调用方 gate 依赖）：程序写入的回响帧 false，用户手势帧 true。
 *  ⑧ nudgeScrollTop 走程序对冲：其后的 scroll 帧不得被读成用户操作。
 *
 * jsdom 局限（盲区）：无布局，scrollHeight/clientHeight/scrollTop 均为注入值；真实
 * 滚动条拖拽的 pointer 事件行为、overflow-anchor: none 的实际效果只能真机验收
 * （见任务报告的人工验证步骤）。这里断言的是状态机与事件归因逻辑。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import type { RefObject } from 'react'
import { useStickyScroll, type StickyScroll } from './useStickyScroll'

/** 容器几何：max = 1000 − 400 = 600，初始 scrollTop 200 → 距底 400px（未贴底） */
const GEO = { scrollHeight: 1000, clientHeight: 400 }

type Result = { current: StickyScroll }

function makeScroller(scrollTop = 200): HTMLDivElement {
  const el = document.createElement('div')
  Object.defineProperty(el, 'scrollHeight', {
    value: GEO.scrollHeight,
    writable: true,
    configurable: true,
  })
  Object.defineProperty(el, 'clientHeight', {
    value: GEO.clientHeight,
    writable: true,
    configurable: true,
  })
  Object.defineProperty(el, 'scrollTop', { value: scrollTop, writable: true, configurable: true })
  return el
}

/** renderHook 不渲染 DOM：hook 内部创建的 scrollRef 需手动挂到元素上。
 *  必须挂进 document —— 手势监听在 window 捕获阶段，离屏元素的事件不会冒泡到 window
 *  （真实 App 中容器恒在 DOM 树内，行为一致）。 */
const attachedScrollers: HTMLDivElement[] = []
function attachScroller(scrollRef: RefObject<HTMLDivElement>): HTMLDivElement {
  const el = makeScroller()
  document.body.appendChild(el)
  attachedScrollers.push(el)
  ;(scrollRef as { current: HTMLDivElement | null }).current = el
  return el
}

/** jsdom 无布局：scrollHeight 是只读 getter，用 defineProperty 覆写模拟内容增长 */
function setScrollHeight(el: HTMLElement, h: number): void {
  Object.defineProperty(el, 'scrollHeight', { value: h, writable: true, configurable: true })
}

function setup() {
  return renderHook((props: { n: number }) => useStickyScroll(props.n), {
    initialProps: { n: 1 },
  })
}

/** followKey 变化（新消息 / 流式 delta）*/
function bumpKey(rerender: (props: { n: number }) => void, n: number): void {
  act(() => {
    rerender({ n })
  })
}

/** 冲刷 rAF 补写并消费浏览器为程序写入派发的 scroll 回响 —— 让一次贴底彻底落定 */
function settle(result: Result): void {
  act(() => {
    vi.advanceTimersByTime(20)
  })
  act(() => {
    result.current.onScroll()
  })
}

/** 跟随着完成一次贴底 */
function followKeyChange(result: Result, rerender: (props: { n: number }) => void, n: number) {
  bumpKey(rerender, n)
  settle(result)
}

const scrollFrame = (result: Result): boolean => {
  let ret = false
  act(() => {
    ret = result.current.onScroll()
  })
  return ret
}

/** 真实滚轮手势：wheel 事件（判定意图）→ 浏览器滚动 → scroll 事件（归因帧） */
function wheelScroll(result: Result, el: HTMLDivElement, deltaY: number): void {
  act(() => {
    el.dispatchEvent(new WheelEvent('wheel', { deltaY, bubbles: true }))
  })
  el.scrollTop = el.scrollTop + deltaY
  scrollFrame(result)
}

function pointerDown(el: HTMLDivElement, clientX: number, clientY: number): void {
  act(() => {
    el.dispatchEvent(new MouseEvent('pointerdown', { clientX, clientY, bubbles: true }))
  })
}

function pointerUpWindow(): void {
  act(() => {
    window.dispatchEvent(new Event('pointerup'))
  })
}

function keyDown(el: HTMLDivElement, key: string): void {
  act(() => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }))
  })
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
  for (const el of attachedScrollers) el.remove()
  attachedScrollers.length = 0
})

describe('useStickyScroll 手势语义', () => {
  it('B1 内容增长改不了跟随态：贴底靠 followKey 驱动，不看距底距离', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)

    followKeyChange(result, rerender, 2)
    expect(el.scrollTop).toBe(600) // 瞬移贴底（max = 1000 − 400）
    expect(result.current.showJumpButton).toBe(false)

    // 流式：内容暴涨把滚动条顶离底部。浏览器不会因此改 scrollTop，即便派发 scroll 帧：
    setScrollHeight(el, 3000)
    expect(scrollFrame(result)).toBe(false) // 静止帧：不按用户操作处理
    expect(result.current.showJumpButton).toBe(false) // 跟随态不被增长改写

    // 跟随态下新内容继续贴底（新 max = 2600）
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(2600)
  })

  it('B1b 位置永远不能重新开启跟随：解锁后落回贴底也不回归', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -120) // 600 → 480，用户上滚解锁
    expect(result.current.showJumpButton).toBe(true)
    act(() => {
      vi.advanceTimersByTime(200) // 手势结束：落点 480 未贴底 → 保持解锁
    })

    // 内容缩短，滚动条此刻恰好落回贴底（旧 80px 容差逻辑会在这里误回归）
    setScrollHeight(el, 700) // max = 300
    el.scrollTop = 300
    scrollFrame(result)
    expect(result.current.showJumpButton).toBe(true) // 位置贴底 ≠ 用户到底
  })

  it('B2 鼠标上滚立即解锁：不等 scroll 事件、不等防抖、不等计时器', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)
    expect(result.current.showJumpButton).toBe(false)

    // 只派发 wheel（紧随的 scroll 事件还没来）：状态必须已经改写
    act(() => {
      el.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }))
    })
    expect(result.current.showJumpButton).toBe(true)

    // 解锁后 followKey 变化不再改 scrollTop
    el.scrollTop = 480 // 浏览器随后执行的滚动
    const frozenAt = el.scrollTop
    act(() => {
      vi.advanceTimersByTime(200) // 手势结束：落点 480 未贴底
    })
    followKeyChange(result, rerender, 3)
    followKeyChange(result, rerender, 4)
    expect(el.scrollTop).toBe(frozenAt)
    expect(result.current.showJumpButton).toBe(true)

    // 无任何「静默 N 秒恢复」：再等 5 分钟也不动
    act(() => {
      vi.advanceTimersByTime(300_000)
    })
    followKeyChange(result, rerender, 5)
    expect(el.scrollTop).toBe(frozenAt)
  })

  it('B3 拖拽滚动条立即解锁，拖拽期间不跟随', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    // pointerdown 落在滚动条槽（clientWidth = 400，按下点 x = 460 越过内容盒）
    pointerDown(el, 460, 50)
    expect(result.current.showJumpButton).toBe(true)

    // 拖拽期间到达的 scroll 事件一律归因用户
    el.scrollTop = 300
    expect(scrollFrame(result)).toBe(true)

    // 拖拽中 followKey 变化不跟随
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(300)

    // 松手停在半途（未贴底）→ 保持解锁
    pointerUpWindow()
    expect(result.current.showJumpButton).toBe(true)
  })

  it('B3b 兜底：无 pointer 事件时，未对冲的 scrollTop 下降即归因用户 → 解锁', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)
    expect(result.current.showJumpButton).toBe(false)

    // 原生滚动条拖拽在部分浏览器不派发 pointer 事件：只有 scrollTop 下降这一个信号
    el.scrollTop = 200
    expect(scrollFrame(result)).toBe(true)
    expect(result.current.showJumpButton).toBe(true)
  })

  it('B4 只有手势结束且落点贴底才回归（滚轮去抖 / 指针松开）', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    // (a) 上滚解锁后停在中途：手势结束不回归
    wheelScroll(result, el, -120) // 600 → 480
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)

    // (b) 向下滚回贴底后手势结束 → 回归
    act(() => {
      el.dispatchEvent(new WheelEvent('wheel', { deltaY: 120, bubbles: true }))
    })
    el.scrollTop = 600
    scrollFrame(result)
    act(() => {
      vi.advanceTimersByTime(200) // 末次手势后 120ms 无新手势 = 手势结束
    })
    expect(result.current.showJumpButton).toBe(false)

    // 回归后 followKey 变化重新贴底（新 max = 1000）
    setScrollHeight(el, 1400)
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(1000)

    // (c) 指针路径：解锁 → 手动拖回底部 → pointerup 才回归
    wheelScroll(result, el, -400) // 1000 → 600
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)
    pointerDown(el, 50, 50) // 内容区按下：不判拖拽槽，但记为指针活跃
    el.scrollTop = 1000 // max
    scrollFrame(result)
    expect(result.current.showJumpButton).toBe(true) // 尚未松手：不回归
    pointerUpWindow()
    expect(result.current.showJumpButton).toBe(false)
  })

  it('B4b 键盘：ArrowUp 解锁，ArrowDown/End 到底后手势结束回归', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    keyDown(el, 'ArrowUp')
    el.scrollTop = 400
    scrollFrame(result)
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)

    keyDown(el, 'End')
    el.scrollTop = 600
    scrollFrame(result)
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(false)
  })

  it('B5 流式顶起滚动条不得被读成「用户到底」：跟随态不误解锁、不误保持', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    // 内容暴涨（滚动条离底很远），但这不是用户操作
    setScrollHeight(el, 8000)
    expect(scrollFrame(result)).toBe(false)
    expect(result.current.showJumpButton).toBe(false) // 仍在跟随

    // 继续跟随到底：新 max = 7600
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(7600)
  })

  it('B6 上滚解锁后可停在任意位置阅读：流式续来也不拽回', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -120) // 600 → 480，停在这里读历史
    act(() => {
      vi.advanceTimersByTime(200)
    })
    const readingAt = el.scrollTop
    expect(result.current.showJumpButton).toBe(true)

    // 连续 5 次流式内容到达：位置纹丝不动
    for (let i = 3; i <= 7; i++) {
      setScrollHeight(el, GEO.scrollHeight + (i - 2) * 1000)
      followKeyChange(result, rerender, i)
    }
    expect(el.scrollTop).toBe(readingAt)

    // 长时间静默也不闪回
    act(() => {
      vi.advanceTimersByTime(600_000)
    })
    expect(el.scrollTop).toBe(readingAt)
    expect(result.current.showJumpButton).toBe(true)
  })

  it('⑦ onScroll 返回值：程序回响帧 false、用户手势帧 true（调用方 gate 依赖）', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)

    bumpKey(rerender, 2) // 程序写入 scrollTop=600，装甲一次待吞
    expect(scrollFrame(result)).toBe(false) // 紧随的这一次 = 程序回响
    expect(scrollFrame(result)).toBe(false) // 静止帧（无手势、delta 0）

    act(() => {
      el.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }))
    })
    el.scrollTop = 480
    expect(scrollFrame(result)).toBe(true) // 用户手势帧
  })

  it('⑧ nudgeScrollTop 是程序写入：其后 scroll 帧不被读成用户操作，冻结态不解', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -120) // 600 → 480，解锁
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      result.current.nudgeScrollTop(50) // 视野锚点补偿
    })
    expect(el.scrollTop).toBe(530)
    expect(scrollFrame(result)).toBe(false) // 程序回响，不参与判定
    expect(result.current.showJumpButton).toBe(true) // 冻结态保持
  })

  it('jumpToBottom / followReset / enterPanel：恢复跟随并瞬移贴底', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -120)
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      result.current.jumpToBottom()
    })
    settle(result)
    expect(el.scrollTop).toBe(600)
    expect(result.current.showJumpButton).toBe(false)

    wheelScroll(result, el, -200) // 600 → 400
    act(() => {
      vi.advanceTimersByTime(200)
    })
    act(() => {
      result.current.enterPanel()
    })
    settle(result)
    expect(el.scrollTop).toBe(600)
    expect(result.current.showJumpButton).toBe(false)
  })
})

/**
 * bottomZonePx（对话窗宽容语义，2026-10-10 修「自动下拉没了」实机回归）
 *
 * 两处根因（都只在对话窗触发）：
 *   ① 内容变矮（草稿气泡被 progress 行替换 / think 块剥离）时浏览器把越界 scrollTop
 *      钳回底部 → 负 delta 被严格版误读成「用户上滚」→ 跟随静默解除且再也回不来；
 *   ② 严格版恢复要「4px 内 + 手势收尾瞬间」双条件，流式下用户滚回底部松手时内容已
 *      又长高一截 → 判定落空，自动下拉永久丢失。
 * 宽容语义：向下滚回区内即恢复（唯一恢复入口）、钳位不误解锁、增长仍不恢复。
 * 严格语义（追踪面板路径）由上方全部用例覆盖 —— 它们都不传选项。
 */
describe('bottomZonePx 对话窗宽容语义', () => {
  function setupZone() {
    return renderHook((props: { n: number }) => useStickyScroll(props.n, { bottomZonePx: 80 }), {
      initialProps: { n: 1 },
    })
  }

  it('内容变矮的收缩钳位不误解锁：仍跟随，后续流式继续贴底', () => {
    const { result, rerender } = setupZone()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)
    expect(el.scrollTop).toBe(600) // max = 1000 − 400

    // 内容变矮 ⇒ 浏览器把越界的 scrollTop 钳回新底部（max = 700 − 400 = 300）
    setScrollHeight(el, 700)
    el.scrollTop = 300
    scrollFrame(result)
    expect(result.current.showJumpButton).toBe(false) // 钳位 ≠ 用户上滚：不得解锁

    // 后续流式继续贴底（新 max = 1400 − 400）
    setScrollHeight(el, 1400)
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(1000)
  })

  it('向下滚回贴底区（80px）即恢复跟随：不必压线 4px、不必等手势收尾', () => {
    const { result, rerender } = setupZone()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -200) // 600 → 400：距底 200 > 80，未进区
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true) // 停在区外：保持解锁

    wheelScroll(result, el, 140) // 400 → 540：距底 60 ≤ 80，进区即恢复
    expect(el.scrollTop).toBe(540)
    expect(result.current.showJumpButton).toBe(false)

    // 恢复后 followKey 变化重新贴底
    setScrollHeight(el, 1400)
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(1000)
  })

  it('上滚停在区内（方向向上）收尾不恢复：不把阅读中的用户拽回', () => {
    const { result, rerender } = setupZone()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -30) // 600 → 570：距底 30 在区内，但方向是向上
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)

    // 流式续来也不拽回
    setScrollHeight(el, 1600)
    followKeyChange(result, rerender, 3)
    expect(el.scrollTop).toBe(570)
  })

  it('内容增长本身不恢复跟随（区内也一样）：向下滚动才是唯一恢复入口', () => {
    const { result, rerender } = setupZone()
    const el = attachScroller(result.current.scrollRef)
    followKeyChange(result, rerender, 2)

    wheelScroll(result, el, -40) // 600 → 560：解锁且停在区内
    act(() => {
      vi.advanceTimersByTime(200)
    })
    expect(result.current.showJumpButton).toBe(true)

    setScrollHeight(el, 5000) // 内容暴涨（增长只改 scrollHeight、不改 scrollTop）
    scrollFrame(result)
    expect(result.current.showJumpButton).toBe(true) // 非用户手势：位置变化不得恢复
  })
})
