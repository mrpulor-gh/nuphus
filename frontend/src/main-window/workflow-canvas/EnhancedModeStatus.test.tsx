import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EnhancedModeStatus } from './EnhancedModeStatus'
import { getWorkflowEnhancedMode, setWorkflowEnhancedMode } from '../lib/api'
import {
  WORKFLOW_ENHANCED_MODE_CHANGED_EVENT,
  WORKFLOW_ENHANCED_MODE_REFRESH_EVENT,
  publishWorkflowEnhancedMode,
  requestWorkflowEnhancedModeRefresh,
} from './enhancedModeEvents'

vi.mock('../lib/api', () => ({
  getLanguage: async () => '',
  getWorkflowEnhancedMode: vi.fn(),
  setWorkflowEnhancedMode: vi.fn(),
}))

describe('EnhancedModeStatus —— 只读状态行（无开关 / 点击跳增强判断模型设置）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.removeItem('nuphus_language')
  })

  const item = () => screen.getByRole('menuitem', { name: /增强模式/ })

  it('挂载即读后端权威状态并把状态文案播报在菜单行里', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    render(<EnhancedModeStatus />)
    expect(await screen.findByRole('menuitem', { name: '增强模式，可用' })).toBeInTheDocument()
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
  })

  it('组件内不提供任何开关：没有 aria-pressed，且点击绝不写后端', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: true,
      status: 'disabled',
    })
    render(<EnhancedModeStatus />)
    const node = await screen.findByRole('menuitem', { name: /增强模式/ })
    expect(node).not.toHaveAttribute('aria-pressed')
    expect(screen.queryByRole('switch')).toBeNull()
    expect(node.tagName).toBe('BUTTON')

    fireEvent.click(node)
    // 「取消开关控制」是产品定稿：状态行点击只导航，绝不改设置
    expect(setWorkflowEnhancedMode).not.toHaveBeenCalled()
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
  })

  it('点击派发 nuphus-nav-models 且 detail.view=jev（复用既有跳转通道）', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({ enabled: false, configured: false })
    const events: CustomEvent<{ view?: string }>[] = []
    const onNav = (e: Event) => events.push(e as CustomEvent<{ view?: string }>)
    window.addEventListener('nuphus-nav-models', onNav)
    try {
      render(<EnhancedModeStatus />)
      fireEvent.click(await screen.findByRole('menuitem', { name: /增强模式/ }))
      expect(events).toHaveLength(1)
      expect(events[0].detail).toEqual({ view: 'jev' })
    } finally {
      window.removeEventListener('nuphus-nav-models', onNav)
    }
  })

  it('a11y：aria-label 含状态文案，title 说明点击去向', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: true,
      status: 'preview',
    })
    render(<EnhancedModeStatus />)
    const node = await screen.findByRole('menuitem', { name: '增强模式，预览' })
    expect(node.getAttribute('title')).toBe('点击前往增强判断模型设置')
  })

  it('enabled 时挂 is-on 状态类（状态语义可见，非恒真）', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    const { unmount } = render(<EnhancedModeStatus />)
    expect((await screen.findByRole('menuitem', { name: /增强模式/ })).className).toContain('is-on')
    unmount()

    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: true,
      status: 'disabled',
    })
    render(<EnhancedModeStatus />)
    expect((await screen.findByRole('menuitem', { name: /增强模式/ })).className).not.toContain(
      'is-on',
    )
  })

  it('CHANGED 事件直接落地推送状态，不再回读后端', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: true,
      status: 'disabled',
    })
    render(<EnhancedModeStatus />)
    await screen.findByRole('menuitem', { name: /增强模式，已关闭/ })

    act(() => publishWorkflowEnhancedMode({ enabled: true, configured: true, status: 'running' }))
    expect(await screen.findByRole('menuitem', { name: '增强模式，执行中' })).toBeInTheDocument()
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
  })

  it('CHANGED 事件携带空 detail 时被忽略（不把状态打回初值）', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    render(<EnhancedModeStatus />)
    await screen.findByRole('menuitem', { name: /增强模式，可用/ })
    act(() => window.dispatchEvent(new CustomEvent(WORKFLOW_ENHANCED_MODE_CHANGED_EVENT)))
    expect(await screen.findByRole('menuitem', { name: /增强模式，可用/ })).toBeInTheDocument()
  })

  it('REFRESH 事件重新读取后端权威状态', async () => {
    vi.mocked(getWorkflowEnhancedMode)
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'ready' })
      .mockResolvedValueOnce({ enabled: false, configured: true, status: 'disabled' })
    render(<EnhancedModeStatus />)
    await screen.findByRole('menuitem', { name: /增强模式，可用/ })

    act(() => requestWorkflowEnhancedModeRefresh())

    expect(await screen.findByRole('menuitem', { name: /增强模式，已关闭/ })).toBeInTheDocument()
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(2)
  })

  it('窗口重新获得焦点时刷新（与 EnhancedModeToggle 同一订阅面）', async () => {
    vi.mocked(getWorkflowEnhancedMode)
      .mockResolvedValueOnce({ enabled: false, configured: true, status: 'disabled' })
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'ready' })
    render(<EnhancedModeStatus />)
    await screen.findByRole('menuitem', { name: /增强模式，已关闭/ })

    fireEvent(window, new Event('focus'))

    expect(await screen.findByRole('menuitem', { name: /增强模式，可用/ })).toBeInTheDocument()
  })

  it('读取失败如实播报「服务不可用」，不谎报可用', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockRejectedValue(new Error('ipc down'))
    render(<EnhancedModeStatus />)
    expect(
      await screen.findByRole('menuitem', { name: '增强模式，服务不可用' }),
    ).toBeInTheDocument()
  })

  it('竞态：较早发出的慢请求结果不得覆盖较新的请求', async () => {
    let resolveOld!: (value: { enabled: boolean; configured: boolean; status: string }) => void
    vi.mocked(getWorkflowEnhancedMode)
      .mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveOld = resolve
          }),
      )
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'ready' })
    render(<EnhancedModeStatus />)
    fireEvent(window, new Event('focus'))
    await screen.findByRole('menuitem', { name: /增强模式，可用/ })
    await act(async () => {
      resolveOld({ enabled: false, configured: true, status: 'needs_accessibility' })
    })
    expect(screen.getByRole('menuitem', { name: /增强模式，可用/ })).toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: /需辅助功能权限/ })).toBeNull()
  })

  it('卸载后不再响应事件（监听器已清理，不泄漏后台读取）', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    const { unmount } = render(<EnhancedModeStatus />)
    await screen.findByRole('menuitem', { name: /增强模式，可用/ })
    unmount()
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
    fireEvent(window, new Event('focus'))
    act(() => requestWorkflowEnhancedModeRefresh())
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
  })

  it('statusLabel 与 EnhancedModeToggle 共用同一实现（文案不一致会立刻暴露）', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: false,
      status: 'primary_fallback',
    })
    render(<EnhancedModeStatus />)
    expect(item()).toBeTruthy()
    expect(await screen.findByRole('menuitem', { name: '增强模式，主模型' })).toBeInTheDocument()
  })
})
