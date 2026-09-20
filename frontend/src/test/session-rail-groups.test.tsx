import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SessionRail from '../main-window/chat/SessionRail'

/**
 * 会话工作台「项目文件夹分组」组件回归（Phase 2）。
 *
 * mock 契约严格对齐 Phase 1 返回体：items（含 project_path）+ projects[] +
 * archived_projects[] + collapsed_limit。断言只打真实调用（api wrapper / 回调），
 * 不 mock 被测分组逻辑本身（分组走 sessionGroups 纯函数，另有其单测）。
 */
const calls: string[] = []

const listShelfSessions = vi.fn()
const switchSession = vi.fn()
const renameSession = vi.fn()
const archiveSession = vi.fn()
const setProjectBookmarks = vi.fn()
const setProjectFolderArchived = vi.fn()

vi.mock('../main-window/lib/api', () => ({
  listShelfSessions: () => listShelfSessions(),
  switchSession: (...args: unknown[]) => switchSession(...args),
  renameSession: (...args: unknown[]) => renameSession(...args),
  archiveSession: (...args: unknown[]) => archiveSession(...args),
  setProjectBookmarks: (...args: unknown[]) => setProjectBookmarks(...args),
  setProjectFolderArchived: (...args: unknown[]) => setProjectFolderArchived(...args),
  SESSION_GROUP_LIMIT_CHANGED_EVENT: 'nuphus:session-group-limit-changed',
}))

/** 当前 active 会话 id：switchSession 成功后后端会切换 active，夹具跟随（否则轮询
 *  会把它当成「外部会话变更」再触发一次 onSessionChanged，测出假的双触发） */
let activeId: string | null = 'cur'

interface ItemOverrides {
  mode?: string
  message_count?: number
  updated_at?: number
}

/** 会话条目夹具（is_active 由 activeId 决定，贴近后端真实语义） */
function item(
  id: string,
  title: string,
  project_path: string | null,
  overrides: ItemOverrides = {},
) {
  return {
    id,
    mode: overrides.mode ?? 'leader',
    title,
    preview: '',
    message_count: overrides.message_count ?? 1,
    updated_at: overrides.updated_at ?? Date.now(),
    is_active: id === activeId,
    project_path,
  }
}

function baseItems() {
  return [
    item('bm1-new', '一号新会话', 'E:\\NUS\\1', { updated_at: Date.now() }),
    item('bm1-old', '一号旧会话', 'E:\\NUS\\1', {
      mode: 'workflow',
      updated_at: Date.now() - 60_000,
    }),
    item('cur', '当前会话', 'E:\\NUS\\Nuphus', { message_count: 2 }),
    item('arch', '归档目录里的会话', 'E:\\work\\Old'),
    item('lonely', '无归属会话', null, { mode: 'custom' }),
  ]
}

/** 后端返回体夹具（字段名/形状与 Phase 1 契约一致） */
function shelfResponse(overrides: Record<string, unknown> = {}) {
  return {
    can_switch: true,
    items: baseItems(),
    projects: [
      { path: 'E:\\NUS\\1', name: '一号', is_current: false, auto: false },
      { path: 'E:\\NUS\\Nuphus', name: 'Nuphus', is_current: true, auto: false },
      { path: 'E:\\NUS\\auto', name: 'auto', is_current: false, auto: true },
    ],
    archived_projects: [
      { path: 'E:\\work\\Old', name: '已归档目录', is_current: false, auto: false },
    ],
    collapsed_limit: 6,
    ...overrides,
  }
}

function renderRail(props: Partial<Parameters<typeof SessionRail>[0]> = {}) {
  const onSwitchProjectDir = vi.fn(async (path: string) => {
    calls.push(`dir:${path}`)
    return true
  })
  const onNewChat = vi.fn(() => calls.push('newChat'))
  const onSessionChanged = vi.fn()
  const utils = render(
    <SessionRail
      onSessionChanged={onSessionChanged}
      onNewChat={onNewChat}
      onOpenProjectDir={vi.fn()}
      onSwitchProjectDir={onSwitchProjectDir}
      onModeSwitched={vi.fn()}
      {...props}
    />,
  )
  return { ...utils, onSwitchProjectDir, onNewChat, onSessionChanged }
}

