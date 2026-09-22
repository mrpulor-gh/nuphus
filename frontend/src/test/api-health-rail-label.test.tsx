import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ApiHealthBadge, initialApiHealthState } from '../main-window/chat/ApiHealthBadge'
import type { ApiHealthState } from '../core/types'

/**
 * api-health rail 的**紧凑实时状态标签 + 状态语义图标**契约。
 *
 * 回归背景：
 * - rail 此前只在 degraded / offline 时显示 4 字状态词（「连接不稳定」），且完全没用上
 *   事件带来的重试进度 —— 用户只看到圆点闪动，不知道该不该等、还要等几次。
 *   现在改为最短词 + 结构化数字：`retry 1/3`。
 * - 图标此前是「实心圆点 + 光晕」，与输入框旁的 mode 圆点无法区分。现改为状态语义图标：
 *   connecting / retry = 旋转加载环，offline = 断裂环，degraded = 同心双弧。
 *
 * 契约：
 * 1. 重试中显示 `retry 当前/上限`（数字来自事件字段 attempt / max_attempts，不解析文案）；
 * 2. offline 优先于「已过期的重试进度」（断了就别再报第几次重试）；
 * 3. degraded 无重试进度时退回单词，不编造数字；
 * 4. 正常态（stable / unknown）不渲染标签，保持极简；
 * 5. 存在重试进度时（哪怕 status 仍是 stable）图标必须是**在途加载环**，
 *    即 `.api-health-connecting` + `.api-health-arc`，而不是实心圆点。
 */
function state(overrides: Partial<ApiHealthState> = {}): ApiHealthState {
  return { ...initialApiHealthState(), status: 'degraded', ...overrides }
}

/** rail 上的标签文本（未渲染时为 null） */
function railLabel(): string | null {
  return document.querySelector('.api-health-label')?.textContent ?? null
}

/** rail 上的徽标根元素（未渲染时为 null） */
function badge(): Element | null {
  return document.querySelector('.api-health-badge')
}

/** rail 图标正在渲染加载环（旋转弧存在） */
function hasLoadingRing(): boolean {
  return !!document.querySelector('.api-health-icon .api-health-arc')
}

describe('api-health rail：实时紧凑状态标签', () => {
  it('重试中 → 显示结构化进度 retry 1/3', () => {
    render(
      <ApiHealthBadge state={state({ retry: { attempt: 1, max: 3, at: Date.now() } })} compact />,
    )

    expect(railLabel()).toBe('retry 1/3')
  })

  it('offline 优先于重试进度（连接已断，不再报第几次重试）', () => {
    render(
      <ApiHealthBadge
        state={state({ status: 'offline', retry: { attempt: 2, max: 3, at: Date.now() } })}
        compact
      />,
    )

    expect(railLabel()).toBe('offline')
  })

  it('degraded 无重试进度 → 退回单词，不编造数字', () => {
    render(<ApiHealthBadge state={state({ retry: null })} compact />)

    expect(railLabel()).toBe('degraded')
  })

  it('connecting → 短词直述（不带「正在 / 连接」冗语）', () => {
    render(<ApiHealthBadge state={state({ status: 'connecting' })} compact />)

    expect(railLabel()).toBe('connecting')
  })

  it('正常态（stable / unknown）→ 整块连接 UI 不渲染（连状态圆点也没有）', () => {
    const stable = render(<ApiHealthBadge state={state({ status: 'stable' })} compact />)
    expect(badge()).toBeNull()
    expect(railLabel()).toBeNull()
    stable.unmount()

    render(<ApiHealthBadge state={state({ status: 'unknown' })} compact />)
    expect(badge()).toBeNull()
    expect(railLabel()).toBeNull()
  })

  // 2026-09-22 契约修正：原用例断言「status 仍 stable 但 retry 存在 → 永久显示 retry 1/3」，
  // 那固化了 useEvents.observeStable 的短路缺陷（单次重试后 status 从未离开 stable，
  // 该分支按 status 一刀切返回 → retry 永不清空，`retry 1/3` 永久驻留）。
  // 适配后的真实契约：**存在重试进度时，图标必须是旋转加载环**（不是实心圆点），
  // 而「重试成功后清空 retry」由 useEvents 状态机负责（见下方状态机级用例）。
  it('重试中（status 仍 stable）→ 图标是在途加载环，与 mode 实心圆点区分', () => {
    render(
      <ApiHealthBadge
        state={state({ status: 'stable', retry: { attempt: 1, max: 3, at: Date.now() } })}
        compact
      />,
    )

    expect(railLabel()).toBe('retry 1/3')
    expect(badge()?.classList.contains('api-health-connecting')).toBe(true)
    expect(hasLoadingRing()).toBe(true)
  })

  it('connecting → 图标是在途加载环（非实心圆点）', () => {
    render(<ApiHealthBadge state={state({ status: 'connecting' })} compact />)

    expect(badge()?.classList.contains('api-health-connecting')).toBe(true)
    expect(hasLoadingRing()).toBe(true)
  })

  it('offline → 图标是断裂环（无 loading 弧）', () => {
    render(<ApiHealthBadge state={state({ status: 'offline', retry: null })} compact />)

    expect(badge()?.classList.contains('api-health-offline')).toBe(true)
    expect(document.querySelector('.api-health-icon .api-health-break')).not.toBeNull()
    expect(hasLoadingRing()).toBe(false)
  })

  it('degraded → 图标是同心双弧（外圈满弧 + 内圈短弧），非 loading 弧', () => {
    render(<ApiHealthBadge state={state({ status: 'degraded', retry: null })} compact />)

    expect(badge()?.classList.contains('api-health-degraded')).toBe(true)
    expect(document.querySelector('.api-health-icon .api-health-arc-inner')).not.toBeNull()
    expect(hasLoadingRing()).toBe(false)
  })
})
