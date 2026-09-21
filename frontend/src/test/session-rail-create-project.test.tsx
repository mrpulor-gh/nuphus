import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SessionRail from '../main-window/chat/SessionRail'

/**
 * 会话工作台「创建项目」弹窗（「项目」行右端 📁+ 的唯一入口）。
 *
 * 断言范围：
 * 1. 弹窗表单规则与字段（UI 稿：项目名称 * / 最多 80 个字符 / 源文件夹 * / 取消 / 创建项目）；
 * 2. 确认后的**调用序契约**：set_project_bookmarks → set_project_dir → create_project_chat
 *    （与「创建对话」同一纪律：先切目录再生成对话，否则归属快照落到旧目录）；
 * 3. 书签整表（同路径不产生重复项、已归档项原样带回）；
 * 4. 草稿对话在 rail 的渲染（标题「新建对话」、当前高亮、无行内重命名/归档）。
 *
 * ⚠️ 分工：草稿的**生命周期**（不落库 / 切换即消失 / 冷启动不出现 / 首条消息落成）
 * 由后端测试证明（src-tauri/src/commands/process/shelf.rs 的 draft_session_* 系列）；
 * 这里只证明**前端把请求打给了谁、按什么顺序、以及怎么渲染后端返回的草稿条目**。
 */
const listShelfSessions = vi.fn()
const switchSession = vi.fn()
const renameSession = vi.fn()
const archiveSession = vi.fn()
const setProjectBookmarks = vi.fn()
const setProjectFolderArchived = vi.fn()
const setSessionSortPrefs = vi.fn()
const createProjectChat = vi.fn()

vi.mock('../main-window/lib/api', () => ({
  listShelfSessions: () => listShelfSessions(),
  switchSession: (...args: unknown[]) => switchSession(...args),
  renameSession: (...args: unknown[]) => renameSession(...args),
  archiveSession: (...args: unknown[]) => archiveSession(...args),
  setProjectBookmarks: (...args: unknown[]) => setProjectBookmarks(...args),
  setProjectFolderArchived: (...args: unknown[]) => setProjectFolderArchived(...args),
  setSessionSortPrefs: (...args: unknown[]) => setSessionSortPrefs(...args),
  createProjectChat: () => createProjectChat(),
  SESSION_GROUP_LIMIT_CHANGED_EVENT: 'nuphus:session-group-limit-changed',
}))

/** Tauri 目录选择器（真实环境会弹出系统对话框，自动化环境只能 mock） */
const openDialog = vi.fn()
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialog(...args),
}))

/** 真实调用序（跨 api wrapper 与宿主回调） */
const calls: string[] = []

function item(
  id: string,
  title: string,
  project_path: string | null,
  extra: Record<string, unknown> = {},
) {
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
    ...extra,
  }
}

/** 返回体夹具：两个书签组 + 一个已归档文件夹（书签整表替换时必须带回） */
function shelfResponse(overrides: Record<string, unknown> = {}) {
  return {
    can_switch: true,
    items: [item('s-a1', '一号会话', 'E:\\A'), item('s-b1', '二号会话', 'E:\\B')],
    projects: [
      { path: 'E:\\A', name: '一号', is_current: false, auto: false },
      { path: 'E:\\B', name: '二号', is_current: false, auto: false },
    ],
    archived_projects: [{ path: 'E:\\Old', name: '旧项目', is_current: false, auto: false }],
    collapsed_limit: 6,
    sort_prefs: { group_order: 'bookmark', sort_key: 'updated' },
    ...overrides,
  }
}

function renderRail(props: Partial<Parameters<typeof SessionRail>[0]> = {}) {
  const onSwitchProjectDir = vi.fn(async (path: string) => {
    calls.push(`set_project_dir:${path}`)
    return true
  })
  const onSessionChanged = vi.fn()
  const utils = render(
    <SessionRail
      onSessionChanged={onSessionChanged}
      onNewChat={vi.fn(async () => true)}
      onSwitchProjectDir={onSwitchProjectDir}
      onModeSwitched={vi.fn()}
      {...props}
    />,
  )
  return { ...utils, onSwitchProjectDir, onSessionChanged }
}

/** 点「项目」行右端 📁+ 打开弹窗，返回 dialog */
async function openModal() {
  await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
  fireEvent.click(screen.getByLabelText('新建项目文件夹'))
  return await waitFor(() => screen.getByRole('dialog', { name: '创建项目' }))
}

function submitBtn(): HTMLButtonElement {
  return screen.getByRole('button', { name: '创建项目' }) as HTMLButtonElement
}

function pickBtn(): HTMLButtonElement {
  return screen.getByRole('button', { name: '选择项目文件夹' }) as HTMLButtonElement
}

function pickedPathText(): string {
  return document.querySelector('.cp-picked')?.textContent ?? ''
}

