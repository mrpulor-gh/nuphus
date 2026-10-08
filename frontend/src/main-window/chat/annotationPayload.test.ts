import { describe, expect, it } from 'vitest'
import { buildAnnotationRef, serializeAnnotations } from './annotationPayload'
import type { AnnotationMessage } from './annotationPayload'

// P2 序列化契约：Leader 上下文载荷 = file + 逐条 css_selector + comment（充分信息）
// + rect/dpr/outer_html_snippet（辅助，均已截断）。走 quote 引用通道注入。

const MSG: AnnotationMessage = {
  type: 'nuphus:annotations',
  file: 'demo.html',
  annotations: [
    {
      css_selector: 'body > div.a:nth-child(2) > p',
      outer_html_snippet: '<p>标题</p>',
      rect: { x: 12, y: 34, w: 120, h: 24 },
      dpr: 1.25,
      comment: '标题改成蓝色',
    },
    {
      css_selector: 'footer:nth-child(3)',
      outer_html_snippet: '<footer>©2026</footer>',
      rect: { x: 0, y: 300, w: 800, h: 40 },
      dpr: 1.25,
      comment: '   ',
    },
  ],
}

describe('serializeAnnotations', () => {
  it('包含 file / 逐条 css_selector / comment 三要素（Leader 改源码的充分信息）', () => {
    const text = serializeAnnotations(MSG)
    expect(text).toContain('demo.html')
    expect(text).toContain('body > div.a:nth-child(2) > p')
    expect(text).toContain('footer:nth-child(3)')
    expect(text).toContain('标题改成蓝色')
    // 空白 comment 归一为可读占位，不留空行歧义
    expect(text).toContain('（无）')
  })

  it('首行即摘要且 ≤60 字符（消息气泡 chip 只截显前 60）', () => {
    const first = serializeAnnotations(MSG).split('\n')[0]
    expect(first).toBe('【界面标注】demo.html · 2 条 · 视口dpr 1.25')
    expect(first.length).toBeLessThanOrEqual(60)
  })

  it('rect 与 outerHTML 摘要按行附上（辅助定位，防爆上下文）', () => {
    const text = serializeAnnotations(MSG)
    expect(text).toContain('rect: 12,34 120x24')
    expect(text).toContain('html: <p>标题</p>')
  })

  it('P3 区域条附加 region 辅助行（元素条无此行）', () => {
    const withRegion: AnnotationMessage = {
      ...MSG,
      annotations: [{ ...MSG.annotations[0], region: { x: 12, y: 24, w: 280, h: 160 } }],
    }
    const text = serializeAnnotations(withRegion)
    expect(text).toContain('region: 12,24 280x160')
    // 既有字段与行序不受 region 影响（html 行仍在 region 行之后）
    expect(text.indexOf('region: 12,24 280x160')).toBeLessThan(text.indexOf('html: <p>标题</p>'))
    expect(text).toContain('rect: 12,34 120x24')
    // 纯元素条不产生 region 行
    expect(serializeAnnotations(MSG)).not.toContain('region:')
  })
})

describe('buildAnnotationRef', () => {
  it('走 quote 引用通道：id 稳定（同内容同 id，addReference 去重）', () => {
    const a = buildAnnotationRef(MSG)
    const b = buildAnnotationRef(MSG)
    expect(a?.type).toBe('quote')
    expect(a?.id).toBe(b?.id)
    expect(a?.id).toMatch(/^q/)
  })

  it('不同标注内容 → 不同 id（不会被去重吞掉）', () => {
    const other = buildAnnotationRef({
      ...MSG,
      annotations: [{ ...MSG.annotations[0], comment: '改成红色' }],
    })
    expect(other?.id).not.toBe(buildAnnotationRef(MSG)?.id)
  })

  it('空 annotations → null（发送侧因此禁用，不产生空引用）', () => {
    expect(buildAnnotationRef({ ...MSG, annotations: [] })).toBeNull()
  })

  it('超长载荷被截断到 ≤2000 字符并带截断标记（不爆 Leader 上下文）', () => {
    const huge: AnnotationMessage = {
      ...MSG,
      annotations: Array.from({ length: 60 }, (_, i) => ({
        css_selector: `body > div.row:nth-child(${i}) > span.tag`,
        outer_html_snippet: '<span class="tag">较长较长的占位片段</span>',
        rect: { x: i, y: i * 10, w: 100, h: 20 },
        dpr: 1,
        comment: `第 ${i} 条批注`,
      })),
    }
    const ref = buildAnnotationRef(huge)
    expect(ref).not.toBeNull()
    expect(ref!.label.length).toBeLessThanOrEqual(2000 + 20)
    expect(ref!.label).toContain('…（已截断）')
  })
})
