import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SEND_TOTAL_TIMEOUT, useSendLock } from './useSendLock'

/** 遮罩的「必定解除」四条路径中，超时与卸载在这里断言；
 *  成功 / 失败都走 cancel()，与第二条用例同一出口。 */
describe('useSendLock', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('上锁：遮罩可见且占用标记同步置位', () => {
    const { result } = renderHook(() => useSendLock())
    expect(result.current.sending).toBe(false)
    expect(result.current.busyRef.current).toBe(false)

    act(() => result.current.begin(() => {}))

    expect(result.current.sending).toBe(true)
    expect(result.current.busyRef.current).toBe(true)
  })

  it('成功 / 失败路径：cancel 立即解锁，且兜底计时已收掉不再回调', () => {
    const onTimeout = vi.fn()
    const { result } = renderHook(() => useSendLock())
    act(() => result.current.begin(onTimeout))

    act(() => result.current.cancel())

    expect(result.current.sending).toBe(false)
    expect(result.current.busyRef.current).toBe(false)
    act(() => void vi.advanceTimersByTime(SEND_TOTAL_TIMEOUT * 2))
    expect(onTimeout).not.toHaveBeenCalled()
  })

  it('超时路径：兜底到点必定解锁并回调一次（界面不会永久锁死）', () => {
    const onTimeout = vi.fn()
    const { result } = renderHook(() => useSendLock())
    act(() => result.current.begin(onTimeout))

    act(() => void vi.advanceTimersByTime(SEND_TOTAL_TIMEOUT))

    expect(onTimeout).toHaveBeenCalledTimes(1)
    expect(result.current.sending).toBe(false)
    expect(result.current.busyRef.current).toBe(false)
    // 解锁已发生：再推进时间不会有第二次回调
    act(() => void vi.advanceTimersByTime(SEND_TOTAL_TIMEOUT))
    expect(onTimeout).toHaveBeenCalledTimes(1)
  })

  it('重复 begin 不叠加计时器：只保留最后一次的兜底', () => {
    const first = vi.fn()
    const second = vi.fn()
    const { result } = renderHook(() => useSendLock())

    act(() => result.current.begin(first))
    act(() => result.current.begin(second))
    act(() => void vi.advanceTimersByTime(SEND_TOTAL_TIMEOUT))

    expect(first).not.toHaveBeenCalled()
    expect(second).toHaveBeenCalledTimes(1)
  })

  it('卸载路径：计时器随卸载收掉，不再有回调打向已卸载的组件', () => {
    const onTimeout = vi.fn()
    const { result, unmount } = renderHook(() => useSendLock())
    act(() => result.current.begin(onTimeout))

    unmount()
    act(() => void vi.advanceTimersByTime(SEND_TOTAL_TIMEOUT * 2))

    expect(onTimeout).not.toHaveBeenCalled()
  })

  it('cancel 幂等：未上锁或重复解锁都不抛错', () => {
    const { result } = renderHook(() => useSendLock())

    act(() => result.current.cancel())
    act(() => result.current.begin(() => {}))
    act(() => result.current.cancel())
    act(() => result.current.cancel())

    expect(result.current.sending).toBe(false)
    expect(result.current.busyRef.current).toBe(false)
  })
})
