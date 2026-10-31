/**
 * lastTurnExecution —— 历史恢复时回填执行快照的取值契约。
 *
 * 只搬运已持久化的 meta / traceItems：步数与耗时来自最后一轮 assistant，
 * 不推算 ctx 令牌（ctx 由模型真实请求的 input token 驱动，重启后保持 `--`）。
 */
import { describe, expect, it } from 'vitest'
import { lastTurnExecution } from './useInit'
import type { HistoryMessage } from '../main-window/lib/api'

function msg(over: Partial<HistoryMessage>): HistoryMessage {
  return { role: 'assistant', content: '', images: [], audio: [], ...over }
}

describe('lastTurnExecution', () => {
  it('空历史 / 无 assistant → null（保持未知态，不伪造 0）', () => {
    expect(lastTurnExecution(null)).toBeNull()
    expect(lastTurnExecution([])).toBeNull()
    expect(lastTurnExecution([msg({ role: 'user', content: 'hi' })])).toBeNull()
  })

  it('取最后一条 assistant 的 meta 与 traceItems', () => {
    const history = [
      msg({ role: 'user', content: 'q' }),
      msg({ meta: { durationMs: 1200, toolCalls: 2 }, content: 'old' }),
      msg({
        content: 'done',
        meta: { durationMs: 3400, toolCalls: 4 },
        traceItems: [
          { kind: 'tool', call_id: 'c1', name: 'Read', status: 'ok' },
          { kind: 'text', text: '完成' },
        ],
      }),
    ]

    const restored = lastTurnExecution(history)
    expect(restored?.meta).toEqual({ durationMs: 3400, toolCalls: 4 })
    expect(restored?.timeline).toEqual([
      expect.objectContaining({ id: 'c1', kind: 'tool_call', toolName: 'Read', status: 'success' }),
      expect.objectContaining({ kind: 'text', text: '完成' }),
    ])
  })

  it('只有轨迹没有 meta 时也恢复轨迹（纯工具轮的轨迹随最终回复折叠保留）', () => {
    const restored = lastTurnExecution([
      // 真实历史形态：纯工具轮（无正文）在前，最终回复在后，折叠时合并为一条。
      msg({
        content: '',
        traceItems: [{ kind: 'tool', call_id: 'c9', name: 'Bash', status: 'fail' }],
      }),
      msg({ content: '已处理' }),
    ])
    expect(restored?.meta).toBeNull()
    expect(restored?.timeline).toHaveLength(1)
    expect(restored?.timeline[0]).toMatchObject({ kind: 'tool_call', status: 'error' })
  })
})
