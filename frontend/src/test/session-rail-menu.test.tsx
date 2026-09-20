import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SessionRail from '../main-window/chat/SessionRail'

/**
 * 会话工作台「项目」行图标与 ⋯ 菜单体系：
 * `⋯`（整理侧边栏 / 排序条件 / 恢复隐藏项目，含两级子菜单）+ `📁+`（新建项目文件夹）。
 *
 * 断言只打真实调用（api wrapper / 回调）与 DOM 顺序/文本；**排序语义本身由
 * src/test/session-groups.test.ts 覆盖**——这里验证「组件把后端读数传给同一个纯函数、
 * 选择后立即重排并落盘、失败可感知」。
 */
const listShelfSessions = vi.fn()
const switchSession = vi.fn()
const renameSession = vi.fn()
const archiveSession = vi.fn()
const setProjectBookmarks = vi.fn()
const setProjectFolderArchived = vi.fn()
const setSessionSortPrefs = vi.fn()

vi.mock('../main-window/lib/api', () => ({
  listShelfSessions: () => listShelfSessions(),
  switchSession: (...args: unknown[]) => switchSession(...args),
  renameSession: (...args: unknown[]) => renameSession(...args),
  archiveSession: (...args: unknown[]) => archiveSession(...args),
  setProjectBookmarks: (...args: unknown[]) => setProjectBookmarks(...args),
  setProjectFolderArchived: (...args: unknown[]) => setProjectFolderArchived(...args),
  setSessionSortPrefs: (...args: unknown[]) => setSessionSortPrefs(...args),
  SESSION_GROUP_LIMIT_CHANGED_EVENT: 'nuphus:session-group-limit-changed',
}))

/** 时间基准：夹具里 created/updated 刻意相反，便于区分两个排序维度 */
const T = 1_700_000_000_000

interface ItemOverrides {
  updated_at?: number
  created_at?: number
}

/** 会话条目夹具（字段与后端 items[] 一致） */
function item(id: string, title: string, project_path: string | null, o: ItemOverrides = {}) {
  const updated = o.updated_at ?? T
  return {
    id,
    mode: 'leader',
    title,
    preview: '',
    message_count: 1,
    updated_at: updated,
    created_at: o.created_at ?? updated,
    is_active: false,
    project_path,
  }
}

/**
 * 返回体夹具：
 * - 组（书签序）：一号 / 空文件夹（无会话）/ 二号，加「未分组」末位；
 * - 一号的三个会话把两个维度**岔开**（否则测不出区别）：
 *   updated 倒序 = [老会话(500), 新会话(400), 中间会话(100)]；
 *   created 升序 = [老会话(100), 中间会话(500), 新会话(900)]；
 * - 组最新会话时间：一号(500) > 二号(300) > 空文件夹(无) → 「近期项目」把空组压到末位。
 */
function shelfResponse(overrides: Record<string, unknown> = {}) {
  return {
    can_switch: true,
    items: [
      item('s-a1', '一号老会话', 'E:\\A', { created_at: T + 100, updated_at: T + 500 }),
      item('s-a2', '一号新会话', 'E:\\A', { created_at: T + 900, updated_at: T + 400 }),
      item('s-a3', '一号中间会话', 'E:\\A', { created_at: T + 500, updated_at: T + 100 }),
      item('s-b1', '二号会话', 'E:\\B', { created_at: T + 300, updated_at: T + 300 }),
      item('s-b2', '二号次会话', 'E:\\B', { created_at: T + 200, updated_at: T + 200 }),
      item('s-none', '无归属会话', null, { created_at: T + 500, updated_at: T + 500 }),
    ],
    projects: [
      { path: 'E:\\A', name: '一号', is_current: false, auto: false },
      { path: 'E:\\Empty', name: '空文件夹', is_current: false, auto: false },
      { path: 'E:\\B', name: '二号', is_current: true, auto: false },
    ],
    archived_projects: [
      { path: 'E:\\work\\Old', name: '已归档目录', is_current: false, auto: false },
    ],
    collapsed_limit: 6,
    sort_prefs: { group_order: 'bookmark', sort_key: 'updated' },
    ...overrides,
  }
}

/**
 * 「后端已落盘的排序偏好」夹具：`set_session_sort_prefs` 写入、`list_shelf_sessions`
 * 读回 —— 这样才真正验证「选择 → 落盘 → 重启/轮询读回保持」的往返链路。
 */
