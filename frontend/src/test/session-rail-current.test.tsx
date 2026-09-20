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
          // Phase 1 起条目带归属路径（null = 无归属 → 归入「未分组」）
          project_path: null,
        },
      ],
      projects: [],
      archived_projects: [],
      collapsed_limit: 6,
    }),
  ),
  switchSession: vi.fn(),
  renameSession: vi.fn(),
  archiveSession: vi.fn(),
  setProjectBookmarks: vi.fn(),
  setProjectFolderArchived: vi.fn(),
  SESSION_GROUP_LIMIT_CHANGED_EVENT: 'nuphus:session-group-limit-changed',
}))

describe('会话工作台当前状态', () => {
  it('保留 aria-current 并显示明确的当前徽标', async () => {
    render(<SessionRail onSessionChanged={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('当前测试会话')).toBeInTheDocument())
    const title = screen.getByText('当前测试会话').closest('button')!
    expect(title).toHaveAttribute('aria-current', 'true')
    expect(title).toBeDisabled()
    // 「当前」徽标：组头的 is_current 徽标只在有当前工作目录组时出现，
    // 此处无 projects → 只剩会话行内的当前徽标
    expect(screen.getByText('当前')).toBeInTheDocument()
    expect(title.closest('.sr-item')).toHaveClass('active')
    // 无归属会话落「未分组」兜底组（组头 + 行内徽标两处「当前」不会混淆）
    expect(screen.getByText('未分组')).toBeInTheDocument()
  })
})
