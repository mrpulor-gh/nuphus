import { afterEach, describe, expect, it, vi } from 'vitest'
import { scheduleIdle } from './idle'

describe('scheduleIdle', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('支持 requestIdleCallback 时交由空闲调度，绝不同步执行', () => {
    const task = vi.fn()
    let idleCallback: IdleRequestCallback | undefined
    let idleOptions: IdleRequestOptions | undefined
    vi.stubGlobal('requestIdleCallback', (cb: IdleRequestCallback, opts?: IdleRequestOptions) => {
      idleCallback = cb
      idleOptions = opts
      return 1
    })

    scheduleIdle(task)

    // 关键约束：预取不得落在启动同步路径上
    expect(task).not.toHaveBeenCalled()
    expect(idleCallback).toBeTypeOf('function')
    // 浏览器长期繁忙时也要有兜底执行时机，且不早于 2000ms
    expect(idleOptions?.timeout).toBeGreaterThanOrEqual(2000)

    idleCallback?.({ didTimeout: false, timeRemaining: () => 50 })
    expect(task).toHaveBeenCalledTimes(1)
  })

  it('不支持 requestIdleCallback 时退化为 setTimeout，延迟不低于 2000ms', () => {
    const task = vi.fn()
    vi.stubGlobal('requestIdleCallback', undefined)
    vi.useFakeTimers()

    scheduleIdle(task)

    vi.advanceTimersByTime(1999)
    expect(task).not.toHaveBeenCalled()

    vi.advanceTimersByTime(1)
    expect(task).toHaveBeenCalledTimes(1)
  })
})
