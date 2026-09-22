/**
 * 页头 island 轻反馈层契约 —— 队列串行 / 停留时长 / hover 暂停 / 点击关闭 /
 * 相位→图标映射 / prefers-reduced-motion。
 *
 * 回归背景：改造前所有轻反馈只有 HUD 独立窗口一条出路（`hud_update`），
 * 且是「追加覆盖」式呈现——前一条没读完就被下一条顶掉。island 要同时修掉
 * 观感（窗口内、页头中央）与行为（队列串行、可读完、可点掉、可播报）。
 *
 * 这里断言的都是可观察结果：播报区的 DOM、胶囊类名、以及「在什么时刻发生什么」。
 */
import { act, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AppIsland } from '../ui/AppIsland'
import {
  ISLAND_DWELL_MS,
  ISLAND_EXIT_MS,
  type AppFeedback,
  getAppIslandSnapshot,
  setAppFocused,
  showAppFeedback,
} from '../ui/islandChannel'
import { IconAlertCircle, IconAlertTriangle, IconCheck, IconInfo } from '../ui/Icons'

/** 播报区当前展示的文案（无提示时为 null） */
function shownText(): string | null {
  return document.querySelector('.app-pill-text')?.textContent ?? null
}

function pill(): HTMLElement {
  const el = document.querySelector<HTMLElement>('.app-island-pill')
  if (!el) throw new Error('island 胶囊未渲染')
  return el
}

/** 从**真实**入口推一条提示（前台 → island） */
function push(message: string, type: AppFeedback) {
  act(() => {
    setAppFocused(true)
    showAppFeedback(message, type)
  })
}

/** 排空队列：一条提示要吃掉「停留 + 退场」两段时间 */
function drainIsland() {
  for (let i = 0; i < 20 && getAppIslandSnapshot().toast !== null; i++) {
    act(() => vi.advanceTimersByTime(10_000))
  }
}

/** stub 系统「减少动态效果」偏好（jsdom 无 matchMedia，须先补上全局） */
function stubReducedMotion(matches: boolean) {
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }))
}

beforeEach(() => {
  vi.useFakeTimers()
  stubReducedMotion(false)
})

afterEach(() => {
  drainIsland()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('AppIsland：队列与停留', () => {
  it('同时两条提示 → 队列串行，一次只渲染一条', () => {
    render(<AppIsland />)
    act(() => {
      setAppFocused(true)
      showAppFeedback('第一条', 'info')
      showAppFeedback('第二条', 'success')
    })

    expect(shownText()).toBe('第一条')
    expect(document.querySelectorAll('.app-island-pill')).toHaveLength(1)

    // 第一条退场结束前，第二条不得进场（禁止堆叠）
    act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS.info))
    expect(pill().className).toContain('app-island-pill--exit')
    expect(shownText()).toBe('第一条')

    act(() => vi.advanceTimersByTime(ISLAND_EXIT_MS))
    expect(shownText()).toBe('第二条')
    expect(pill().className).toContain('app-pill--success')
  })

  it('自动消失时长按相位区分：info 3.2s，error 5s', () => {
    render(<AppIsland />)

    push('已中断', 'info')
    act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS.info - 1))
    expect(pill().className).not.toContain('app-island-pill--exit')

    act(() => vi.advanceTimersByTime(1))
    expect(pill().className).toContain('app-island-pill--exit')
    act(() => vi.advanceTimersByTime(ISLAND_EXIT_MS))
    expect(shownText()).toBeNull()

    push('执行失败', 'error')
    // error 的停留更长：用 info 的时长推不动它
    act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS.info))
    expect(pill().className).not.toContain('app-island-pill--exit')

    act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS.error - ISLAND_DWELL_MS.info + 1))
    expect(pill().className).toContain('app-island-pill--exit')
  })

  it('hover 暂停自动消失计时，移开后按剩余时长继续', () => {
    render(<AppIsland />)
    push('慢慢读', 'info')

    act(() => vi.advanceTimersByTime(1000))
    fireEvent.mouseEnter(pill())
    act(() => vi.advanceTimersByTime(30_000))
    expect(shownText()).toBe('慢慢读')

    fireEvent.mouseLeave(pill())
    act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS.info - 1001))
    expect(pill().className).not.toContain('app-island-pill--exit')

    act(() => vi.advanceTimersByTime(1))
    expect(pill().className).toContain('app-island-pill--exit')
  })

  it('点击立即关闭（先退场再消失），退场后轮到下一条', () => {
    render(<AppIsland />)
    act(() => {
      setAppFocused(true)
      showAppFeedback('点我就关', 'warning')
      showAppFeedback('下一条', 'info')
    })

    fireEvent.click(pill())
    expect(pill().className).toContain('app-island-pill--exit')

    act(() => vi.advanceTimersByTime(ISLAND_EXIT_MS))
    expect(shownText()).toBe('下一条')
  })
})

