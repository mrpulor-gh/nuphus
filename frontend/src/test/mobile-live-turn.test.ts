/**
 * 执行中「当前轮」恢复（live_turn_restore）契约。
 *
 * 场景：桌面端执行中，手机重开/重连。history 只能给「执行前历史 + 当前轮 user」，
 * 本轮已产生的思考/文本/工具调用由 /agent-status 的 liveTurn 快照补回，
 * 补成 streaming 气泡后，后续 llm_text_delta 直接续写同一条。
 */
import { describe, expect, it } from 'vitest'
import { chatReducer, initialChatState, type ChatMessage, type TraceItem } from '../mobile/store'

const trace: TraceItem[] = [
  { kind: 'tool', callId: 'c1', name: 'Bash', status: 'ok', params: '{}' },
]

describe('live_turn_restore', () => {
  it('空列表时补成一条 streaming 气泡（后续 delta 可续写）', () => {
    const next = chatReducer(initialChatState, {
      type: 'live_turn_restore',
      snapshot: { text: '部分正文', thinking: '已有思考', trace },
    })
    expect(next.messages).toHaveLength(1)
    const m = next.messages[0]
    expect(m.role).toBe('assistant')
    expect(m.streaming).toBe(true)
    expect(m.content).toBe('部分正文')
    expect(m.thinking).toBe('已有思考')
    expect(m.traceItems).toEqual(trace)
  })

  it('已有 streaming 气泡时只补空缺，不覆盖本端已累积内容', () => {
    const base = {
      ...initialChatState,
      messages: [
        {
          id: 'a1',
          role: 'assistant',
          content: '本地已累积',
          streaming: true,
        } as ChatMessage,
      ],
    }
    const next = chatReducer(base, {
      type: 'live_turn_restore',
      snapshot: { text: '后端快照正文', thinking: '补回思考', trace },
    })
    expect(next.messages).toHaveLength(1)
    expect(next.messages[0].content).toBe('本地已累积')
    expect(next.messages[0].thinking).toBe('补回思考')
    expect(next.messages[0].traceItems).toEqual(trace)
  })

  it('空快照不改状态（未执行/旧后端无 liveTurn）', () => {
    const next = chatReducer(initialChatState, {
      type: 'live_turn_restore',
      snapshot: { text: '', thinking: '', trace: [] },
    })
    expect(next).toBe(initialChatState)
  })
})
