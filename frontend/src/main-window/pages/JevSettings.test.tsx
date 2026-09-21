import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { JevSettings } from './JevSettings'
import { getJevConfig, saveJevConfig } from '../lib/api'

vi.mock('../lib/api', () => ({
  getJevConfig: vi.fn(),
  saveJevConfig: vi.fn(),
  clearJevApiKey: vi.fn(),
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
      confidence_floor: 0.2,
    })
    render(<JevSettings />)

    const keyInput = await screen.findByLabelText('Jev API Key')
    await waitFor(() => expect(keyInput).toHaveAttribute('placeholder', '已配置；输入新密钥可覆盖'))
    expect(keyInput).toHaveValue('')
    expect(keyInput).toHaveAttribute('type', 'password')
    expect(
      screen.getByText('已配置', { selector: '.jev-settings-summary strong' }),
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
      confidence_floor: 0.35,
    })
    vi.mocked(saveJevConfig).mockResolvedValue({
      enabled: true,
      base_url: 'https://api.typesafe.ai',
      model: 'jev-1.13.0',
      has_key: true,
      timeout_ms: 15000,
      max_retries: 3,
      fallback_to_primary_model: false,
      confidence_floor: 0.6,
    })
    render(<JevSettings />)

    const timeout = await screen.findByLabelText('Jev 请求超时')
    expect(timeout).toHaveValue(8000)
    expect(screen.getByLabelText('Jev 最大重试次数')).toHaveValue(1)
    expect(screen.getByLabelText('Jev 置信度下限')).toHaveValue(0.35)
    expect(screen.getByLabelText('回退到主模型')).toBeChecked()

    fireEvent.change(screen.getByLabelText('Jev 模型'), { target: { value: 'jev-1.13.0' } })
    fireEvent.change(timeout, { target: { value: '15000' } })
    fireEvent.change(screen.getByLabelText('Jev 最大重试次数'), { target: { value: '3' } })
    fireEvent.change(screen.getByLabelText('Jev 置信度下限'), { target: { value: '0.6' } })
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
        confidenceFloor: 0.6,
      }),
    )
    expect(await screen.findByText('Jev 配置已保存')).toBeInTheDocument()
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
      confidence_floor: 0.2,
    })
    render(<JevSettings />)

    const confidence = await screen.findByLabelText('Jev 置信度下限')
    fireEvent.change(confidence, { target: { value: '1.5' } })
    fireEvent.click(screen.getByRole('button', { name: '保存配置' }))

    expect(await screen.findByText('置信度下限必须是 0–1 之间的数字')).toBeInTheDocument()
    expect(saveJevConfig).not.toHaveBeenCalled()
  })
})
