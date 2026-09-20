import { createRef } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInputBar } from '../main-window/chat/ChatInputBar'

/**
 * 输入框项目 chip = **纯展示**当前对话归属的项目文件夹。
 *
 * 回归背景：chip 曾是可点击入口（有可见书签 → 弹快捷切换菜单；无可见书签 → 直接进项目中心），
 * ZPY 实测后定调「我只要显示当前对话是在哪个项目文件目录，不需要点击管理」→ chip 收敛为
 * 非交互元素：不弹菜单、不进管理；ChatInputBar 不再持有任何项目入口
 * （项目中心的入口只剩会话栏「项目」行 📁+，见 session-rail-menu.test.tsx）。
 *
 * 覆盖：
 * 1. chip 存在，显示工作目录末段名 + 完整路径 title；
 * 2. chip 非交互：不是 button / 无 role / 不可聚焦，点击后文档中不出现 role="menu"；
 * 3. 未设置目录 → 回退 `input.projectDir` 文案（弱化态，无 is-set）。
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

/** 当前工作目录（chip 的 title = 该路径；末段名 = "A"） */
const ACTIVE_DIR = 'E:\\NUS\\A'

/** ChatInputBar 必填 props 的最小夹具（projectDir 为唯一本用例关心的输入） */
function renderInputBar(projectDir: string) {
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
      projectDir={projectDir}
      hints={['输入框提示']}
      hintIndex={0}
      hintFade={false}
    />,
  )
}

/** 项目 chip（输入框内唯一持有该 class 的元素） */
function chip(): HTMLElement {
  const el = document.querySelector('.input-project-chip')
  if (!el) throw new Error('未渲染 .input-project-chip')
  return el as HTMLElement
}

describe('输入框项目 chip：纯展示当前项目文件夹', () => {
  it('显示工作目录末段名 + 完整路径 tooltip', () => {
    renderInputBar(ACTIVE_DIR)

    expect(chip()).toHaveTextContent('A')
    expect(chip()).toHaveAttribute('title', ACTIVE_DIR)
    expect(chip()).toHaveClass('is-set')
  })

  it('非交互：不是按钮、无 role，点击不弹菜单', () => {
    renderInputBar(ACTIVE_DIR)

    expect(chip().tagName).toBe('SPAN')
    expect(chip().closest('button')).toBeNull()
    expect(chip()).not.toHaveAttribute('role')
    // 无 tabindex → 键盘 Tab 也聚焦不到该元素
    expect(chip()).not.toHaveAttribute('tabindex')

    fireEvent.click(chip())

    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(screen.queryAllByRole('menuitem')).toHaveLength(0)
    expect(screen.queryByText('管理项目…')).not.toBeInTheDocument()
  })

  it('未设置目录 → 回退 input.projectDir 文案（弱化态）', () => {
    renderInputBar('')

    expect(chip()).toHaveTextContent('项目目录')
    expect(chip()).toHaveAttribute('title', '项目目录')
    expect(chip()).not.toHaveClass('is-set')
  })
})
