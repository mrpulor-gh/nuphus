import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SessionRail from '../main-window/chat/SessionRail'

/**
 * 会话工作台「新建对话弹窗」（列表首位动作行 → 会话标题 + 归属项目）。
 *
 * 断言只打真实调用（api wrapper / 宿主回调）与表单规则；分组、排序、归档语义由
 * session-groups.test.ts 与 session-rail-groups.test.tsx 覆盖，这里不重复。
 *
 * ⚠️ 命令名对应：宿主把两条后端命令收在两个回调里（单一实现，不重复调 IPC）——
 *   `onSwitchProjectDir` = ChatPanel.switchProject → `set_project_dir`
 *   `onNewChat`          = useSession.handleNewChat → `new_chat_session_cmd`（带标题）
 * 因此「创建流程顺序」断言里的调用序即契约里的命令序，`set_project_dir` 失败时
 * `new_chat_session_cmd` 必须**一次都没被调用**（先建后切会落错组）。
 *
 * ⚠️ 确认**不创建会话**：`new_chat_session_cmd` 只把标题记录到后端（会话仍在欢迎页直发
 * 首条消息那一刻诞生），所以断言里不看「列表多出一条会话」，只看「标题确实交给了后端」
 * 与「失败时弹窗保持打开」。
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

/** Tauri 目录选择器（真实环境会弹出系统对话框，自动化环境只能 mock） */
const openDialog = vi.fn()
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialog(...args),
}))

/** 真实调用序（跨 api 与宿主回调） */
const calls: string[] = []

function item(id: string, title: string, project_path: string | null) {
  return {
    id,
    mode: 'leader',
    title,
    preview: '',
    message_count: 2,
    updated_at: 1_700_000_000_000,
    created_at: 1_700_000_000_000,
    is_active: false,
    project_path,
  }
}

/** 返回体夹具：两个书签组 + 一个已归档文件夹（书签整表替换时必须带回） */
function shelfResponse(overrides: Record<string, unknown> = {}) {
  return {
    can_switch: true,
    items: [item('s-a1', '一号会话', 'E:\\A'), item('s-b1', '二号会话', 'E:\\B')],
    projects: [
      { path: 'E:\\A', name: '一号', is_current: false, auto: false },
      { path: 'E:\\B', name: '二号', is_current: true, auto: false },
    ],
    archived_projects: [{ path: 'E:\\Old', name: '旧项目', is_current: false, auto: false }],
    collapsed_limit: 6,
    sort_prefs: { group_order: 'bookmark', sort_key: 'updated' },
    ...overrides,
  }
}

function renderRail(props: Partial<Parameters<typeof SessionRail>[0]> = {}) {
  const onNewChat = vi.fn(async (title?: string) => {
    calls.push(`new_chat_session_cmd:${title ?? ''}`)
    return true
  })
  const onSwitchProjectDir = vi.fn(async (path: string) => {
    calls.push(`set_project_dir:${path}`)
    return true
  })
  const utils = render(
    <SessionRail
      onSessionChanged={vi.fn()}
      onNewChat={onNewChat}
      onOpenProjectDir={vi.fn()}
      onSwitchProjectDir={onSwitchProjectDir}
      onModeSwitched={vi.fn()}
      {...props}
    />,
  )
  return { ...utils, onNewChat, onSwitchProjectDir }
}

/** 点入口动作行打开弹窗，返回 dialog */
async function openModal() {
  await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
  fireEvent.click(document.querySelector('.sr-new-chat-btn') as HTMLElement)
  return await waitFor(() => screen.getByRole('dialog', { name: '新建对话' }))
}

function createBtn(): HTMLButtonElement {
  return screen.getByRole('button', { name: '创建对话' }) as HTMLButtonElement
}

function hintText(): string {
  return document.querySelector('.nc-hint')?.textContent ?? ''
}

function option(name: RegExp): HTMLElement {
  return screen.getByRole('option', { name })
}

