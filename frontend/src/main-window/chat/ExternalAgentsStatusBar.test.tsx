import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import ExternalAgentsStatusBar, { EXT_AGENT_PINNED_EVENT } from './ExternalAgentsStatusBar'
import * as api from '../lib/api'
import type { ExternalAgentStatus } from '../lib/api'

/**
 * 回归钉：外部 Agent 列表栏（.ext-agents-hover-zone 所属胶囊）的移出语义。
 *
 * 旧实现的两处错误（本轮修复）：
 *  1. 「从状态栏移除」写 localStorage 后把头像折叠成数字入口，重启仍在；
 *  2. 点数字入口「恢复显示」时，因 pin 集合启动清零 + idle 不渲染，视觉效果是
 *     「恢复即消失／被删掉」——恢复链路本身不可自证。
 *
 * 新语义（本文件的断言基线）：
 *  - 移出 = 头像从 DOM 真移除，且仅本轮会话内存态（不落盘，重启不残留）；
 *  - 移出必须向用户反馈（调用方注入的 onNotice → HUD 轻提示，**不进 messages 数组**）；
 *  - 该 agent 再次被调用（门铃新上报，updated_at 更新）→ 自动回到列表栏；
 *  - 列表栏只显示「被调用过的（非 idle）」或「本轮配置中心保存过的（pin）」agent。
 */

vi.mock('./PreviewOverlay', () => ({ PreviewOverlay: () => null }))

vi.mock('../lib/api', () => ({
  listAgentStatuses: vi.fn(),
  listAgentDeliverables: vi.fn(),
  listExternalAgents: vi.fn(),
  deleteAgentDeliverable: vi.fn(),
  notifyExtAgentRemoved: vi.fn().mockResolvedValue(undefined),
  extractAgentIcon: vi.fn().mockResolvedValue(null),
}))

const AVATAR_LABEL = '外部 Agent · opencode'
/** 固定在过去的时间戳：默认状态下移出后不应被 poll 误判为「有新活动」 */
const PAST = '2026-01-01T00:00:00+08:00'

function status(over: Partial<ExternalAgentStatus> = {}): ExternalAgentStatus {
  return { agent: 'opencode', state: 'done', task_id: '0916-01', updated_at: PAST, ...over }
}

function mountBar() {
  const onNotice = vi.fn()
  render(<ExternalAgentsStatusBar onNotice={onNotice} />)
  return onNotice
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
  vi.mocked(api.listAgentStatuses).mockResolvedValue([status()])
  vi.mocked(api.listAgentDeliverables).mockResolvedValue([])
  vi.mocked(api.listExternalAgents).mockResolvedValue([])
})

afterEach(() => {
  cleanup()
})

describe('外部 Agent 列表栏 · 移出语义', () => {
  it('移出 = 头像从 DOM 真移除 + 一句提示反馈，且不写 localStorage', async () => {
    const onNotice = mountBar()
    // 「被调用过」（state=done）的 agent 常驻显示
    fireEvent.click(await screen.findByRole('button', { name: AVATAR_LABEL }))

    const removeBtn = await screen.findByRole('button', { name: '从列表栏移除' })
    fireEvent.click(removeBtn)

    // DOM 真移除（不是折叠成数字入口）
    await waitFor(() => expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull())
    expect(screen.queryByText(/已隐藏/)).toBeNull()

    // 用户面反馈：只提示「被移出」这一件事（不夹带实现细节，不解释规则）
    expect(onNotice).toHaveBeenCalledTimes(1)
    expect(onNotice.mock.calls[0][0]).toBe('已从列表栏移出「opencode」')

    // 同时通知 agent（后续需用户显式指定才可调用）
    expect(api.notifyExtAgentRemoved).toHaveBeenCalledWith('opencode')

    // 移出为内存态：不得留下任何持久化痕迹
    expect(localStorage.getItem('nuphus.extAgents.hiddenAgents')).toBeNull()
  })

  it('移出后该 agent 再次被调用（门铃新上报）→ 自动回到列表栏', async () => {
    mountBar()
    fireEvent.click(await screen.findByRole('button', { name: AVATAR_LABEL }))
    fireEvent.click(await screen.findByRole('button', { name: '从列表栏移除' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull())

    // 门铃新上报：updated_at 晚于移出时刻
    vi.mocked(api.listAgentStatuses).mockResolvedValue([
      status({ state: 'in_progress', updated_at: new Date(Date.now() + 3_600_000).toISOString() }),
    ])

    // 轮询周期 3s：等待自动回归
    await waitFor(() => expect(screen.getByRole('button', { name: AVATAR_LABEL })).toBeTruthy(), {
      timeout: 6000,
    })
  }, 10_000)

  it('历史遗留的持久化隐藏/pin 记录被清理，不再产生幽灵条目', async () => {
    localStorage.setItem('nuphus.extAgents.hiddenAgents', JSON.stringify(['opencode']))
    localStorage.setItem('nuphus.extAgents.pinned', JSON.stringify(['opencode']))
    mountBar()

    // 被调用的 agent 照常渲染（旧 hidden 记录不生效）
    expect(await screen.findByRole('button', { name: AVATAR_LABEL })).toBeTruthy()
    await waitFor(() => expect(localStorage.getItem('nuphus.extAgents.hiddenAgents')).toBeNull())
    expect(localStorage.getItem('nuphus.extAgents.pinned')).toBeNull()
  })

  it('空闲且未经配置中心保存的 agent 不占位；配置中心保存（pin）后立即显示', async () => {
    vi.mocked(api.listAgentStatuses).mockResolvedValue([status({ state: 'idle', task_id: '' })])
    mountBar()

    // idle = 本轮未经门铃验证的历史残留 → 不渲染（列表栏只显示被调用过的）
    await waitFor(() => expect(api.listExternalAgents).toHaveBeenCalled())
    expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull()

    // 配置中心保存 → pin → 立即出现在列表栏
    fireEvent(window, new CustomEvent(EXT_AGENT_PINNED_EVENT, { detail: 'opencode' }))
    expect(await screen.findByRole('button', { name: AVATAR_LABEL })).toBeTruthy()
  })
})
