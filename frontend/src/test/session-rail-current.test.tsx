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
      // 带一个「当前工作目录」分组：旧版会在组头渲染第二个「当前」徽标（本次已移除），
      // 该分组存在时本用例才能证明「当前」唯一出现在会话行
      projects: [{ path: 'E:\\NUS\\Nuphus', name: 'Nuphus', is_current: true, auto: false }],
      archived_projects: [],
      collapsed_limit: 6,
      sort_prefs: { group_order: 'bookmark', sort_key: 'updated' },
    }),
  ),
  switchSession: vi.fn(),
  renameSession: vi.fn(),
  archiveSession: vi.fn(),
  setProjectBookmarks: vi.fn(),
  setProjectFolderArchived: vi.fn(),
  setSessionSortPrefs: vi.fn(),
  SESSION_GROUP_LIMIT_CHANGED_EVENT: 'nuphus:session-group-limit-changed',
}))

describe('会话工作台当前状态', () => {
  it('保留 aria-current 并显示明确的当前徽标', async () => {
    render(<SessionRail onSessionChanged={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('当前测试会话')).toBeInTheDocument())
    const title = screen.getByText('当前测试会话').closest('button')!
    expect(title).toHaveAttribute('aria-current', 'true')
    expect(title).toBeDisabled()
    // 唯一「当前」= 会话行：即使存在「当前工作目录」分组，组头也不渲染「当前」徽标、
    // 不带 is-current 类（否则 getByText('当前') 会因命中两处而抛错）
    expect(document.querySelector('.sr-group-head.is-current')).toBeNull()
    expect(document.querySelector('.sr-group-badge')).toBeNull()
    expect(screen.getByText('当前')).toBeInTheDocument()
    expect(document.querySelector('.sr-current-badge')).not.toBeNull()
    expect(title.closest('.sr-item')).toHaveClass('active')
    // 无归属会话落「未分组」兜底组
    expect(screen.getByText('未分组')).toBeInTheDocument()
  })
})
