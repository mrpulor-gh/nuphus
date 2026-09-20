import { createRef } from 'react'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInputBar } from '../main-window/chat/ChatInputBar'

/**
 * 输入框项目 chip 下拉：「已归档」书签必须与项目中心 / 会话工作台同语义 —— 归档 = 隐藏。
 *
 * 回归背景：chip 菜单曾直接渲染**全量**书签，ZPY 实测点开看到已归档的 `Old`
 * （会话工作台已把归档组整组隐藏，chip 却能一键切进去 → 语义自相矛盾）。
 *
 * 覆盖：
 * 1. 未归档书签照常列出（「当前」徽标 / 点击切换 / 分隔线 +「管理项目…」行为不变）；
 * 2. `archived === true` 的项不出现在菜单里；
 * 3. 可见书签为空（全部归档）→ 点 chip 走既有 onOpenProjectDir()，不开空菜单。
 *
 * 数据源契约：过滤写在**消费端**（ChatInputBar）。ChatPanel 的 state 与项目中心
 * 仍持有全量书签表（归档项要在「已归档文件夹」区恢复，不能被上游过滤掉）。
 */
vi.mock('../main-window/lib/api', () => ({
  wfGateStatus: vi.fn(() => Promise.resolve({ locked: false, reason: 'idle' })),
  isBusy: vi.fn(() => Promise.resolve(false)),
  getAppendQueue: vi.fn(() => Promise.resolve([])),
  removeAppendQueueItem: vi.fn(() => Promise.resolve([])),
  listCustomAgents: vi.fn(() => Promise.resolve([])),
  getActiveCustomAgent: vi.fn(() => Promise.resolve(null)),
  setActiveCustomAgent: vi.fn(() => Promise.resolve(null)),
  // 以下为 VoiceButton / SecurityPrompt 的 api 依赖：本用例不触达，桩住即可
  sttStatus: vi.fn(() => Promise.resolve(null)),
  sttStart: vi.fn(() => Promise.resolve(null)),
  sttStop: vi.fn(() => Promise.resolve(null)),
  sttCancel: vi.fn(() => Promise.resolve(null)),
  sttDownloadModel: vi.fn(() => Promise.resolve(null)),
  getCapabilities: vi.fn(() => Promise.resolve(null)),
  setCapability: vi.fn(() => Promise.resolve(null)),
  approveOnceSecurity: vi.fn(),
  approveSessionSecurity: vi.fn(),
  rejectSecurity: vi.fn(),
}))

// ChatInputBar 直连的 Tauri 事件 / 窗口 API：jsdom 下无 IPC，桩住（不改组件）
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
  }),
}))

/** 当前工作目录（chip 的 title = 该路径，用于定位 chip 按钮） */
const ACTIVE_DIR = 'E:\\NUS\\A'

type Bookmark = { name: string; path: string; archived?: boolean }

/** 后端书签形状（archived 缺省 = 未归档），与 ProjectPage.test 同口径 */
function bookmark(name: string, path: string, archived = false): Bookmark {
  return archived ? { name, path, archived: true } : { name, path }
}

/** ChatInputBar 必填 props 的最小夹具（可选 props 走组件默认值） */
function renderInputBar(bookmarks: Bookmark[]) {
  const onOpenProjectDir = vi.fn()
  const onSwitchProject = vi.fn()
  render(
    <ChatInputBar
      input=""
      onInputChange={vi.fn()}
      onInputKeyDown={vi.fn()}
      textareaRef={createRef<HTMLTextAreaElement>()}
      imageInputRef={createRef<HTMLInputElement>()}
      isProcessing={false}
      pauseState={null}
      refineState={null}
      tokenUsage={null}
      mainTokenUsage={null}
      execTokenUsage={null}
      totalDurationMs={undefined}
      totalCalls={undefined}
      mood="idle"
      contextLimit={undefined}
      security={null}
      mode="leader"
      modelLabel="DeepSeek"
      modelName="deepseek-chat"
      effort={null}
      supportedEfforts={[]}
      onEffortChange={vi.fn()}
      onModelSwitch={vi.fn()}
      onSend={vi.fn()}
      onFileSelect={vi.fn()}
      onImageAttach={vi.fn()}
      projectDir={ACTIVE_DIR}
      onOpenProjectDir={onOpenProjectDir}
      projectBookmarks={bookmarks}
      onSwitchProject={onSwitchProject}
      hints={['输入框提示']}
      hintIndex={0}
      hintFade={false}
    />,
  )
  return { onOpenProjectDir, onSwitchProject }
}

/** chip 按钮：title = 当前项目目录绝对路径（菜单项 title 是各自路径，不会撞） */
function chip(): HTMLElement {
  return screen.getByTitle(ACTIVE_DIR)
}

describe('输入框项目 chip 下拉：归档项 = 隐藏', () => {
  it('已归档书签不出现在 chip 下拉菜单里', async () => {
    renderInputBar([
      bookmark('当前目录', ACTIVE_DIR),
      bookmark('二号目录', 'E:\\NUS\\B'),
      bookmark('已归档目录', 'E:\\work\\Old', true),
    ])
    fireEvent.click(chip())

    const menu = await screen.findByRole('menu')
    expect(within(menu).getByText('当前目录')).toBeInTheDocument()
    expect(within(menu).getByText('二号目录')).toBeInTheDocument()
    expect(within(menu).queryByText('已归档目录')).not.toBeInTheDocument()
    // 「当前」徽标仍只贴在路径与当前工作目录一致的项上
    const currentItem = within(menu).getByText('当前目录').closest('button')!
    expect(within(currentItem).getByText('当前')).toBeInTheDocument()
    expect(within(menu).getByText('二号目录').closest('button')).not.toHaveTextContent('当前')
    // 分隔线与「管理项目…」保持原样（未被归档过滤误伤）
    expect(menu.querySelector('.input-tool-menu-divider')).not.toBeNull()
    expect(within(menu).getByText('管理项目…')).toBeInTheDocument()
  })

  it('全部书签都已归档 → 点 chip 进项目中心，不打开空菜单', async () => {
    const { onOpenProjectDir } = renderInputBar([
      bookmark('已归档甲', 'E:\\work\\Old', true),
      bookmark('已归档乙', 'E:\\work\\Ancient', true),
    ])
    fireEvent.click(chip())

    await waitFor(() => expect(onOpenProjectDir).toHaveBeenCalledTimes(1))
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(screen.queryByText('管理项目…')).not.toBeInTheDocument()
    expect(screen.queryByText('已归档甲')).not.toBeInTheDocument()
  })

  it('点击未归档书签仍执行 onSwitchProject 并收起菜单', async () => {
    const { onSwitchProject } = renderInputBar([
      bookmark('二号目录', 'E:\\NUS\\B'),
      bookmark('已归档目录', 'E:\\work\\Old', true),
    ])
    fireEvent.click(chip())

    const menu = await screen.findByRole('menu')
    fireEvent.click(within(menu).getByText('二号目录'))

    expect(onSwitchProject).toHaveBeenCalledWith('E:\\NUS\\B')
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })
})
