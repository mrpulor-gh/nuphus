import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { CompactModal } from './CompactModal'

describe('CompactModal', () => {
  it('settings 层级会把 portal 遮罩提升到设置中心之上', () => {
    render(
      <CompactModal open onClose={vi.fn()} title="定时运行" layer="settings">
        内容
      </CompactModal>,
    )

    expect(screen.getByRole('dialog', { name: '定时运行' }).parentElement).toHaveClass(
      'compact-overlay--above-settings',
    )
  })
})
