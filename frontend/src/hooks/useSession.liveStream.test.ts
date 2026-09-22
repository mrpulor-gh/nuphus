import { describe, it, expect } from 'vitest'
import { liveStreamText } from './useSession'
import type { TimelineEntry } from '../core/types'

const entry = (patch: Partial<TimelineEntry>): TimelineEntry => ({
  id: 'e1',
  kind: 'thinking',
  ...patch,
})

describe('liveStreamText — ThinkingIndicator 流式展示源', () => {
  it('空 timeline 返回空', () => {
    expect(liveStreamText([])).toBe('')
  })

  it('最后一个条目在流式时返回其文本', () => {
    const t = [entry({ id: 'a', kind: 'thinking', text: '分析中' })]
    expect(liveStreamText(t)).toBe('分析中')
  })

  it('正文（text）同样可作为流式源', () => {
    const t = [entry({ id: 'a', kind: 'text', text: '正在输出' })]
    expect(liveStreamText(t)).toBe('正在输出')
  })

  it('最后一个条目被工具调用接管后返回空（不保留内容）', () => {
    const t = [
      entry({ id: 'a', kind: 'thinking', text: '已完成的一段思考' }),
      entry({ id: 'b', kind: 'tool_call', toolName: 'Read' }),
    ]
    expect(liveStreamText(t)).toBe('')
  })

  it('回归：长文本不被截断 —— 高度必须能继续增长以触发换行脉冲', () => {
    // 定长窗口（如 slice(-180)）会让文本长度封顶、行高不再增长，
    // 导致 ResizeObserver 测不到换行、脉冲永不触发。
    const long = 'x'.repeat(5000)
    const t = [entry({ id: 'a', kind: 'thinking', text: long })]
    expect(liveStreamText(t)).toHaveLength(5000)
    expect(liveStreamText(t)).toBe(long)
  })

  it('文本单调增长（逐字追加语义）', () => {
    const a = liveStreamText([entry({ id: 'a', text: 'abc' })])
    const b = liveStreamText([entry({ id: 'a', text: 'abcdef' })])
    expect(b.startsWith(a)).toBe(true)
    expect(b.length).toBeGreaterThan(a.length)
  })

  it('无文本字段时返回空', () => {
    expect(liveStreamText([entry({ id: 'a', kind: 'thinking' })])).toBe('')
    expect(liveStreamText([entry({ id: 'a', kind: 'thinking', text: '' })])).toBe('')
  })
})
