import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { EnhancedModeToggle } from './EnhancedModeToggle'
import { getWorkflowEnhancedMode, setWorkflowEnhancedMode } from '../lib/api'

vi.mock('../lib/api', () => ({
  getWorkflowEnhancedMode: vi.fn(),
  setWorkflowEnhancedMode: vi.fn(),
}))

describe('EnhancedModeToggle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('未配置时提示配置入口且不启用', async () => {
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: false,
      configured: false,
      status: 'unconfigured',
    })
    const onNotice = vi.fn()
    render(<EnhancedModeToggle onNotice={onNotice} />)

    const button = await screen.findByRole('button', { name: /增强模式，未配置/ })
    fireEvent.click(button)

    expect(onNotice).toHaveBeenCalledWith(expect.stringContaining('模型设置 → Jev 增强判断'))
    expect(setWorkflowEnhancedMode).not.toHaveBeenCalled()
    expect(button).toHaveAttribute('aria-pressed', 'false')
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
})
