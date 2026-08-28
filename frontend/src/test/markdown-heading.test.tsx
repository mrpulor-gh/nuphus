import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import MarkdownContent from '../main-window/chat/MarkdownContent'

/**
 * 回归钉：紧凑 Markdown（标题与正文之间无空行）曾把正文整段静默丢弃——
 * BlockRenderer 标题分支只渲染 lines[0]，标题块内随行的正文被吞，表现为
 * 「回复块有标题但没有内容」。GLM 的紧凑输出风格高频触发，DeepSeek 的
 * 标准风格（标题后带空行）几乎不触发，故两端现象频率不同。
 */
describe('MarkdownContent 紧凑标题回归钉', () => {
  it('标题与正文之间无空行时，正文必须渲染', () => {
    const html = renderToString(<MarkdownContent content={'## 结论\n差距巨大，核心在调试断层。'} />)
    expect(html).toContain('结论')
    expect(html).toContain('差距巨大，核心在调试断层。')
  })

  it('多段紧凑标题：每个标题���的正文都必须渲染', () => {
    const md = ['## 第一节', '第一节的内容。', '## 第二节', '第二节的内容。'].join('\n')
    const html = renderToString(<MarkdownContent content={md} />)
    expect(html).toContain('第一节的内容。')
    expect(html).toContain('第二节的内容。')
  })

  it('标准风格（标题后有空行）不受影响', () => {
    const html = renderToString(<MarkdownContent content={'## 标题\n\n正文内容。'} />)
    expect(html).toContain('标题')
    expect(html).toContain('正文内容。')
  })

  it('纯标题（无正文）正常渲染', () => {
    const html = renderToString(<MarkdownContent content={'## 只有标题'} />)
    expect(html).toContain('只有标题')
  })
})
