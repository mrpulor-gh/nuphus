import { beforeEach, describe, expect, it, vi } from 'vitest'

// 只关心 invoke 的命令名与参数；bridge 整体打桩，避免真实 IPC
vi.mock('../../core/bridge', () => ({ invoke: vi.fn(async () => null) }))

import { invoke } from '../../core/bridge'
import { browserOpenUrl, openExternal } from './api'

const mockedInvoke = vi.mocked(invoke)

/**
 * 外链落点契约（2026-10-09 大王定调：统一进 Agent 浏览器 / CDP）。
 *
 * 钉住三件事：
 * 1. 用户可见外链走 `browser_open_url`（新标签打开，不打断 Agent 正在自动化的页面）；
 * 2. 返回的 `launched` / `url` 原样透出——前端据 `launched` 决定是否提示"正在启动"；
 * 3. `openExternal` 仍在，但**只**服务模型厂商 OAuth（那条链路依赖外部浏览器的回调）。
 */
describe('外链落点', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockedInvoke.mockResolvedValue({
      launched: true,
      url: 'https://example.com/',
    } as never)
  })

  it('browserOpenUrl → browser_open_url + { url }', async () => {
    await browserOpenUrl('https://example.com/')
    expect(mockedInvoke).toHaveBeenCalledWith('browser_open_url', {
      url: 'https://example.com/',
    })
  })

  it('launched / url 原样透出（不重命名、不吞掉）', async () => {
    await expect(browserOpenUrl('https://example.com/')).resolves.toEqual({
      launched: true,
      url: 'https://example.com/',
    })
  })

  it('openExternal 保留给 OAuth：仍是 open_external，不进 Agent 浏览器', async () => {
    await openExternal('https://sso.example.com/authorize')
    expect(mockedInvoke).toHaveBeenCalledWith('open_external', {
      url: 'https://sso.example.com/authorize',
    })
  })
})
