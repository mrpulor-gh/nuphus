import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { JevSettings } from './JevSettings'
import { getJevConfig } from '../lib/api'

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
      base_url: 'https://api.typesafe.ai',
      model: 'jev-latest',
      has_key: true,
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
})
