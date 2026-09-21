import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ApiHealthBadge, initialApiHealthState } from '../main-window/chat/ApiHealthBadge'
import type { ApiHealthState } from '../core/types'

/**
 * api-health rail 的**紧凑实时状态标签**契约。
 *
 * 回归背景：rail 此前只在 degraded / offline 时显示 4 字状态词（「连接不稳定」），
 * 且完全没用上事件带来的重试进度 —— 用户只看到圆点闪动（连击波动），
 * 不知道该不该等、还要等几次。现在改为最短词 + 结构化数字：`retry 1/3`。
 *
 * 契约：
 * 1. 重试中显示 `retry 当前/上限`（数字来自事件字段 attempt / max_attempts，不解析文案）；
 * 2. offline 优先于「已过期的重试进度」（断了就别再报第几次重试）；
 * 3. degraded 无重试进度时退回单词，不编造数字；
 * 4. 正常态（stable / unknown）不渲染标签，保持极简圆点。
 */
function state(overrides: Partial<ApiHealthState> = {}): ApiHealthState {
  return { ...initialApiHealthState(), status: 'degraded', ...overrides }
}

/** rail 上的标签文本（未渲染时为 null） */
function railLabel(): string | null {
  return document.querySelector('.api-health-label')?.textContent ?? null
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
    expect(document.querySelector('.api-health-badge')).toBeNull()
    expect(railLabel()).toBeNull()
    stable.unmount()

    render(<ApiHealthBadge state={state({ status: 'unknown' })} compact />)
    expect(document.querySelector('.api-health-badge')).toBeNull()
    expect(railLabel()).toBeNull()
  })

  it('单次重试（尚未判定 degraded，status 仍 stable）也要显示进度', () => {
    render(
      <ApiHealthBadge
        state={state({ status: 'stable', retry: { attempt: 1, max: 3, at: Date.now() } })}
        compact
      />,
    )

    expect(railLabel()).toBe('retry 1/3')
  })
})