/** 选目录（走真实入口按钮 → 宿主 → 系统选择器桩） */
async function pickDir(dir: string) {
  openDialog.mockResolvedValueOnce(dir)
  fireEvent.click(pickBtn())
  await waitFor(() => expect(pickedPathText()).toBe(dir))
}

/** 某个项目分组容器（按组名定位） */
function groupEl(name: string): HTMLElement {
  const head = screen.getByText(name)
  return head.closest('.sr-group') as HTMLElement
}

describe('会话工作台：创建项目弹窗（📁+ → 名称 + 源文件夹 → 书签 + 空对话）', () => {
  beforeEach(() => {
    calls.length = 0
    listShelfSessions.mockReset().mockImplementation(async () => shelfResponse())
    switchSession.mockReset().mockResolvedValue(undefined)
    renameSession.mockReset().mockResolvedValue(undefined)
    archiveSession.mockReset().mockResolvedValue(undefined)
    setProjectBookmarks.mockReset().mockImplementation(async (bookmarks: unknown[]) => {
      calls.push('set_project_bookmarks')
      return bookmarks
    })
    setProjectFolderArchived.mockReset().mockResolvedValue([])
    setSessionSortPrefs.mockReset().mockImplementation(async (g: string, s: string) => ({
      group_order: g,
      sort_key: s,
    }))
    createProjectChat.mockReset().mockImplementation(async () => {
      calls.push('create_project_chat')
      return { id: 'draft-1', mode: 'leader', project_path: 'E:\\New' }
    })
    openDialog.mockReset()
  })

  it('📁+ 打开「创建项目」弹窗：标题 / 两个必填字段 / 名称上限 80 与 helper / 底部两按钮', async () => {
    renderRail()
    const dialog = await openModal()

    expect(within(dialog).getByLabelText('项目名称')).toHaveAttribute('maxlength', '80')
    expect(within(dialog).getByText('最多 80 个字符')).toBeInTheDocument()
    expect(within(dialog).getByText('源文件夹')).toBeInTheDocument()
    expect(pickBtn()).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '取消' })).toBeInTheDocument()
    expect(submitBtn().textContent).toBe('创建项目')
  })

  it('未填名称或未选目录 → 「创建项目」禁用；两者齐备才可提交', async () => {
    renderRail()
    const dialog = await openModal()

    // ① 都空
    expect(submitBtn()).toBeDisabled()
    // ② 只填名称，未选目录 → 仍禁用
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '新项目' } })
    expect(submitBtn()).toBeDisabled()
    // ③ 名称清空 + 已选目录（下面选）→ 仍禁用
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '   ' } })
    await pickDir('E:\\New')
    expect(submitBtn()).toBeDisabled()
    // ④ 名称 + 目录齐备 → 可提交
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '新项目' } })
    expect(submitBtn()).toBeEnabled()
  })

  it('「选择项目文件夹」调系统目录选择器，并在按钮下方显示所选路径', async () => {
    renderRail()
    await openModal()
    expect(pickedPathText()).toBe('')

    await pickDir('E:\\Work\\New')

    expect(openDialog).toHaveBeenCalledTimes(1)
    expect(openDialog.mock.calls[0][0]).toMatchObject({ directory: true, multiple: false })
    expect(pickedPathText()).toBe('E:\\Work\\New')
  })

  it('确认创建：书签 → 切当前目录 → 生成空对话（按序），随后关弹窗、收抽屉、刷新列表、重拉聊天区', async () => {
    const { onSessionChanged } = renderRail()
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '新项目' } })
    await pickDir('E:\\New')

    const listCallsBefore = listShelfSessions.mock.calls.length
    fireEvent.click(submitBtn())

    // 顺序契约：书签 → 切目录 → 生成草稿（先建后切会让归属快照落到旧目录）
    await waitFor(() =>
      expect(calls).toEqual([
        'set_project_bookmarks',
        'set_project_dir:E:\\New',
        'create_project_chat',
      ]),
    )
    expect(createProjectChat).toHaveBeenCalledTimes(1)

    await waitFor(() => expect(screen.queryByRole('dialog', { name: '创建项目' })).toBeNull())
    expect(document.querySelector('.session-rail-drawer')?.classList.contains('is-open')).toBe(
      false,
    )
    expect(listShelfSessions.mock.calls.length).toBeGreaterThan(listCallsBefore)
    expect(onSessionChanged).toHaveBeenCalled()
  })

  it('书签整表：新项目追加在未归档书签之后、已归档项原样带回、同路径不产生重复项', async () => {
    renderRail()
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '新项目' } })
    await pickDir('E:\\New')
    fireEvent.click(submitBtn())
    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))

    // 追加路径：一号 / 二号（原序）→ 新项目 → 旧项目（归档标记带回）
    expect(setProjectBookmarks.mock.calls[0][0]).toEqual([
      { name: '一号', path: 'E:\\A' },
      { name: '二号', path: 'E:\\B' },
      { name: '新项目', path: 'E:\\New' },
      { name: '旧项目', path: 'E:\\Old', archived: true },
    ])

    // 同路径已存在（E:\A）→ 只更新名称，不新增重复项（表长仍为 3）
    await openModal()
    fireEvent.change(screen.getByLabelText('项目名称'), { target: { value: '一号改名' } })
    await pickDir('E:\\A')
    fireEvent.click(submitBtn())
    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(2))
    const table = setProjectBookmarks.mock.calls[1][0] as { path: string; name: string }[]
    expect(table.filter(b => b.path === 'E:\\A')).toHaveLength(1)
    expect(table.find(b => b.path === 'E:\\A')?.name).toBe('一号改名')
    expect(table).toHaveLength(3)
  })

  it('切目录失败 → 中止：create_project_chat 一次都不调用，弹窗保持打开', async () => {
    renderRail({ onSwitchProjectDir: vi.fn(async () => false) })
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '不会创建' } })
    await pickDir('E:\\New')

    fireEvent.click(submitBtn())

    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))
    expect(createProjectChat).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog', { name: '创建项目' })).toBeInTheDocument()
    expect(screen.getByText('切换项目失败，未创建对话')).toBeInTheDocument()
  })

  it('生成空对话失败（后端拒绝）→ 弹窗保持打开，用户可重试', async () => {
    createProjectChat.mockRejectedValueOnce('busy')
    renderRail()
    const dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '新项目' } })
    await pickDir('E:\\New')

    fireEvent.click(submitBtn())

    await waitFor(() => expect(createProjectChat).toHaveBeenCalledTimes(1))
    expect(screen.getByRole('dialog', { name: '创建项目' })).toBeInTheDocument()
    // 后端 busy 文案（与切换失败共用映射）
    expect(screen.getByText('当前会话正在执行任务，等待完成即可切换')).toBeInTheDocument()
  })

  it('关闭路径（取消 / Esc / 点遮罩）都能关，且重开表单复位', async () => {
    renderRail()
    let dialog = await openModal()
    fireEvent.change(within(dialog).getByLabelText('项目名称'), { target: { value: '草稿名' } })
    await pickDir('E:\\New')
    expect(submitBtn()).toBeEnabled()

    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '创建项目' })).toBeNull())

    // 重开：名称空 / 未选目录 / 主按钮 disabled
    dialog = await openModal()
    expect(within(dialog).getByLabelText('项目名称')).toHaveValue('')
    expect(pickedPathText()).toBe('')
    expect(submitBtn()).toBeDisabled()

    // Esc
    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '创建项目' })).toBeNull())

    // 点遮罩（关闭带动画，关闭是异步的）
    await openModal()
    fireEvent.click(document.querySelector('.compact-overlay') as HTMLElement)
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '创建项目' })).toBeNull())
  })

  it('Esc 只关弹窗不收起抽屉（弹窗与抽屉同层）', async () => {
    renderRail()
    await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
    fireEvent.click(document.querySelector('.session-rail-chip') as HTMLElement)
    const drawer = document.querySelector('.session-rail-drawer') as HTMLElement
    expect(drawer.classList.contains('is-open')).toBe(true)

    await openModal()
    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '创建项目' })).toBeNull())
    expect(drawer.classList.contains('is-open')).toBe(true)
  })

  it('草稿对话在 rail：落在所属文件夹组内、带着色高亮与「当前」、标题「新建对话」、无行内重命名/归档', async () => {
    listShelfSessions.mockImplementation(async () =>
      shelfResponse({
        items: [
          item('draft-1', '', 'E:\\New', { is_active: true, draft: true, message_count: 0 }),
          item('s-a1', '一号会话', 'E:\\A'),
        ],
        projects: [
          { path: 'E:\\A', name: '一号', is_current: false, auto: false },
          { path: 'E:\\New', name: '新项目', is_current: true, auto: false },
        ],
      }),
    )
    renderRail()

    await waitFor(() => expect(screen.getByText('一号会话')).toBeInTheDocument())
    const newGroup = groupEl('新项目')
    // 归属正确：该组的分组键就是草稿的归属路径（不是「未分组」兜底组）
    expect(newGroup.querySelector('.sr-group-toggle')?.getAttribute('title')).toBe('E:\\New')
    const heads = Array.from(document.querySelectorAll('.sr-group-name')).map(e => e.textContent)
    expect(heads).not.toContain('未分组')

    const draftRow = newGroup.querySelector('.sr-item') as HTMLElement
    expect(draftRow).not.toBeNull()
    expect(draftRow.classList.contains('active')).toBe(true)
    expect(draftRow.querySelector('.sr-title-btn')?.textContent).toBe('新建对话')
    expect(draftRow.querySelector('.sr-current-badge')?.textContent).toBe('当前')
    // 草稿无行内操作：重命名会写 sessions 行（违反「空对话不落库」）
    expect(draftRow.querySelector('.sr-actions')).toBeNull()
  })
})
