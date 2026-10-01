/**
 * autoRaiseForceThreshold 单测
 *
 * 钉死「后端已生效 forceThreshold 低于当前滑块下限时自动抬到下限」：
 * usage 把下限顶高而阈值不动 = 下一轮提前触发（显示撒谎）。同时守住语义
 * 边界：等于 / 高于下限不调 invoke（无谓写盘）；invoke 失败静默 resolve
 * （自动自愈不是用户操作，不得吐 rejection、不得 toast）。
 */
import { describe, expect, it, vi } from 'vitest'
import type { RefineState } from './useExecutionUI'
import { autoRaiseForceThreshold, refineForceMinPct } from './useExecutionUI'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('../core/bridge', () => ({
  invoke: invokeMock,
  listen: vi.fn(() => Promise.resolve(() => {})),
}))

/** 大窗口典型形：usage 70% 把下限顶到 70（50 floor 与 ceil(70/5)*5 取大） */
function makeState(forceThreshold: number, usagePercent = 70): RefineState {
  return {
    usagePercent,
    totalLimit: 1000,
    tier: 'large',
    forceThreshold,
    forceMin: 0.5,
    forceMax: 0.8,
  }
}

describe('autoRaiseForceThreshold', () => {
  it('① 低于下限 → invoke 一次且参数 = 下限/100', async () => {
    invokeMock.mockClear()
    // 0.55 × 100 = 55 < 下限 70（usage 顶高）→ 抬到 0.70
    expect(refineForceMinPct(makeState(0.55, 70))).toBe(70)

    await autoRaiseForceThreshold(makeState(0.55))

    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('set_session_refine_config', { forceThreshold: 0.7 })
  })

  it('② 等于下限 → 不调 invoke', async () => {
    invokeMock.mockClear()
    await autoRaiseForceThreshold(makeState(0.7))
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('③ 高于下限 → 不调 invoke', async () => {
    invokeMock.mockClear()
    await autoRaiseForceThreshold(makeState(0.75))
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('④ invoke reject → resolve 不抛（静默自愈）', async () => {
    invokeMock.mockClear()
    invokeMock.mockRejectedValueOnce(new Error('后端拒绝：配置越界'))

    await expect(autoRaiseForceThreshold(makeState(0.55))).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenCalledTimes(1)
  })
})
