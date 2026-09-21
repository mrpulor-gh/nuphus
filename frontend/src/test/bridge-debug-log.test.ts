import { beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * bridge 高频日志开关契约（`core/debug.ts`）。
 *
 * 背景：`invoke` 每次成功都 `console.log` 结果体 → DevTools 被刷满（实测 ≈67 条/分钟），
 * 既掩盖真实错误，又让每次调用白做一次 `JSON.stringify`（开销落在主线程）。
 *
 * 契约：
 * 1. **默认静音** —— 不打日志，且连结果序列化都不执行（真正零开销）；
 * 2. `localStorage['nuphus:debug'] = '1'` 时恢复结果体日志；
 * 3. 失败路径不受开关约束 —— `invoke` 抛错仍 `console.warn`（那是必须被看见的信号）。
 *
 * 注：开关在 `core/debug.ts` 模块加载时求值，故每个用例都 `resetModules` 后重新 import。
 */
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => ({ ok: true })),
}))

/** 让 `isTauriAvailable()` 返回 true，走 Tauri 分支（应用真实路径） */
function enableTauri() {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
}

async function loadBridge() {
  vi.resetModules()
  return await import('../core/bridge')
}

describe('bridge 高频日志开关', () => {
  beforeEach(() => {
    localStorage.removeItem('nuphus:debug')
    enableTauri()
    vi.restoreAllMocks()
  })

  it('默认静音：invoke 成功不打日志，也不做结果序列化', async () => {
    const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
    const stringifySpy = vi.spyOn(JSON, 'stringify')
    const { invoke } = await loadBridge()

    const result = await invoke('demo_cmd')

    expect(result).toEqual({ ok: true })
    expect(logSpy).not.toHaveBeenCalled()
    // 零开销：关闭时不该对结果体做 stringify（只有参数是结果体时才计数）
    const stringifiedResult = stringifySpy.mock.calls.some(call => {
      const arg = call[0] as { ok?: boolean } | undefined
      return !!arg && typeof arg === 'object' && arg.ok === true
    })
    expect(stringifiedResult).toBe(false)
  })

  it('开关打开：恢复结果体日志', async () => {
    localStorage.setItem('nuphus:debug', '1')
    const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {})
    const { invoke } = await loadBridge()

    await invoke('demo_cmd')

    expect(
      logSpy.mock.calls.some(call =>
        String(call[0]).includes('[Bridge] invoke demo_cmd raw result'),
      ),
    ).toBe(true)
  })

  it('失败路径不受开关约束：invoke 抛错仍 console.warn', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const core = await import('@tauri-apps/api/core')
    ;(core.invoke as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error('boom'))
    const { invoke } = await loadBridge()

    await expect(invoke('demo_cmd')).rejects.toThrow('IPC invoke demo_cmd failed')

    expect(
      warnSpy.mock.calls.some(call =>
        String(call[0]).includes('[Bridge] Tauri invoke demo_cmd failed'),
      ),
    ).toBe(true)
  })
})
