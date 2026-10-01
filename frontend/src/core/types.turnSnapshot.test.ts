/**
 * applyTurnSnapshot —— 轮询快照合入 turnMeta 的钉子测试
 *
 * 钉死两条规则（历史缺陷：步数恒久显示为 1）：
 *   ① 步数**直接落定**快照权威值，不得被 `prev.toolCalls || 快照` 之类合并
 *      冻住——主轮（react_loop）不发 execution_progress，轮询是普通会话唯一的
 *      实时步数通道，冻住后无论走多少步都停在首个非零读数（实测恒为 1）。
 *   ② 起点「只在缺省时补」：事件通道（execution_started）写过的起点不被快照
 *      覆盖，避免混入别的轮次 / 别的窗口的起点。
 */
import { describe, expect, it } from 'vitest'
import { applyTurnSnapshot, resolveTurnCalls, resolveTurnDuration } from './types'

describe('applyTurnSnapshot', () => {
  it('① 步数取快照权威值：前值更小也不得短路冻住', () => {
    // 复现缺陷路径：轮询 1.5s 一帧，首帧只看到 1 次调用
    let meta = applyTurnSnapshot({ startedAtMs: 1000 }, { startedAtMs: 1000, toolCalls: 0 })
    expect(resolveTurnCalls(meta)).toBe(0)

    meta = applyTurnSnapshot(meta, { startedAtMs: 1000, toolCalls: 1 })
    expect(resolveTurnCalls(meta)).toBe(1)

    // 后续帧步数继续增长（旧实现在此被 `prev.toolCalls ||` 短路，恒显 1）
    meta = applyTurnSnapshot(meta, { startedAtMs: 1000, toolCalls: 2 })
    expect(resolveTurnCalls(meta)).toBe(2)
    meta = applyTurnSnapshot(meta, { startedAtMs: 1000, toolCalls: 7 })
    expect(resolveTurnCalls(meta)).toBe(7)

    // 零步（本轮无工具调用）保持 0，不拿别处的值充数
    meta = applyTurnSnapshot(meta, { startedAtMs: 1000, toolCalls: 0 })
    expect(resolveTurnCalls(meta)).toBe(0)
  })

  it('② 起点缺省时补快照，已有起点不被覆盖', () => {
    // 刷新后 turnMeta 为空 → 快照补回后端权威起点（耗时据此实时推算）
    const restored = applyTurnSnapshot(null, { startedAtMs: 1_700_000_000_000, toolCalls: 3 })
    expect(restored.startedAtMs).toBe(1_700_000_000_000)
    expect(resolveTurnCalls(restored)).toBe(3)
    expect(resolveTurnDuration(restored, 1_700_000_000_500)).toBe(500)

    // 事件通道已写过起点 → 快照不覆盖（防止把别的轮次起点混进来）
    const kept = applyTurnSnapshot(
      { startedAtMs: 1000, toolCalls: 1 },
      { startedAtMs: 9999, toolCalls: 4 },
    )
    expect(kept.startedAtMs).toBe(1000)
    expect(resolveTurnCalls(kept)).toBe(4)
  })

  it('③ 其它字段不被快照冲掉（token / 已落定耗时原样保留）', () => {
    const prev = {
      startedAtMs: 1000,
      durationMs: 4200,
      inputTokens: 130801,
      outputTokens: 199,
      cacheHitTokens: 51200,
      toolCalls: 1,
    }
    const next = applyTurnSnapshot(prev, { startedAtMs: 1000, toolCalls: 5 })
    expect(next).toEqual({ ...prev, toolCalls: 5 })
  })
})
