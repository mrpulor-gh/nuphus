/**
 * 轻反馈「无旁路」契约 —— 前端不许再直发 `invoke('hud_update')`。
 *
 * 背景：收编前 17 处前端直调点把提示直接甩给 HUD 独立窗口，绕过了 ui/islandChannel
 * 的前台/后台分流（应用就在眼前也弹屏幕角落的绿/黄小窗）。现在全部经岛通道，
 * 唯一允许直发 HUD 的地方是 islandChannel.ts 内部的后台分支（后端另有两处启动期
 * 直调，刻意保留，见 src-tauri 的 toolbar.rs / startup_guard.rs 注释）。
 *
 * 这里做两件事：① 全量源码扫描（防止新代码重新长出旁路，含未来新增文件）；
 * ② 验证 `hudUpdate` 封装在前台不再触达 hud_update，且 HUD 相位按映射表落到岛语义
 *   （done → success），后台仍原样交给 HUD。
 */
import { act } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getAppIslandSnapshot, setAppFocused } from '../ui/islandChannel'

const bridge = vi.hoisted(() => ({ invoke: vi.fn(async () => undefined) }))

vi.mock('../core/bridge', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
  invoke: bridge.invoke,
}))

// 真实封装（本文件刻意不 mock ../main-window/lib/api：被验证的正是它）
import { hudUpdate } from '../main-window/lib/api'

/** 直发 HUD 的唯一合法位置：岛的「不在前台」分支 */
const ALLOW_DIRECT_HUD = new Set(['src/ui/islandChannel.ts'])
const DIRECT_HUD_CALL = /invoke\(\s*['"]hud_update['"]/

/** 源码原文（不经转译）：扫描用；?raw 让 Vite 直接返回文件内容 */
const SOURCE_FILES = import.meta.glob('/src/**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

/** 排空 island 队列（一条提示要吃掉「停留 + 退场」两段时间） */
function drainIsland() {
  for (let i = 0; i < 20 && getAppIslandSnapshot().toast !== null; i++) {
    act(() => vi.advanceTimersByTime(10_000))
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  bridge.invoke.mockClear()
})

afterEach(() => {
  drainIsland()
  vi.useRealTimers()
})

describe('轻反馈无旁路', () => {
  it('前端源码里只有 islandChannel 的后台分支直发 hud_update', () => {
    const scanned = Object.entries(SOURCE_FILES).filter(([path]) => !path.includes('.test.'))
    expect(scanned.length).toBeGreaterThan(50) // 扫描面必须真的覆盖到源码（防 glob 失效假绿）

    const direct = scanned
      .filter(([, code]) => DIRECT_HUD_CALL.test(code))
      .map(([path]) => path.replace(/^\//, ''))
      .sort()

    // 断言「命中的就是唯一允许的那一处」：既验证无旁路，也验证命中检测本身有效
    // （只断言 offenders 为空时，正则写错也会假绿）
    expect(direct).toEqual([...ALLOW_DIRECT_HUD])
  })

  it('前台：hudUpdate 不再触达 hud_update，且 done 相位落到岛的 success', () => {
    act(() => {
      setAppFocused(true)
      hudUpdate('执行完成', 'done')
    })

    expect(bridge.invoke).not.toHaveBeenCalledWith('hud_update', expect.anything())
    expect(getAppIslandSnapshot().toast).toMatchObject({ message: '执行完成', type: 'success' })
  })

  it('未知相位 → island info（不猜语义，也不静默丢弃）', () => {
    act(() => {
      setAppFocused(true)
      hudUpdate('未知相位提示', 'whatever')
    })

    expect(getAppIslandSnapshot().toast).toMatchObject({ message: '未知相位提示', type: 'info' })
  })

  it('后台/最小化：相位原样交给 HUD（done 保持 HUD 自己的 15s 语义）', () => {
    act(() => {
      setAppFocused(false)
      hudUpdate('执行完成', 'done')
    })

    expect(bridge.invoke).toHaveBeenCalledWith('hud_update', {
      text: '执行完成',
      phase: 'done',
    })
    expect(getAppIslandSnapshot().toast).toBeNull()
  })
})
