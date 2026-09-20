import { describe, expect, it, vi } from 'vitest'
import { handleExternalAnchorClick } from './externalLink'

/** 构造「锚点内的子节点」场景：与真实点击 target 一致（点击常落在 span 上）。 */
function anchorWithChild(href: string, target?: string) {
  const a = document.createElement('a')
  a.setAttribute('href', href)
  if (target) a.setAttribute('target', target)
  const span = document.createElement('span')
  span.textContent = 'click me'
  a.appendChild(span)
  document.body.appendChild(a)
  return span
}

describe('handleExternalAnchorClick 外链点击接管', () => {
  it('target="_blank" 的 http(s) 外链 → 接管并回调 URL（点击落在子节点上也生效）', () => {
    const open = vi.fn()
    const span = anchorWithChild('https://github.com/mrpulor-gh/nuphus', '_blank')

    expect(handleExternalAnchorClick(span, open)).toBe(true)
    expect(open).toHaveBeenCalledExactlyOnceWith('https://github.com/mrpulor-gh/nuphus')
  })

  it('站内路由 / 锚点 / mailto / 无 target 的链接一律放行原行为', () => {
    const open = vi.fn()
    const cases: Array<[string, string | undefined]> = [
      ['/settings', undefined],
      ['#section', undefined],
      ['mailto:a@b.com', '_blank'],
      ['https://example.com', undefined], // 有 href 但非 _blank：不属本次接管范围
    ]

    for (const [href, target] of cases) {
      const span = anchorWithChild(href, target)
      expect(handleExternalAnchorClick(span, open)).toBe(false)
    }
    expect(open).not.toHaveBeenCalled()
  })

  it('非锚点目标（普通元素 / null）→ 不接管', () => {
    const open = vi.fn()
    const div = document.createElement('div')
    document.body.appendChild(div)

    expect(handleExternalAnchorClick(div, open)).toBe(false)
    expect(handleExternalAnchorClick(null, open)).toBe(false)
    expect(open).not.toHaveBeenCalled()
  })
})
