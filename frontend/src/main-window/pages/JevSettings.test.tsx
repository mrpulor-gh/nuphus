import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { JevSettings } from './JevSettings'
import {
  clearJevApiKey,
  getJevConfig,
  getWorkflowEnhancedMode,
  saveJevConfig,
  testJevConnection,
} from '../lib/api'

vi.mock('../lib/api', () => ({
  getJevConfig: vi.fn(),
  saveJevConfig: vi.fn(),
  clearJevApiKey: vi.fn(),
  getWorkflowEnhancedMode: vi.fn(),
  testJevConnection: vi.fn(),
}))

describe('JevSettings', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('已配置时只显示 has_key 状态，不回显 API Key', async () => {
    vi.mocked(getJevConfig).mockResolvedValue({
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: true,
      timeout_ms: 10000,
      max_retries: 2,
      fallback_to_primary_model: true,
    })
    render(<JevSettings />)

    const keyInput = await screen.findByLabelText('增强判断模型 API Key')
    await waitFor(() => expect(keyInput).toHaveAttribute('placeholder', '已配置；输入新密钥可覆盖'))
    expect(keyInput).toHaveValue('')
    expect(keyInput).toHaveAttribute('type', 'password')
    expect(
      screen.getByText('已配置', { selector: '.decision-settings-summary strong' }),
    ).toBeInTheDocument()
  })

  it('加载并保存完整 Jev 策略配置', async () => {
    vi.mocked(getJevConfig).mockResolvedValue({
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: true,
      timeout_ms: 8000,
      max_retries: 1,
      fallback_to_primary_model: true,
    })
    vi.mocked(saveJevConfig).mockResolvedValue({
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-1.13.0',
      has_key: true,
      timeout_ms: 15000,
      max_retries: 3,
      fallback_to_primary_model: false,
    })
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    const changed = vi.fn()
    window.addEventListener('nuphus:workflow-enhanced-mode-changed', changed)
    render(<JevSettings />)

    const timeout = await screen.findByLabelText('增强判断模型请求超时')
    expect(timeout).toHaveValue(8000)
    expect(screen.getByLabelText('增强判断模型最大重试次数')).toHaveValue(1)
    expect(screen.getByLabelText('回退到主模型')).toBeChecked()
    expect(screen.queryByText('置信度下限')).not.toBeInTheDocument()

    fireEvent.change(screen.getByLabelText('增强判断模型'), {
      target: { value: 'jev-1.13.0' },
    })
    fireEvent.change(timeout, { target: { value: '15000' } })
    fireEvent.change(screen.getByLabelText('增强判断模型最大重试次数'), {
      target: { value: '3' },
    })
    fireEvent.click(screen.getByLabelText('回退到主模型'))
    fireEvent.click(screen.getByRole('button', { name: '保存配置' }))

    await waitFor(() =>
      expect(saveJevConfig).toHaveBeenCalledWith({
        apiKey: undefined,
        baseUrl: 'https://api.typesafe.ai',
        model: 'jev-1.13.0',
        timeoutMs: 15000,
        maxRetries: 3,
        fallbackToPrimaryModel: false,
      }),
    )
    expect(await screen.findByText('增强判断模型配置已保存')).toBeInTheDocument()
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
    expect((changed.mock.calls[0][0] as CustomEvent).detail).toEqual({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    window.removeEventListener('nuphus:workflow-enhanced-mode-changed', changed)
  })

  it('连接测试保存配置后同步增强模式状态', async () => {
    const config = {
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: true,
      timeout_ms: 10000,
      max_retries: 2,
      fallback_to_primary_model: true,
    }
    vi.mocked(getJevConfig).mockResolvedValue(config)
    vi.mocked(saveJevConfig).mockResolvedValue(config)
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: true,
      status: 'ready',
    })
    vi.mocked(testJevConnection).mockResolvedValue({
      status: 'ready',
      model: 'jev-1.13.0',
    })
    const changed = vi.fn()
    window.addEventListener('nuphus:workflow-enhanced-mode-changed', changed)
    render(<JevSettings />)

    fireEvent.click(await screen.findByRole('button', { name: '测试连接' }))

    await waitFor(() => expect(testJevConnection).toHaveBeenCalledTimes(1))
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
    expect(changed).toHaveBeenCalledTimes(1)
    expect(await screen.findByText('连接正常（jev-1.13.0）')).toBeInTheDocument()
    window.removeEventListener('nuphus:workflow-enhanced-mode-changed', changed)
  })

  it('拒绝越界的策略参数且不调用保存 IPC', async () => {
    vi.mocked(getJevConfig).mockResolvedValue({
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: true,
      timeout_ms: 10000,
      max_retries: 2,
      fallback_to_primary_model: true,
    })
    render(<JevSettings />)

    const timeout = await screen.findByLabelText('增强判断模型请求超时')
    fireEvent.change(timeout, { target: { value: '10' } })
    fireEvent.click(screen.getByRole('button', { name: '保存配置' }))

    expect(await screen.findByText('请求超时必须是 100–120000 毫秒之间的整数')).toBeInTheDocument()
    expect(saveJevConfig).not.toHaveBeenCalled()
  })

  it('清除密钥后同步为主模型状态，不关闭增强模式偏好', async () => {
    vi.mocked(getJevConfig).mockResolvedValue({
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: true,
      timeout_ms: 10000,
      max_retries: 2,
      fallback_to_primary_model: true,
    })
    vi.mocked(clearJevApiKey).mockResolvedValue(undefined)
    vi.mocked(getWorkflowEnhancedMode).mockResolvedValue({
      enabled: true,
      configured: false,
      status: 'primary_fallback',
    })
    const changed = vi.fn()
    window.addEventListener('nuphus:workflow-enhanced-mode-changed', changed)
    render(<JevSettings />)

    fireEvent.click(await screen.findByRole('button', { name: '清除已保存的增强判断模型 API Key' }))
    expect(await screen.findByRole('dialog', { name: '清除 API Key' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '确认清除' }))

    await waitFor(() => expect(clearJevApiKey).toHaveBeenCalledTimes(1))
    expect(getWorkflowEnhancedMode).toHaveBeenCalledTimes(1)
    expect((changed.mock.calls[0][0] as CustomEvent).detail).toEqual({
      enabled: true,
      configured: false,
      status: 'primary_fallback',
    })
    window.removeEventListener('nuphus:workflow-enhanced-mode-changed', changed)
  })

  it('API Key 行提供直达 TypeSafe 控制台的外链', async () => {
    vi.mocked(getJevConfig).mockResolvedValue({
      enabled: false,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: false,
      timeout_ms: 10000,
      max_retries: 2,
      fallback_to_primary_model: true,
    })
    render(<JevSettings />)

    // 未配置时也需要能立刻拿到 Key：链接常驻 API Key 行，不随 has_key 变化
    const link = await screen.findByRole('link', { name: /获取 API Key/ })
    expect(link).toHaveAttribute('href', 'https://console.typesafe.ai/')
    // 桌面端 WebView 不处理 target：真实跳转由 App 层 externalLink 捕获接管
    expect(link).toHaveAttribute('target', '_blank')
  })
})