let backendPrefs: { group_order: string; sort_key: string }

function renderRail(props: Partial<Parameters<typeof SessionRail>[0]> = {}) {
  const onOpenProjectDir = vi.fn()
  const utils = render(
    <SessionRail
      onSessionChanged={vi.fn()}
      onNewChat={vi.fn()}
      onOpenProjectDir={onOpenProjectDir}
      onSwitchProjectDir={vi.fn(async () => true)}
      onModeSwitched={vi.fn()}
      {...props}
    />,
  )
  return { ...utils, onOpenProjectDir }
}

/** 抽屉收起时内容带 aria-hidden（getByRole 不可达）→ 交互前先展开 */
function openDrawer() {
  fireEvent.click(document.querySelector('.session-rail-chip') as HTMLElement)
}

/** 打开 ⋯ 菜单（触发器是按钮，菜单是 role=menu：用 role 查询避免撞名） */
function openMenu() {
  fireEvent.click(screen.getByRole('button', { name: '项目菜单' }))
  return screen.getByRole('menu', { name: '项目菜单' })
}

/** 展开某个子菜单（点击带 aria-haspopup 的项） */
function openSubmenu(root: HTMLElement, name: string | RegExp) {
  fireEvent.click(within(root).getByRole('menuitem', { name }))
  return screen.getByRole('menu', { name: typeof name === 'string' ? name : name.source })
}

function groupNames(): string[] {
  return Array.from(document.querySelectorAll('.sr-group-name')).map(e => e.textContent ?? '')
}

function titlesIn(groupName: string): string[] {
  const group = screen.getByText(groupName).closest('.sr-group') as HTMLElement
  return Array.from(group.querySelectorAll('.sr-title-btn')).map(e => e.textContent ?? '')
}