describe('SessionRail 项目文件夹分组渲染', () => {
  beforeEach(() => {
    calls.length = 0
    activeId = 'cur'
    listShelfSessions.mockReset().mockImplementation(async () => shelfResponse())
    switchSession.mockReset().mockImplementation(async (id: string) => {
      calls.push('switch')
      // 后端切换成功后 active 跟随（夹具同步，避免轮询误判为外部变更）
      activeId = id
    })
    renameSession.mockReset().mockResolvedValue(undefined)
    archiveSession.mockReset().mockResolvedValue(undefined)
    setProjectBookmarks.mockReset().mockResolvedValue([])
    setProjectFolderArchived.mockReset().mockResolvedValue([])
  })

  it('组头按 projects[] 顺序渲染，未分组末位，归档文件夹整组隐藏', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const heads = Array.from(document.querySelectorAll('.sr-group-name')).map(e => e.textContent)
    expect(heads).toEqual(['一号', 'Nuphus', 'auto', '未分组'])

    // 当前工作目录组带「当前」徽标，但顺序不上浮（仍在第 2 位）
    const currentHead = screen.getByText('Nuphus').closest('.sr-group-head')!
    expect(within(currentHead as HTMLElement).getByText('当前')).toBeInTheDocument()

    // 归档文件夹：组名与组内会话都不可见
    expect(screen.queryByText('已归档目录')).not.toBeInTheDocument()
    expect(screen.queryByText('归档目录里的会话')).not.toBeInTheDocument()
    // 归档组下的会话不得落进「未分组」
    const ungrouped = screen.getByText('未分组').closest('.sr-group') as HTMLElement
    expect(within(ungrouped).getByText('无归属会话')).toBeInTheDocument()
    expect(within(ungrouped).queryByText('归档目录里的会话')).not.toBeInTheDocument()
    // 空文件夹（无会话的 auto 书签组在此夹具中无会话）仍显示，组内给弱提示
    expect(within(ungrouped).queryByText('该文件夹暂无会话')).not.toBeInTheDocument()
    const autoGroup = screen.getByText('auto').closest('.sr-group') as HTMLElement
    expect(within(autoGroup).getByText('该文件夹暂无会话')).toBeInTheDocument()
  })

  it('组内按全局 collapsed_limit 折叠，点「展开其余 N 个会话」后全显', async () => {
    const many = Array.from({ length: 9 }, (_, i) =>
      item(`s${i}`, `会话${i}`, 'E:\\NUS\\1', { updated_at: 10_000 - i }),
    )
    listShelfSessions.mockImplementation(async () =>
      shelfResponse({
        items: many,
        projects: [{ path: 'E:\\NUS\\1', name: '一号', is_current: false, auto: false }],
        archived_projects: [],
        collapsed_limit: 6,
      }),
    )
    renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const group = screen.getByText('一号').closest('.sr-group') as HTMLElement
    expect(within(group).getAllByText(/^会话\d$/)).toHaveLength(6)
    const more = within(group).getByText('展开其余 3 个会话')
    fireEvent.click(more)
    await waitFor(() => expect(within(group).getAllByText(/^会话\d$/)).toHaveLength(9))
    expect(within(group).queryByText('展开其余 3 个会话')).not.toBeInTheDocument()
  })

  it('点击组内会话：先切会话（原子切 mode）再切工作目录，最后通知父级重载', async () => {
    const { onSwitchProjectDir, onSessionChanged } = renderRail()
    await waitFor(() => expect(screen.getByText('一号新会话')).toBeInTheDocument())

    fireEvent.click(screen.getByText('一号新会话'))
    await waitFor(() => expect(onSessionChanged).toHaveBeenCalledTimes(1))

    expect(switchSession).toHaveBeenCalledWith('bm1-new', 'leader')
    expect(onSwitchProjectDir).toHaveBeenCalledWith('E:\\NUS\\1')
    // 顺序：装载 → 切目录 → 通知重载（决策 4：先装载后切目录）
    expect(calls).toEqual(['switch', 'dir:E:\\NUS\\1'])
  })

  it('点击当前工作目录组内会话时不重复切目录（后端 set_project_dir 幂等，省一次 IPC）', async () => {
    listShelfSessions.mockImplementation(async () =>
      shelfResponse({
        // items 在每次拉取时重建：切换后 active 跟随（见 beforeEach 的 switchSession mock）
        items: [
          item('cur', '当前会话', 'E:\\NUS\\Nuphus', { message_count: 2 }),
          item('cur-other', '同组另一会话', 'E:\\NUS\\Nuphus', { updated_at: Date.now() - 10 }),
        ],
      }),
    )
    const { onSwitchProjectDir, onSessionChanged } = renderRail()
    await waitFor(() => expect(screen.getByText('同组另一会话')).toBeInTheDocument())

    fireEvent.click(screen.getByText('同组另一会话'))
    await waitFor(() => expect(onSessionChanged).toHaveBeenCalledTimes(1))
    expect(switchSession).toHaveBeenCalledWith('cur-other', 'leader')
    expect(onSwitchProjectDir).not.toHaveBeenCalled()
  })

  it('点击无归属会话：只切会话与 mode，不改写工作目录（禁止推断归属）', async () => {
    const { onSwitchProjectDir, onSessionChanged } = renderRail()
    await waitFor(() => expect(screen.getByText('无归属会话')).toBeInTheDocument())

    fireEvent.click(screen.getByText('无归属会话'))
    await waitFor(() => expect(onSessionChanged).toHaveBeenCalledTimes(1))

    expect(switchSession).toHaveBeenCalledWith('lonely', 'custom')
    expect(onSwitchProjectDir).not.toHaveBeenCalled()
  })

  it('组内「新建对话」：先切到该文件夹再走新建（同源 onNewChat）', async () => {
    const { onNewChat, onSwitchProjectDir } = renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const group = screen.getByText('一号').closest('.sr-group') as HTMLElement
    fireEvent.click(within(group).getByLabelText('在该文件夹新建对话'))
    await waitFor(() => expect(onNewChat).toHaveBeenCalledTimes(1))

    expect(onSwitchProjectDir).toHaveBeenCalledWith('E:\\NUS\\1')
    expect(calls).toEqual(['dir:E:\\NUS\\1', 'newChat'])
  })

  it('auto 只读组不提供重命名/归档入口，但仍可点击会话', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('auto')).toBeInTheDocument())

    const autoGroup = screen.getByText('auto').closest('.sr-group') as HTMLElement
    expect(within(autoGroup).queryByLabelText('重命名文件夹')).not.toBeInTheDocument()
    expect(within(autoGroup).queryByLabelText('归档文件夹')).not.toBeInTheDocument()
    expect(within(autoGroup).getByLabelText('在该文件夹新建对话')).toBeInTheDocument()

    const bmGroup = screen.getByText('一号').closest('.sr-group') as HTMLElement
    expect(within(bmGroup).getByLabelText('重命名文件夹')).toBeInTheDocument()
    expect(within(bmGroup).getByLabelText('归档文件夹')).toBeInTheDocument()
    // 未分组组：无路径 → 无新建/重命名/归档
    const ungrouped = screen.getByText('未分组').closest('.sr-group') as HTMLElement
    expect(within(ungrouped).queryByLabelText('在该文件夹新建对话')).not.toBeInTheDocument()
    expect(within(ungrouped).queryByLabelText('归档文件夹')).not.toBeInTheDocument()
  })

  it('组头可整组折叠/展开（默认展开）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号新会话')).toBeInTheDocument())

    const head = screen.getByText('一号').closest('.sr-group-head') as HTMLElement
    fireEvent.click(within(head).getByText('一号'))
    await waitFor(() => expect(screen.queryByText('一号新会话')).not.toBeInTheDocument())
    expect(screen.getByText('一号')).toBeInTheDocument() // 组头仍在

    fireEvent.click(screen.getByText('一号'))
    await waitFor(() => expect(screen.getByText('一号新会话')).toBeInTheDocument())
  })

  it('重命名文件夹：整表提交包含已归档书签，不丢归档记录，auto 组不写入', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const group = screen.getByText('一号').closest('.sr-group') as HTMLElement
    fireEvent.click(within(group).getByLabelText('重命名文件夹'))
    const input = within(group).getByDisplayValue('一号')
    fireEvent.change(input, { target: { value: '一号改名' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))

    expect(setProjectBookmarks).toHaveBeenCalledWith([
      { name: '一号改名', path: 'E:\\NUS\\1' },
      { name: 'Nuphus', path: 'E:\\NUS\\Nuphus' },
      { name: '已归档目录', path: 'E:\\work\\Old', archived: true },
    ])
  })

  it('归档文件夹：确认弹窗后整组隐藏（set_project_folder_archived(path, true)）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const group = screen.getByText('一号').closest('.sr-group') as HTMLElement
    fireEvent.click(within(group).getByLabelText('归档文件夹'))
    const dialog = screen.getByRole('dialog', { name: '归档该文件夹？' })
    fireEvent.click(within(dialog).getByRole('button', { name: '归档' }))
    await waitFor(() => expect(setProjectFolderArchived).toHaveBeenCalledWith('E:\\NUS\\1', true))
  })

  it('已归档文件夹：头部菜单列出并支持恢复（set_project_folder_archived(path, false)）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    fireEvent.click(screen.getByLabelText('项目文件夹'))
    expect(screen.getByText('已归档文件夹')).toBeInTheDocument()
    expect(screen.getByText('已归档目录')).toBeInTheDocument()

    fireEvent.click(screen.getByText('恢复'))
    await waitFor(() =>
      expect(setProjectFolderArchived).toHaveBeenCalledWith('E:\\work\\Old', false),
    )
  })

  it('归档列表为空时不显示恢复项，给出空态文案', async () => {
    listShelfSessions.mockImplementation(async () => shelfResponse({ archived_projects: [] }))
    renderRail()
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    fireEvent.click(screen.getByLabelText('项目文件夹'))
    expect(screen.getByText('暂无已归档文件夹')).toBeInTheDocument()
    expect(screen.queryByText('恢复')).not.toBeInTheDocument()
  })

  it('切换失败映射稳定错误码文案（busy → 业务等待提示）', async () => {
    switchSession.mockRejectedValue('busy')
    renderRail()
    await waitFor(() => expect(screen.getByText('一号新会话')).toBeInTheDocument())

    fireEvent.click(screen.getByText('一号新会话'))
    await waitFor(() =>
      expect(screen.getByText('当前会话正在执行任务，等待完成即可切换')).toBeInTheDocument(),
    )
  })
})
