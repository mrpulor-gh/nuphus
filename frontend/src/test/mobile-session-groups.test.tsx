import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import NavBar from '../mobile/components/NavBar'
import type { ActivityState } from '../mobile/store'
import type { ShelfSessions } from '../mobile/api'

/**
 * 移动端会话清单分组（Phase 2）回归。
 *
 * 与桌面 SessionRail 共用同一返回体与同一纯函数（sessionGroups.ts）：
 * 这里只断言渲染层是否按分组顺序建组、组头是否可见、归档组是否整组隐藏、
 * 折叠上限是否与桌面同源，以及点选仍走 onSwitchSession(id, mode)。
 */
vi.mock('../mobile/api', () => ({
  fetchCustomAgents: vi.fn(() => Promise.resolve([])),
  fetchModelConfig: vi.fn(() => Promise.resolve(null)),
  switchMobileMode: vi.fn(() => Promise.resolve({ ok: true })),
  switchMobileModel: vi.fn(() => Promise.resolve({ ok: true })),
}))

const activity: ActivityState = {
  running: false,
  goal: '',
  mode: 'leader',
  tools: [],
  paused: false,
}

function sessionsFixture(overrides: Partial<ShelfSessions> = {}): ShelfSessions {
  return {
    can_switch: true,
    items: [
      {
        id: 'bm1',
        mode: 'leader',
        title: '一号会话',
        message_count: 3,
        updated_at: 2_000,
        is_active: true,
        project_path: 'E:\\NUS\\1',
      },
      {
        id: 'bm1-b',
        mode: 'workflow',
        title: '一号次会话',
        message_count: 1,
        updated_at: 3_000,
        is_active: false,
        project_path: 'E:\\NUS\\1',
      },
      {
        id: 'arch',
        mode: 'leader',
        title: '已归档目录会话',
        message_count: 1,
        updated_at: 1_000,
        is_active: false,
        project_path: 'E:\\work\\Old',
      },
      {
        id: 'lonely',
        mode: 'custom',
        title: '无归属会话',
        message_count: 1,
        updated_at: 500,
        is_active: false,
        project_path: null,
      },
    ],
    projects: [
      { path: 'E:\\NUS\\1', name: '一号', is_current: false, auto: false },
      { path: 'E:\\NUS\\Nuphus', name: 'Nuphus', is_current: true, auto: false },
    ],
    archived_projects: [
      { path: 'E:\\work\\Old', name: '已归档目录', is_current: false, auto: false },
    ],
    collapsed_limit: 1,
    // 桌面 ⋯ 菜单设置的排序偏好（同一返回体下发，移动端只读跟随）
    sort_prefs: { group_order: 'bookmark', sort_key: 'updated' },
    ...overrides,
  }
}

function renderNav(sessions: ShelfSessions, onSwitchSession = vi.fn()) {
  render(
    <NavBar
      wsStatus="online"
      activity={activity}
      token="token"
      model="test-model"
      sessions={sessions}
      onSwitchSession={onSwitchSession}
    />,
  )
  // 打开设置抽屉（会话列表在主视图内）
  fireEvent.click(screen.getByLabelText('菜单'))
  return { onSwitchSession }
}