describe('会话工作台「项目」行：图标与 ⋯ 菜单', () => {
  beforeEach(() => {
    backendPrefs = { group_order: 'bookmark', sort_key: 'updated' }
    listShelfSessions
      .mockReset()
      .mockImplementation(async () => shelfResponse({ sort_prefs: { ...backendPrefs } }))
    switchSession.mockReset().mockResolvedValue(undefined)
    renameSession.mockReset().mockResolvedValue(undefined)
    archiveSession.mockReset().mockResolvedValue(undefined)
    setProjectBookmarks.mockReset().mockResolvedValue([])
    setProjectFolderArchived.mockReset().mockResolvedValue([])
    setSessionSortPrefs
      .mockReset()
      .mockImplementation(async (groupOrder: string, sortKey: string) => {
        backendPrefs = { group_order: groupOrder, sort_key: sortKey }
        return { ...backendPrefs }
      })
  })

  it('「项目」行两个图标：⋯ 在前、📁+ 在后；📁+ 走既有项目中心入口', async () => {
    const { onOpenProjectDir } = renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const label = document.querySelector('.sr-list-label') as HTMLElement
    const menuBtn = within(label).getByLabelText('项目菜单')
    const newFolderBtn = within(label).getByLabelText('新建项目文件夹')

    const following = Node.DOCUMENT_POSITION_FOLLOWING
    expect(menuBtn.compareDocumentPosition(newFolderBtn) & following).toBeTruthy()

    fireEvent.click(newFolderBtn)
    expect(onOpenProjectDir).toHaveBeenCalledTimes(1)
    // 打开项目中心前先收起菜单（不叠在弹窗之上）
    expect(screen.queryByRole('menu', { name: '项目菜单' })).not.toBeInTheDocument()
  })

  it('⋯ 菜单三项齐备，且主菜单不渲染子菜单项', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const menu = openMenu()
    expect(within(menu).getAllByRole('menuitem')).toHaveLength(3)
    expect(within(menu).getByRole('menuitem', { name: '整理侧边栏' })).toHaveAttribute(
      'aria-haspopup',
      'menu',
    )
    expect(within(menu).getByRole('menuitem', { name: '排序条件' })).toHaveAttribute(
      'aria-expanded',
      'false',
    )
    expect(within(menu).getByRole('menuitem', { name: '恢复隐藏项目 (1)' })).toBeInTheDocument()
  })

  it('子菜单层级：「整理侧边栏」两项 / 「排序条件」三项（第三项再套一级）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const arrange = openSubmenu(openMenu(), '整理侧边栏')
    expect(within(arrange).getAllByRole('menuitem')).toHaveLength(2)
    expect(within(arrange).getByRole('menuitem', { name: '全部展开' })).toBeInTheDocument()
    expect(within(arrange).getByRole('menuitem', { name: '全部关闭' })).toBeInTheDocument()

    const sort = openSubmenu(screen.getByRole('menu', { name: '项目菜单' }), '排序条件')
    expect(within(sort).getAllByRole('menuitemradio')).toHaveLength(2)
    expect(within(sort).getByRole('menuitemradio', { name: '按项目' })).toBeInTheDocument()
    expect(within(sort).getByRole('menuitemradio', { name: '近期项目' })).toBeInTheDocument()

    const time = openSubmenu(sort, '按时间顺序')
    expect(within(time).getAllByRole('menuitemradio')).toHaveLength(2)
  })

  it('✓ 选中态与后端偏好一致（默认为「按项目 / 更新时间」）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const sort = openSubmenu(openMenu(), '排序条件')
    expect(within(sort).getByRole('menuitemradio', { name: '按项目' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    expect(within(sort).getByRole('menuitemradio', { name: '近期项目' })).toHaveAttribute(
      'aria-checked',
      'false',
    )

    const time = openSubmenu(sort, '按时间顺序')
    expect(within(time).getByRole('menuitemradio', { name: '更新时间' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    expect(within(time).getByRole('menuitemradio', { name: '创建时间' })).toHaveAttribute(
      'aria-checked',
      'false',
    )
  })

  it('切「近期项目」：调 set_session_sort_prefs 且组顺序立即重排（空组末位、未分组恒末位）', async () => {
    renderRail()
    await waitFor(() => expect(groupNames()).toEqual(['一号', '空文件夹', '二号', '未分组']))

    openDrawer()
    const sort = openSubmenu(openMenu(), '排序条件')
    fireEvent.click(within(sort).getByRole('menuitemradio', { name: '近期项目' }))

    await waitFor(() => expect(setSessionSortPrefs).toHaveBeenCalledWith('recent', 'updated'))
    await waitFor(() => expect(groupNames()).toEqual(['一号', '二号', '空文件夹', '未分组']))
    // 组内顺序不受组序维度影响（仍是更新时间倒序）
    expect(titlesIn('一号')).toEqual(['一号老会话', '一号新会话', '一号中间会话'])
  })

  it('切「按时间顺序 → 创建时间」：只改组内序，组顺序不动', async () => {
    renderRail()
    await waitFor(() => expect(groupNames()).toEqual(['一号', '空文件夹', '二号', '未分组']))

    openDrawer()
    const sort = openSubmenu(openMenu(), '排序条件')
    const time = openSubmenu(sort, '按时间顺序')
    fireEvent.click(within(time).getByRole('menuitemradio', { name: '创建时间' }))

    await waitFor(() => expect(setSessionSortPrefs).toHaveBeenCalledWith('bookmark', 'created'))
    await waitFor(() =>
      expect(titlesIn('一号')).toEqual(['一号老会话', '一号中间会话', '一号新会话']),
    )
    expect(titlesIn('二号')).toEqual(['二号次会话', '二号会话'])
    expect(groupNames()).toEqual(['一号', '空文件夹', '二号', '未分组'])
  })

  it('启动即按后端偏好渲染（落盘偏好重启后保持：recent + created）', async () => {
    // 模拟「上次退出前已落盘」：list 返回体直接带 recent + created
    backendPrefs = { group_order: 'recent', sort_key: 'created' }
    renderRail()
    await waitFor(() => expect(groupNames()).toEqual(['一号', '二号', '空文件夹', '未分组']))
    expect(titlesIn('一号')).toEqual(['一号老会话', '一号中间会话', '一号新会话'])

    // 菜单选中态与后端读数一致
    openDrawer()
    const sort = openSubmenu(openMenu(), '排序条件')
    expect(within(sort).getByRole('menuitemradio', { name: '近期项目' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
    const time = openSubmenu(sort, '按时间顺序')
    expect(within(time).getByRole('menuitemradio', { name: '创建时间' })).toHaveAttribute(
      'aria-checked',
      'true',
    )
  })

  it('排序写盘失败：回滚到切换前的排序并给出可感知提示', async () => {
    setSessionSortPrefs.mockRejectedValue(new Error('IPC failed'))
    renderRail()
    await waitFor(() => expect(groupNames()).toEqual(['一号', '空文件夹', '二号', '未分组']))

    openDrawer()
    const sort = openSubmenu(openMenu(), '排序条件')
    fireEvent.click(within(sort).getByRole('menuitemradio', { name: '近期项目' }))

    await waitFor(() => expect(screen.getByText('排序设置未保存，请重试')).toBeInTheDocument())
    // 回滚：组顺序回到书签序
    expect(groupNames()).toEqual(['一号', '空文件夹', '二号', '未分组'])
  })

  it('整理侧边栏：全部关闭收起所有组，全部展开恢复', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const arrange = openSubmenu(openMenu(), '整理侧边栏')
    fireEvent.click(within(arrange).getByRole('menuitem', { name: '全部关闭' }))
    await waitFor(() => expect(screen.queryByText('一号老会话')).not.toBeInTheDocument())
    // 组头仍在，只有组体收起
    expect(groupNames()).toEqual(['一号', '空文件夹', '二号', '未分组'])

    const arrange2 = openSubmenu(openMenu(), '整理侧边栏')
    fireEvent.click(within(arrange2).getByRole('menuitem', { name: '全部展开' }))
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    expect(screen.getByText('二号次会话')).toBeInTheDocument()
  })

  it('恢复隐藏项目 (N)：子菜单直接列出归档文件夹，点选调 set_project_folder_archived(path,false) 并刷新', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()
    const before = listShelfSessions.mock.calls.length

    const restore = openSubmenu(openMenu(), '恢复隐藏项目 (1)')
    fireEvent.click(within(restore).getByRole('menuitem', { name: '已归档目录' }))

    await waitFor(() =>
      expect(setProjectFolderArchived).toHaveBeenCalledWith('E:\\work\\Old', false),
    )
    // 恢复后刷新列表（菜单保持打开，可连续恢复多个）
    await waitFor(() => expect(listShelfSessions.mock.calls.length).toBeGreaterThan(before))
    expect(screen.getByRole('menu', { name: '恢复隐藏项目 (1)' })).toBeInTheDocument()
  })

  it('无归档项时「恢复隐藏项目 (0)」禁用，且不渲染空子菜单', async () => {
    listShelfSessions.mockImplementation(async () =>
      shelfResponse({ archived_projects: [], sort_prefs: { ...backendPrefs } }),
    )
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const menu = openMenu()
    const restoreItem = within(menu).getByRole('menuitem', { name: '恢复隐藏项目 (0)' })
    expect(restoreItem).toBeDisabled()
    fireEvent.click(restoreItem)
    expect(screen.queryByRole('menu', { name: '恢复隐藏项目 (0)' })).not.toBeInTheDocument()
  })

  it('Esc 逐层收菜单（时间顺序 → 排序条件 → 主菜单），不连带收起抽屉', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号老会话')).toBeInTheDocument())
    openDrawer()

    const sort = openSubmenu(openMenu(), '排序条件')
    const time = openSubmenu(sort, '按时间顺序')

    // ① 第一层 Esc：收第三层「按时间顺序」，父级「排序条件」仍在
    fireEvent.keyDown(within(time).getByRole('menuitemradio', { name: '创建时间' }), {
      key: 'Escape',
    })
    expect(screen.queryByRole('menu', { name: '按时间顺序' })).not.toBeInTheDocument()
    expect(screen.getByRole('menu', { name: '排序条件' })).toBeInTheDocument()

    // ② 第二层 Esc：收「排序条件」，主菜单仍在
    fireEvent.keyDown(document.querySelector('.sr-list-label') as HTMLElement, { key: 'Escape' })
    await waitFor(() =>
      expect(screen.queryByRole('menu', { name: '排序条件' })).not.toBeInTheDocument(),
    )
    expect(screen.getByRole('menu', { name: '项目菜单' })).toBeInTheDocument()

    // ③ 第三层 Esc：收主菜单，抽屉仍在（收菜单不连带收起抽屉）
    fireEvent.keyDown(document.querySelector('.sr-list-label') as HTMLElement, { key: 'Escape' })
    await waitFor(() =>
      expect(screen.queryByRole('menu', { name: '项目菜单' })).not.toBeInTheDocument(),
    )
    expect(document.querySelector('.session-rail-drawer')).toHaveClass('is-open')

    // 点击菜单外关闭（抽屉也一并收起，符合既有「面板外点击」语义）
    openMenu()
    fireEvent.mouseDown(document.body)
    await waitFor(() =>
      expect(screen.queryByRole('menu', { name: '项目菜单' })).not.toBeInTheDocument(),
    )
  })
})
