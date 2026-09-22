/**
 * 轻反馈**通道分流**契约 —— 前台走应用内 island，非前台/最小化仍走 HUD。
 *
 * 入口唯一：`useInit.showToast`（全站调用点都走它，见 useSession / useAgentControl /
 * useExecutionUI / App）。所以分流只需在这里验证一次，调用点无需感知通道差异。
 *
 * 这里用真实入口（showToast）驱动，断言两条出路：island 队列 vs hud_update 调用。
 */
import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useInit } from '../hooks/useInit'
import { getAppIslandSnapshot, setAppFocused } from '../ui/islandChannel'

const bridge = vi.hoisted(() => ({ invoke: vi.fn(async () => undefined) }))

vi.mock('../core/bridge', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
  invoke: bridge.invoke,
}))

// 挂载期会拉配置 / 历史 / 上下文限额：整包 stub，隔离初始化副作用
vi.mock('../main-window/lib/api', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>()
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(actual)) {
    out[key] = typeof value === 'function' ? vi.fn(async () => undefined) : value
  }
  return out
})

// 依赖必须是**稳定**引用：每次渲染新建 vi.fn() 会让 useInit 的初始化 effect 反复重跑
// （setStartupStats 每次写入新对象 → 状态变化 → 重渲染 → 新依赖 → 死循环 + 内存暴涨）
const initDeps = {
  setMessages: vi.fn(),
  setModelName: vi.fn(),
  setSessionId: vi.fn(),
  messagesRestoredRef: { current: false },
  setMode: vi.fn(),
}

function mountInit() {
  return renderHook(() => useInit(initDeps))
}

/** 排空 island 队列（一条提示要吃掉「停留 + 退场」两段时间） */
function drainIsland() {
  for (let i = 0; i < 20 && getAppIslandSnapshot().toast !== null; i++) {
    act(() => vi.advanceTimersByTime(10_000))
  }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  drainIsland()
  vi.useRealTimers()
})

describe('轻反馈通道分流', () => {
  it('应用窗口在前台 → 进 island 队列，不调用 hud_update', async () => {
    const { result } = mountInit()
    await act(async () => {}) // 让挂载期的初始化 invoke 落定
    bridge.invoke.mockClear()

    act(() => {
      setAppFocused(true)
      result.current.showToast('已保存', 'success')
    })

    expect(bridge.invoke).not.toHaveBeenCalledWith('hud_update', expect.anything())
    expect(getAppIslandSnapshot().toast).toMatchObject({
      message: '已保存',
      type: 'success',
    })
  })

  it('应用不在前台 / 最小化 → 仍走 HUD 独立窗口，不进 island', async () => {
    const { result } = mountInit()
    await act(async () => {})
    bridge.invoke.mockClear()

    act(() => {
      setAppFocused(false)
      result.current.showToast('已中断', 'warning')
    })

    expect(bridge.invoke).toHaveBeenCalledWith('hud_update', {
      text: '已中断',
      phase: 'warning',
    })
    expect(getAppIslandSnapshot().toast).toBeNull()
  })

  it('相位直通 HUD（error → phase error），与改造前一致', async () => {
    const { result } = mountInit()
    await act(async () => {})
    bridge.invoke.mockClear()

    act(() => {
      setAppFocused(false)
      result.current.showToast('执行失败', 'error')
    })

    expect(bridge.invoke).toHaveBeenCalledWith('hud_update', {
      text: '执行失败',
      phase: 'error',
    })
  })
})