describe('会话工作台：新建对话弹窗（动作行 → 标题 + 归属项目）', () => {
  beforeEach(() => {
    calls.length = 0
    listShelfSessions.mockReset().mockImplementation(async () => shelfResponse())
    switchSession.mockReset().mockResolvedValue(undefined)
    renameSession.mockReset().mockResolvedValue(undefined)
    archiveSession.mockReset().mockResolvedValue(undefined)
    setProjectBookmarks.mockReset().mockImplementation(async (bookmarks: unknown[]) => bookmarks)
    setProjectFolderArchived.mockReset().mockResolvedValue([])
    setSessionSortPrefs.mockReset().mockImplementation(async (g: string, s: string) => ({
      group_order: g,
      sort_key: s,
    }))
    openDialog.mockReset()
  })

  it('入口动作行点开弹窗，标题输入框打开即聚焦', async () => {
    renderRail()
    const dialog = await openModal()
    const input = within(dialog).getByLabelText('会话标题')
    expect(input).toHaveFocus()
    expect(input).toHaveAttribute('maxlength', '40')
    // 项目常驻列表（不是下拉浮层）：两个未归档项目 + 末位「浏览本地目录…」
    expect(within(dialog).getAllByRole('option')).toHaveLength(3)
    expect(within(dialog).getByRole('option', { name: /浏览本地目录/ })).toBeInTheDocument()
  })

  it('未选项目或标题为空 → 「创建对话」disabled；两者齐备才可创建', async () => {
    renderRail()
    const dialog = await openModal()

    // ① 标题空 + 未选
    expect(createBtn()).toBeDisabled()
    // ② 只填标题、未选项目
    fireEvent.change(within(dialog).getByLabelText('会话标题'), {
      target: { value: '接口联调复盘' },
    })
    expect(createBtn()).toBeDisabled()
    // ③ 选中项目 → 可创建
    fireEvent.click(option(/一号/))
    expect(createBtn()).toBeEnabled()
    // ④ 标题清空 → 回到 disabled
    fireEvent.change(within(dialog).getByLabelText('会话标题'), { target: { value: '   ' } })
    expect(createBtn()).toBeDisabled()
  })

  it('选中项目后底部 hint 显示所选目录完整路径', async () => {
    renderRail()
    await openModal()
    expect(hintText()).toBe('')
    fireEvent.click(option(/一号/))
    expect(hintText()).toBe('E:\\A')
    fireEvent.click(option(/二号/))
    expect(hintText()).toBe('E:\\B')
  })

  it('关闭路径（取消 / Esc / 点遮罩）都能关，且重开表单复位', async () => {
    renderRail()
    let dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('会话标题'), { target: { value: '草稿' } })
    fireEvent.click(option(/一号/))
    expect(createBtn()).toBeEnabled()

    // 取消
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '新建对话' })).toBeNull())

    // 重开：标题空 / 无选中项 / 主按钮 disabled
    dialog = await openModal()
    expect(within(dialog).getByLabelText('会话标题')).toHaveValue('')
    expect(
      within(dialog)
        .getAllByRole('option')
        .every(o => o.getAttribute('aria-selected') !== 'true'),
    ).toBe(true)
    expect(hintText()).toBe('')
    expect(createBtn()).toBeDisabled()

    // Esc
    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '新建对话' })).toBeNull())

    // 点遮罩（取消带动画，关闭是异步的）
    await openModal()
    fireEvent.click(document.querySelector('.compact-overlay') as HTMLElement)
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '新建对话' })).toBeNull())
  })

  it('弹窗是独立层级：Esc 只关弹窗不收起抽屉，点弹窗内部不算点抽屉外', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
    // 展开抽屉后再开弹窗
    fireEvent.click(document.querySelector('.session-rail-chip') as HTMLElement)
    const drawer = document.querySelector('.session-rail-drawer') as HTMLElement
    expect(drawer.classList.contains('is-open')).toBe(true)

    await openModal()
    // 点弹窗内部（选项目）不收起抽屉——关窗后入口行与焦点都还在
    fireEvent.mouseDown(option(/一号/))
    expect(drawer.classList.contains('is-open')).toBe(true)

    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '新建对话' })).toBeNull())
    expect(drawer.classList.contains('is-open')).toBe(true)
  })

  it('浏览返回未知目录 → 先追加书签（带回已归档项）再按该目录归属', async () => {
    openDialog.mockResolvedValue('E:\\New')
    renderRail()
    const dialog = await openModal()
    fireEvent.click(within(dialog).getByRole('option', { name: /浏览本地目录/ }))

    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))
    // 整表替换：书签组保序 + 新目录追加 + 已归档项原样回填（否则归档记录被抹掉）
    expect(setProjectBookmarks.mock.calls[0][0]).toEqual([
      { name: '一号', path: 'E:\\A' },
      { name: '二号', path: 'E:\\B' },
      { name: 'New', path: 'E:\\New' },
      { name: '旧项目', path: 'E:\\Old', archived: true },
    ])
    // 目录选择器按目录模式调用（本环境用 Tauri dialog，不用 showDirectoryPicker）
    expect(openDialog).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true, multiple: false }),
    )
    // 新目录成为选中项：hint 显示完整路径；标题仍为空 → 主按钮仍 disabled（两段齐备才可创建）
    await waitFor(() => expect(hintText()).toBe('E:\\New'))
    expect(createBtn()).toBeDisabled()

    fireEvent.change(within(dialog).getByLabelText('会话标题'), { target: { value: '新项目会话' } })
    expect(createBtn()).toBeEnabled()
    fireEvent.click(createBtn())
    await waitFor(() =>
      expect(calls).toEqual(['set_project_dir:E:\\New', 'new_chat_session_cmd:新项目会话']),
    )
  })

  it('浏览返回已知目录 → 不写书签，直接按既有项目归属', async () => {
    openDialog.mockResolvedValue('E:\\B')
    renderRail()
    const dialog = await openModal()
    fireEvent.click(within(dialog).getByRole('option', { name: /浏览本地目录/ }))

    await waitFor(() => expect(hintText()).toBe('E:\\B'))
    expect(setProjectBookmarks).not.toHaveBeenCalled()
    expect(createBtn()).toBeDisabled() // 标题仍为空
  })

  it('浏览取消（无返回）不提示、不改选中', async () => {
    openDialog.mockResolvedValue(null)
    renderRail()
    const dialog = await openModal()
    fireEvent.click(within(dialog).getByRole('option', { name: /浏览本地目录/ }))

    await waitFor(() => expect(openDialog).toHaveBeenCalled())
    expect(hintText()).toBe('')
    expect(setProjectBookmarks).not.toHaveBeenCalled()
    expect(document.querySelector('.sr-notice')).toBeNull()
  })

  it('创建流程顺序：先 set_project_dir，再把标题交给 new_chat_session_cmd', async () => {
    renderRail()
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('会话标题'), { target: { value: '一号复盘' } })
    fireEvent.click(option(/一号/))
    const listCallsBefore = listShelfSessions.mock.calls.length

    fireEvent.click(createBtn())

    await waitFor(() =>
      expect(calls).toEqual(['set_project_dir:E:\\A', 'new_chat_session_cmd:一号复盘']),
    )
    // 确认后：弹窗关闭 + 抽屉收起 + 列表重拉（当前目录 chip / is_current 同步）
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '新建对话' })).toBeNull())
    expect(document.querySelector('.session-rail-drawer')?.classList.contains('is-open')).toBe(
      false,
    )
    expect(listShelfSessions.mock.calls.length).toBeGreaterThan(listCallsBefore)
  })

  it('set_project_dir 失败 → 中止创建，new_chat_session_cmd 一次都不调用', async () => {
    renderRail({ onSwitchProjectDir: vi.fn(async () => false) })
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('会话标题'), { target: { value: '不会创建' } })
    fireEvent.click(option(/一号/))
    fireEvent.click(createBtn())

    await waitFor(() => expect(document.querySelector('.sr-notice')).not.toBeNull())
    expect(calls).toEqual([]) // 未调用 new_chat_session_cmd（先建后切会落错组）
    expect(screen.getByText('切换项目失败，未创建对话')).toBeInTheDocument()
    // 弹窗保持打开：用户可改选项目后重试
    expect(screen.getByRole('dialog', { name: '新建对话' })).toBeInTheDocument()
  })

  it('new_chat_session_cmd 被拒（执行中）→ 标题不丢：弹窗保持打开，可直接重试', async () => {
    const onNewChat = vi.fn(async (title?: string) => {
      calls.push(`new_chat_session_cmd:${title ?? ''}`)
      return false // 后端 guard 拒绝（busy / append_pending）
    })
    renderRail({ onNewChat })
    // 先展开抽屉（失败路径要求抽屉保持展开，不能空断言一个本来就 false 的状态）
    await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
    fireEvent.click(document.querySelector('.session-rail-chip') as HTMLElement)
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('会话标题'), {
      target: { value: '接口联调复盘' },
    })
    fireEvent.click(option(/一号/))
    fireEvent.click(createBtn())

    await waitFor(() =>
      expect(calls).toEqual(['set_project_dir:E:\\A', 'new_chat_session_cmd:接口联调复盘']),
    )
    // 弹窗保持打开 + 标题仍在输入框里（用户不必重填），抽屉不收起
    const stillOpen = screen.getByRole('dialog', { name: '新建对话' })
    expect(within(stillOpen).getByLabelText('会话标题')).toHaveValue('接口联调复盘')
    expect(document.querySelector('.session-rail-drawer')?.classList.contains('is-open')).toBe(true)
  })

  it('入口动作行位于「项目」标签行之前（仍是列表首位动作）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
    const row = document.querySelector('.sr-new-chat-btn') as HTMLElement
    const label = document.querySelector('.sr-list-label') as HTMLElement
    const following = Node.DOCUMENT_POSITION_FOLLOWING
    expect(row.compareDocumentPosition(label) & following).toBeTruthy()
  })
})
