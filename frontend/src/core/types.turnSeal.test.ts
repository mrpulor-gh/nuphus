/**
 * sealTurnDuration —— 「终止即封口」的钉子测试。
 *
 * 背景（2026-10-26 大王报障）：主项目强行中断 / API error 之后，
 * `.turn-meta-bar--actions .turn-meta-chip` 仍在计时。根因是气泡计时只认
 * `msg.meta.durationMs`，而唯一写入它的通道是 `execution_completed`；中断路径
 * （execution_error）没有该事件，于是 `TurnMetaBar` 的
 * `isLive = durationMs == null && startedAtMs > 0` 恒真 → 200ms 自 tick 永不停。
 *
 * 修复口径：与输入框（其 tick 由 isProcessing 门控）同源 —— 执行状态终止即停表；
 * 封口值走同一个出口 `resolveTurnDuration`，不另写算法。
 */
import { describe, it, expect } from 'vitest'
import { resolveTurnDuration, sealTurnDuration } from './types'

describe('sealTurnDuration（终止即封口）', () => {
  it('进行中（只有 startedAtMs）→ 定格为此刻推算值', () => {
    const meta = { startedAtMs: Date.now() - 3_000, toolCalls: 3 }
    const sealed = sealTurnDuration(meta)
    expect(sealed.durationMs).toBeGreaterThan(0)
    // 封口值 = 同一出口 resolveTurnDuration 的取值（单一算法）
    expect(resolveTurnDuration(sealed)).toBe(sealed.durationMs)
    expect(sealed.toolCalls).toBe(3)
  })

  it('已结束（durationMs 有值）→ 原样返回，后端权威值不被覆盖', () => {
    const meta = { startedAtMs: 1_000, durationMs: 4_321 }
    expect(sealTurnDuration(meta)).toEqual(meta)
  })

  it('无线索 → 不伪造（无 startedAtMs / null 均原样）', () => {
    expect(sealTurnDuration({ toolCalls: 2 })).toEqual({ toolCalls: 2 })
    expect(sealTurnDuration(null)).toEqual({})
    expect(sealTurnDuration(undefined)).toEqual({})
  })

  it('回归：封口后耗时不再随时钟增长（中断后计时器永动的死因）', () => {
    const sealed = sealTurnDuration({ startedAtMs: Date.now() - 5_000 })
    const atSeal = resolveTurnDuration(sealed, Date.now())
    const minuteLater = resolveTurnDuration(sealed, Date.now() + 60_000)
    expect(minuteLater).toBe(atSeal)
  })

  it('回归：未封口时会随时钟增长（证明该测试确实钉住行为）', () => {
    const live = { startedAtMs: Date.now() - 1_000 }
    const a = resolveTurnDuration(live, Date.now())
    const b = resolveTurnDuration(live, Date.now() + 60_000)
    expect(b - a).toBeGreaterThanOrEqual(59_000)
  })
})
