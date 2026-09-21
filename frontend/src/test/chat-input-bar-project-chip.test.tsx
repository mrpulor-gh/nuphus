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

/** ChatInputBar 必填 props 的最小夹具（projectDir 为本文件用例关心的输入；extra 覆盖 token 数据） */
function renderInputBar(
  projectDir: string,
  extra: Partial<Parameters<typeof ChatInputBar>[0]> = {},
) {
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
      {...extra}
    />,
  )
}

/** 展开 ctx 详情弹窗（hover 触发） */
function openCtxDetail() {
  const ctx = document.querySelector('.input-bar-ctx')
  if (!ctx) throw new Error('未渲染 .input-bar-ctx')
  fireEvent.mouseEnter(ctx)
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

/**
 * ctx 弹窗「执行详情」数据源契约：**整组同源**。
 *
 * 回归背景（ZPY 实测报「dispatch 时 cache 数据丢失」）：弹窗曾把 tokens/ttft/speed 取自
 * exec 源、cache 单独取自 main 源——分子是一次 exec 调用的 cache、分母是 Leader 上下文，
 * 比出来的百分比是废数（甚至整行消失）。修复后：exec 有活动整套用 exec，否则整套用 main。
 */
describe('ctx 弹窗执行详情：整组同源（禁止 exec/main 混源）', () => {
  const MAIN_USAGE = { inputTokens: 100_000, outputTokens: 0, cacheHitTokens: 200 }
  const EXEC_USAGE = {
    inputTokens: 1_000,
    outputTokens: 500,
    cacheHitTokens: 800,
    genTps: 42,
    ttftMs: 300,
  }

  it('exec 在执行：cache 与 tokens 都取 exec（cache = 800/1000 = 80%，tokens = 1.5k）', () => {
    renderInputBar(ACTIVE_DIR, {
      mainTokenUsage: MAIN_USAGE,
      execTokenUsage: EXEC_USAGE,
      contextLimit: 128_000,
      totalCalls: 7,
    })
    openCtxDetail()

    expect(screen.getByText('80%')).toBeInTheDocument() // exec 命中率，不是 200/100000
    expect(screen.getByText('1.5k')).toBeInTheDocument() // 1000 + 500
    expect(screen.getByText('300ms')).toBeInTheDocument() // ttft 同源
  })

  it('exec 无活动：整套回落到主模型数据（cache = 200/100000 = 0%）', () => {
    renderInputBar(ACTIVE_DIR, {
      mainTokenUsage: MAIN_USAGE,
      execTokenUsage: null,
      contextLimit: 128_000,
    })
    openCtxDetail()

    expect(screen.getByText('0%')).toBeInTheDocument()
  })
})
