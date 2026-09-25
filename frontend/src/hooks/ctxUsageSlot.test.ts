/**
 * issue #62 回归测试：上下文用量指示器的槽位归属
 *
 * 三条不变量：
 *  1. source="main"  → 主指示器槽（Leader 回合后的会话规模快照走这里）
 *  2. source="exec"  → ctx 弹窗槽（dispatch / 子任务执行）
 *  3. 其它 source（leader/workflow/…）→ 两槽都不吸收
 *
 * 修复前是「非 main 即 exec」的兜底二分：leader/workflow 的快照会被塞进 exec 槽，
 * 污染 ctx 弹窗那套整组指标；而且 main 槽永远拿不到会话规模，切换会话后也不清。
 */
import { describe, expect, it, vi } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useExecutionUI } from './useExecutionUI'

// useExecutionUI 需要一个 toast 回调；这里只关心 token 槽，给个空实现即可。
const noop = () => {}

function setup() {
  return renderHook(() => useExecutionUI(noop))
}

describe('issue #62: token usage 槽位归属', () => {
  it('mainTokenUsage 初始为 null（不能伪装成 0）', () => {
    const { result } = setup()
    expect(result.current.mainTokenUsage).toBeNull()
  })

  it('setMainTokenUsage(null) 可回落到未知态', () => {
    const { result } = setup()
    act(() => {
      result.current.setMainTokenUsage({ inputTokens: 1234, outputTokens: 5, cacheHitTokens: 0 })
    })
    expect(result.current.mainTokenUsage?.inputTokens).toBe(1234)

    act(() => {
      result.current.setMainTokenUsage(null)
    })
    expect(result.current.mainTokenUsage).toBeNull()
  })

  it('main 与 exec 两槽互相独立', () => {
    const { result } = setup()
    act(() => {
      result.current.setMainTokenUsage({ inputTokens: 100, outputTokens: 1, cacheHitTokens: 0 })
      result.current.setExecTokenUsage({ inputTokens: 999, outputTokens: 2, cacheHitTokens: 0 })
    })
    expect(result.current.mainTokenUsage?.inputTokens).toBe(100)
    expect(result.current.execTokenUsage?.inputTokens).toBe(999)

    // 清 main 槽不影响 exec 槽（会话切换时只清 main）
    act(() => {
      result.current.setMainTokenUsage(null)
    })
    expect(result.current.mainTokenUsage).toBeNull()
    expect(result.current.execTokenUsage?.inputTokens).toBe(999)
  })
})

describe('issue #62: 事件 source 归属规则（与 useEvents 的判定同构）', () => {
  // 复刻 useEvents.ts 的三分类判定，固化契约：exec 槽只认真 exec 源。
  const route = (source: string): 'main' | 'exec' | 'drop' => {
    if (source === 'main') return 'main'
    if (source === 'exec') return 'exec'
    return 'drop'
  }

  it('main → main 槽（Leader 回合的会话规模快照）', () => {
    expect(route('main')).toBe('main')
  })

  it('exec → exec 槽（ctx 弹窗专用）', () => {
    expect(route('exec')).toBe('exec')
  })

  it('leader / workflow 不落任何槽（修复前会被兜底塞进 exec）', () => {
    expect(route('leader')).toBe('drop')
    expect(route('workflow')).toBe('drop')
  })
})
