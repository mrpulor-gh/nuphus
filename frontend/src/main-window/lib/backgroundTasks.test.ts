import { describe, expect, it, vi } from 'vitest'
import {
  formatElapsed,
  notifyBackgroundTasksChanged,
  reportHasKillFailure,
  subscribeBackgroundTasksChanged,
} from './backgroundTasks'

describe('formatElapsed — 用后端 elapsed_ms 本地格式化', () => {
  it('不足一分钟只报秒', () => {
    expect(formatElapsed(0)).toBe('0s')
    expect(formatElapsed(45_000)).toBe('45s')
    expect(formatElapsed(59_999)).toBe('59s')
  })

  it('分钟级补零到两位（12m31s）', () => {
    expect(formatElapsed(751_000)).toBe('12m31s')
    expect(formatElapsed(60_000)).toBe('1m00s')
    expect(formatElapsed(599_000)).toBe('9m59s')
  })

  it('小时级只报时与分（秒位无信息量，省掉）', () => {
    expect(formatElapsed(3_600_000)).toBe('1h00m')
    expect(formatElapsed(7_500_000)).toBe('2h05m')
  })

  it('脏值一律退化为 0s，绝不把 NaN 漏到界面上', () => {
    expect(formatElapsed(null)).toBe('0s')
    expect(formatElapsed(undefined)).toBe('0s')
    expect(formatElapsed(-1)).toBe('0s')
    expect(formatElapsed(Number.NaN)).toBe('0s')
    expect(formatElapsed(Number.POSITIVE_INFINITY)).toBe('0s')
  })
})

describe('reportHasKillFailure — 只认「有进程没杀掉」这一个失败面', () => {
  it('含「未能终止」时为真（失败面必须原样带出）', () => {
    expect(
      reportHasKillFailure(
        'Task interrupted: 无活动工作流；已终止 0 个前台进程（1 个自行退出），另有 2 个前台进程未能终止（已如实记录）',
      ),
    ).toBe(true)
  })

  it('正常汇报为假', () => {
    expect(
      reportHasKillFailure('Task interrupted: 无活动工作流；已终止 1 个前台进程（0 个自行退出）'),
    ).toBe(false)
  })

  it('null / 空串为假（没有事实就没有失败面）', () => {
    expect(reportHasKillFailure(null)).toBe(false)
    expect(reportHasKillFailure(undefined)).toBe(false)
    expect(reportHasKillFailure('')).toBe(false)
  })
})

describe('账本变化信号总线', () => {
  it('订阅者能被通知，且退订后不再收到', () => {
    const listener = vi.fn()
    const unsubscribe = subscribeBackgroundTasksChanged(listener)
    notifyBackgroundTasksChanged()
    expect(listener).toHaveBeenCalledTimes(1)

    unsubscribe()
    notifyBackgroundTasksChanged()
    expect(listener).toHaveBeenCalledTimes(1)
  })
})