describe('移动端会话列表分组', () => {
  it('按 projects[] 顺序建组，未分组末位，归档组整组隐藏', async () => {
    renderNav(sessionsFixture())
    const sheet = screen.getByRole('dialog', { name: '设置' })

    await waitFor(() => expect(within(sheet).getByText('一号')).toBeInTheDocument())
    const heads = Array.from(sheet.querySelectorAll('.mobile-sess-group-name')).map(
      e => e.textContent,
    )
    expect(heads).toEqual(['一号', 'Nuphus', '未分组'])

    // 归档组：组名与其会话都不可见
    expect(within(sheet).queryByText('已归档目录')).not.toBeInTheDocument()
    expect(within(sheet).queryByText('已归档目录会话')).not.toBeInTheDocument()
    // 未分组组：只收无归属会话
    const groupEl = (name: string) =>
      Array.from(sheet.querySelectorAll('.mobile-sess-group')).find(
        g => g.querySelector('.mobile-sess-group-name')?.textContent === name,
      ) as HTMLElement
    const ungrouped = groupEl('未分组')
    expect(within(ungrouped).getByText('无归属会话')).toBeInTheDocument()
    // 空文件夹（书签存在但无会话）→ 组仍显示 + 弱提示
    // （组名「Nuphus」与抽屉品牌名重复，故按组容器定位而非文本全局查找）
    expect(within(groupEl('Nuphus')).getByText('该文件夹暂无会话')).toBeInTheDocument()
  })

  it('组内按 collapsed_limit 折叠并支持展开其余；点选仍传 (id, mode)', async () => {
    const onSwitchSession = vi.fn()
    renderNav(sessionsFixture(), onSwitchSession)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const group = screen.getByText('一号').closest('.mobile-sess-group') as HTMLElement
    // collapsed_limit=1 → 组内只显示最近 1 条（updated_at 倒序：一号次会话在前）
    expect(within(group).getByText('一号次会话')).toBeInTheDocument()
    expect(within(group).queryByText('一号会话')).not.toBeInTheDocument()
    fireEvent.click(within(group).getByText('展开其余 1 个会话'))
    await waitFor(() => expect(within(group).getByText('一号会话')).toBeInTheDocument())

    fireEvent.click(within(group).getByText('一号会话'))
    expect(onSwitchSession).toHaveBeenCalledWith('bm1', 'leader')
  })

  it('排序偏好跟随桌面（同一返回体 sort_prefs：组序维度 + 组内键）', async () => {
    renderNav(
      sessionsFixture({
        collapsed_limit: 6,
        items: [
          {
            id: 'bm1',
            mode: 'leader',
            title: '一号会话',
            message_count: 3,
            updated_at: 2_000,
            created_at: 9_000,
            is_active: false,
            project_path: 'E:\\NUS\\1',
          },
          {
            id: 'bm1-b',
            mode: 'workflow',
            title: '一号次会话',
            message_count: 1,
            updated_at: 1_000,
            created_at: 8_000,
            is_active: false,
            project_path: 'E:\\NUS\\1',
          },
          {
            id: 'np',
            mode: 'leader',
            title: 'Nuphus会话',
            message_count: 1,
            updated_at: 5_000,
            created_at: 1_000,
            is_active: true,
            project_path: 'E:\\NUS\\Nuphus',
          },
        ],
        // 桌面选了「近期项目 + 创建时间」
        sort_prefs: { group_order: 'recent', sort_key: 'created' },
      }),
    )
    const sheet = screen.getByRole('dialog', { name: '设置' })
    await waitFor(() => expect(within(sheet).getByText('一号')).toBeInTheDocument())

    // 组序维度：Nuphus 最近会话 5000 > 一号 2000（书签序本是一号在前）
    const heads = Array.from(sheet.querySelectorAll('.mobile-sess-group-name')).map(
      e => e.textContent,
    )
    expect(heads).toEqual(['Nuphus', '一号'])
    // 组内键：创建时间升序（一号次 8000 在 一号 9000 之前）
    const group = within(sheet).getByText('一号').closest('.mobile-sess-group') as HTMLElement
    // 标题文本取 .mobile-sess-title 的直接文本节点（mode 徽标是 aria-hidden 的子元素）
    const titles = Array.from(group.querySelectorAll('.mobile-sess-title')).map(e =>
      Array.from(e.childNodes)
        .filter(n => n.nodeType === Node.TEXT_NODE)
        .map(n => n.textContent ?? '')
        .join('')
        .trim(),
    )
    expect(titles).toEqual(['一号次会话', '一号会话'])
  })

  it('组头可整组收起/展开', async () => {
    renderNav(sessionsFixture())
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const head = screen.getByText('一号').closest('.mobile-sess-group-head') as HTMLElement
    expect(head).toHaveAttribute('aria-expanded', 'true')
    fireEvent.click(head)
    await waitFor(() => expect(head).toHaveAttribute('aria-expanded', 'false'))
    expect(screen.queryByText('一号次会话')).not.toBeInTheDocument()
  })
})
