import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EnhancedModeToggle } from './EnhancedModeToggle'
import { getWorkflowEnhancedMode, setWorkflowEnhancedMode } from '../lib/api'
import { requestWorkflowEnhancedModeRefresh } from './enhancedModeEvents'

vi.mock('../lib/api', () => ({
  getWorkflowEnhancedMode: vi.fn(),
  setWorkflowEnhancedMode: vi.fn(),
}))

describe('EnhancedModeToggle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('未配置时提示影响，确认后由主模型继续增强模式', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: false,
      status: 'unconfigured',
    })
    vi.mocked(setWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: false,
      status: 'primary_fallback',
    })
    render(<EnhancedModeToggle compact />)

    const button = await screen.findByRole('button', { name: /增强模式，未配置/ })
    fireEvent.click(button)

    expect(await screen.findByRole('dialog', { name: '未配置增强判断模型' })).toBeInTheDocument()
    expect(screen.getByText(/效果可能较差，并会消耗更多 Token/)).toBeInTheDocument()
    expect(setWorkflowEnhancedMode).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: '仍然开启' }))

    await waitFor(() => expect(setWorkflowEnhancedMode).toHaveBeenCalledWith(true))
    expect(await screen.findByRole('button', { name: /增强模式，主模型/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
  })

  it('未配置时可直达增强判断模型配置页', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: false,
      status: 'unconfigured',
    })
    const navigation = vi.fn()
    window.addEventListener('nuphus-nav-models', navigation)

    render(<EnhancedModeToggle />)
    fireEvent.click(await screen.findByRole('button', { name: /增强模式，未配置/ }))
    fireEvent.click(await screen.findByRole('button', { name: '前往配置' }))

    expect(navigation).toHaveBeenCalledTimes(1)
    expect((navigation.mock.calls[0][0] as CustomEvent).detail).toEqual({ view: 'jev' })
    expect(setWorkflowEnhancedMode).not.toHaveBeenCalled()
    window.removeEventListener('nuphus-nav-models', navigation)
  })

  it('通过结构化 IPC 切换并展示返回状态', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: true,
      status: 'ready',
    })
    vi.mocked(setWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    render(<EnhancedModeToggle />)

    fireEvent.click(await screen.findByRole('button', { name: /增强模式，已关闭/ }))

    await waitFor(() => expect(setWorkflowEnhancedMode).toHaveBeenCalledWith(true))
    expect(await screen.findByRole('button', { name: /增强模式，可用/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
  })

  it('聊天与画布入口共享切换结果', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: true,
      status: 'disabled',
    })
    vi.mocked(setWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })

    render(
      <>
        <EnhancedModeToggle compact />
        <EnhancedModeToggle />
      </>,
    )

    const buttons = await screen.findAllByRole('button', { name: /增强模式，已关闭/ })
    expect(buttons[0]).toHaveClass('is-compact')
    fireEvent.click(buttons[0])

    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: /增强模式，可用/ })).toHaveLength(2),
    )
    for (const button of screen.getAllByRole('button', { name: /增强模式，可用/ })) {
      expect(button).toHaveAttribute('aria-pressed', 'true')
    }
    expect(setWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
  })

  it('会话变化后重新读取后端权威状态', async () => {
    vi.mocked(getWorkflowEnhancedMode)
      .mockResolvedValueOnce({
        enabled: true,
        configured: true,
        status: 'ready',
      })
      .mockResolvedValueOnce({
        enabled: false,
        configured: true,
        status: 'disabled',
      })

    render(<EnhancedModeToggle />)
    expect(await screen.findByRole('button', { name: /增强模式，可用/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    )

    act(() => requestWorkflowEnhancedModeRefresh())

    expect(await screen.findByRole('button', { name: /增强模式，已关闭/ })).toHaveAttribute(
      'aria-pressed',
      'false',
    )
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(2)
  })

  it.each([
    ['needs_accessibility', '需辅助功能权限'],
    ['unsupported_platform', '当前平台未支持'],
  ])('配置存在但 %s 时不显示可用，也不丢失开启偏好', async (status, label) => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status,
    })
    render(<EnhancedModeToggle />)
    expect(await screen.findByRole('button', { name: `增强模式，${label}` })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
    expect(screen.queryByRole('button', { name: /增强模式，可用/ })).not.toBeInTheDocument()
    expect(setWorkflowEnhancedMode).not.toHaveBeenCalled()
  })

  it('从系统设置返回时刷新授权状态，撤销授权后也刷新', async () => {
    vi.mocked(getWorkflowEnhancedMode)
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'needs_accessibility' })
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'ready' })
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'needs_accessibility' })
    const { unmount } = render(<EnhancedModeToggle />)
    await screen.findByRole('button', { name: /增强模式，需辅助功能权限/ })

    fireEvent(window, new Event('focus'))
    await screen.findByRole('button', { name: /增强模式，可用/ })

    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
    fireEvent(document, new Event('visibilitychange'))
    expect(await screen.findByRole('button', { name: /增强模式，需辅助功能权限/ })).toHaveAttribute(
      'aria-pressed',
      'true',
    )
    expect(setWorkflowEnhancedMode).not.toHaveBeenCalled()
    unmount()
    fireEvent(window, new Event('focus'))
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(3)
    visibility.mockRestore()
  })

  it('返回应用后忽略较早请求的过期权限状态', async () => {
    let resolveOld!: (value: { enabled: boolean; configured: boolean; status: string }) => void
    vi.mocked(getWorkflowEnhancedMode)
      .mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveOld = resolve
          }),
      )
      .mockResolvedValueOnce({ enabled: true, configured: true, status: 'ready' })
    render(<EnhancedModeToggle />)
    fireEvent(window, new Event('focus'))
    await screen.findByRole('button', { name: /增强模式，可用/ })
    await act(async () => {
      resolveOld({ enabled: true, configured: true, status: 'needs_accessibility' })
    })
    expect(screen.getByRole('button', { name: /增强模式，可用/ })).toBeInTheDocument()
  })
})