describe('AppIsland：语义与可达性', () => {
  it('相位 → 图标映射（图标经 ui/Icons.tsx 出口）', () => {
    const cases: Array<[AppFeedback, typeof IconInfo]> = [
      ['info', IconInfo],
      ['success', IconCheck],
      ['warning', IconAlertTriangle],
      ['error', IconAlertCircle],
    ]
    render(<AppIsland />)

    for (const [phase, Icon] of cases) {
      const probe = render(<Icon size={14} />)
      const expected = probe.container.querySelector('svg')?.innerHTML
      probe.unmount()

      push('相位用例', phase)
      expect(pill().className).toContain(`app-pill--${phase}`)
      expect(document.querySelector('.app-pill-icon')?.innerHTML).toBe(expected)
      act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS[phase] + ISLAND_EXIT_MS))
    }
  })

  it('播报区常驻，错误用 assertive、其余 polite；文案只出现在文本节点', () => {
    render(<AppIsland />)
    const region = document.querySelector('.app-island')
    expect(region?.getAttribute('role')).toBe('status')
    expect(region?.getAttribute('aria-live')).toBe('polite')

    push('执行失败', 'error')
    expect(document.querySelector('.app-island')?.getAttribute('aria-live')).toBe('assertive')
    expect(shownText()).toBe('执行失败')
    // 语义靠类名 + 图标，不靠整块彩色底：胶囊上没有内联背景色
    expect(pill().getAttribute('style')).toBeNull()
  })

  it('活跃态（info）才呼吸；一次性结果不持续脉动', () => {
    render(<AppIsland />)

    push('进行中', 'info')
    expect(pill().className).toContain('app-island-pill--active')

    act(() => vi.advanceTimersByTime(ISLAND_DWELL_MS.info + ISLAND_EXIT_MS))
    push('已完成', 'success')
    expect(pill().className).not.toContain('app-island-pill--active')
  })
})

describe('AppIsland：prefers-reduced-motion', () => {
  it('命中时不挂呼吸、容器带 --still（CSS 位移/动画随之关闭）', () => {
    stubReducedMotion(true)
    render(<AppIsland />)
    push('少动一点', 'info')

    expect(document.querySelector('.app-island')?.className).toContain('app-island--still')
    // 活跃态没有呼吸类：无脉动
    expect(pill().className).not.toContain('app-island-pill--active')
    // 相位语义不受影响（图标/指示点照常按 info 上色）
    expect(pill().className).toContain('app-pill--info')
  })

  it('系统偏好翻转（用户在系统设置里改）后立即生效', () => {
    const listeners = new Set<() => void>()
    let matches = false
    vi.stubGlobal('matchMedia', (query: string) => ({
      get matches() {
        return matches
      },
      media: query,
      onchange: null,
      addEventListener: (_: string, cb: () => void) => listeners.add(cb),
      removeEventListener: (_: string, cb: () => void) => listeners.delete(cb),
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }))

    render(<AppIsland />)
    push('进行中', 'info')
    expect(pill().className).toContain('app-island-pill--active')

    matches = true
    act(() => listeners.forEach(cb => cb()))
    expect(document.querySelector('.app-island')?.className).toContain('app-island--still')
    expect(pill().className).not.toContain('app-island-pill--active')
  })
})
