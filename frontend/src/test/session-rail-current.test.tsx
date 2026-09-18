import { render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import SessionRail from '../main-window/chat/SessionRail'

vi.mock('../main-window/lib/api', () => ({
  listShelfSessions: vi.fn(() =>
    Promise.resolve({
      can_switch: true,
      items: [
        {
          id: 'active-session',
          mode: 'leader',
          title: '当前测试会话',
          preview: '',
          message_count: 2,
          updated_at: Date.now(),
          is_active: true,
        },
      ],
    }),
  ),
  switchSession: vi.fn(),
  renameSession: vi.fn(),
  archiveSession: vi.fn(),
}))

describe('会话工作台当前状态', () => {
  it('保留 aria-current 并显示明确的当前徽标', async () => {
    render(<SessionRail onSessionChanged={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('当前测试会话')).toBeInTheDocument())
    const title = screen.getByText('当前测试会话').closest('button')!
    expect(title).toHaveAttribute('aria-current', 'true')
    expect(title).toBeDisabled()
    expect(screen.getByText('当前')).toBeInTheDocument()
    expect(title.closest('.sr-item')).toHaveClass('active')
  })
})
