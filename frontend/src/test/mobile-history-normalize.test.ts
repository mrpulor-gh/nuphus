import { describe, expect, it } from 'vitest'
import { normalizeHistory } from '../mobile/api'

/**
 * 历史消息字段归一化回归。
 *
 * 后端 `state::HistoryMessage` 未加 rename_all，执行过程以 `trace_items` 下发；
 * 前端类型用 `traceItems`。归一化缺失时，手机重开/重连只能看到最终回复，
 * 工具与思考记录全部丢失（大王 2026-11-16 反馈）。
 */
describe('normalizeHistory', () => {
  it('把后端 snake_case 的 trace_items 映射为 traceItems', () => {
    const out = normalizeHistory([
      {
        role: 'assistant',
        content: '最终回复',
        message_id: 'm1',
        trace_items: [
          { kind: 'tool', callId: 'c1', name: 'Bash', status: 'ok' },
          { kind: 'thinking', text: '思考中' },
        ],
      },
    ])
    expect(out).toHaveLength(1)
    expect(out[0].traceItems).toHaveLength(2)
    const first = out[0].traceItems?.[0]
    expect(first?.kind).toBe('tool')
    if (first?.kind === 'tool') expect(first.name).toBe('Bash')
    expect(out[0].traceItems?.[1].kind).toBe('thinking')
  })

  it('已经 camelCase 的 traceItems 原样保留', () => {
    const out = normalizeHistory([
      { role: 'assistant', content: 'x', traceItems: [{ kind: 'text', text: 'hi' }] },
    ])
    expect(out[0].traceItems).toEqual([{ kind: 'text', text: 'hi' }])
  })

  it('缺失 trace 时不伪造空数组（保持 undefined，交给渲染层判定）', () => {
    const out = normalizeHistory([{ role: 'user', content: '你好' }])
    expect(out[0].traceItems).toBeUndefined()
  })

  it('非数组输入返回空列表（后端异常时不炸调用方）', () => {
    expect(normalizeHistory(null)).toEqual([])
    expect(normalizeHistory({ error: 'x' })).toEqual([])
  })
})
