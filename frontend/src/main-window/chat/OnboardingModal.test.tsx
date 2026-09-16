import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { configureLlm, getSupportedProviders } from '../lib/api'
import { OnboardingModal } from './OnboardingModal'

vi.mock('../lib/api', () => ({
  configureLlm: vi.fn(async () => 'ok'),
  getSupportedProviders: vi.fn(),
}))

const customProvider = {
  id: 'custom',
  name: '自定义',
  provider_type: 'custom',
  base_url: '',
  default_model: '',
  auth_header: 'Authorization',
  auth_prefix: 'Bearer ',
}

const builtinProvider = {
  id: 'deepseek',
  name: 'DeepSeek',
  provider_type: 'deepseek',
  base_url: 'https://api.deepseek.com/v1',
  default_model: 'deepseek-chat',
  auth_header: 'Authorization',
  auth_prefix: 'Bearer ',
}

async function openProvider(name: string) {
  render(<OnboardingModal onComplete={() => {}} onSkip={() => {}} />)
  await waitFor(() => expect(screen.getByRole('button', { name: new RegExp(name) })).toBeVisible())
  fireEvent.click(screen.getByRole('button', { name: new RegExp(name) }))
}

describe('OnboardingModal custom provider endpoint', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(getSupportedProviders).mockResolvedValue([customProvider, builtinProvider])
  })

  it('requires a valid HTTP(S) endpoint for a custom provider', async () => {
    await openProvider('自定义')

    expect(screen.getByLabelText('API 接入点')).toBeVisible()
    fireEvent.change(screen.getByLabelText('API Key'), { target: { value: 'key' } })
    fireEvent.change(screen.getByLabelText('模型名称'), { target: { value: 'custom-model' } })
    fireEvent.click(screen.getByRole('button', { name: '完成' }))
    expect(screen.getByText('请输入 API 接入点')).toBeVisible()

    fireEvent.change(screen.getByLabelText('API 接入点'), { target: { value: 'ftp://invalid' } })
    fireEvent.click(screen.getByRole('button', { name: '完成' }))
    expect(screen.getByText('请输入有效的 HTTP 或 HTTPS API 接入点')).toBeVisible()
    expect(configureLlm).not.toHaveBeenCalled()
  })

  it('submits the custom endpoint after trimming it', async () => {
    await openProvider('自定义')

    fireEvent.change(screen.getByLabelText('API 接入点'), {
      target: { value: '  https://gateway.example.com/v1  ' },
    })
    fireEvent.change(screen.getByLabelText('API Key'), { target: { value: ' key ' } })
    fireEvent.change(screen.getByLabelText('模型名称'), { target: { value: ' model ' } })
    fireEvent.click(screen.getByRole('button', { name: '完成' }))

    await waitFor(() =>
      expect(configureLlm).toHaveBeenCalledWith(
        'key',
        'model',
        'custom',
        'https://gateway.example.com/v1',
      ),
    )
  })

  it('keeps the predefined endpoint for built-in providers', async () => {
    await openProvider('DeepSeek')

    expect(screen.queryByLabelText('API 接入点')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('API Key'), { target: { value: 'key' } })
    fireEvent.click(screen.getByRole('button', { name: '完成' }))

    await waitFor(() =>
      expect(configureLlm).toHaveBeenCalledWith(
        'key',
        'deepseek-chat',
        'deepseek',
        'https://api.deepseek.com/v1',
      ),
    )
  })
})
