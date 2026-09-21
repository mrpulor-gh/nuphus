import { createRef } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInputBar } from '../main-window/chat/ChatInputBar'

/**
 * mode **hover 弹窗的 title** = 纯展示当前对话归属的项目文件夹（原「输入框右下角项目 chip」迁入）。
 *
 * 回归背景：项目 chip 曾是可点击入口（有可见书签 → 弹快捷切换菜单；无可见书签 → 直接进项目中心），
 * ZPY 实测后定调「我只要显示当前对话是在哪个项目文件目录，不需要点击管理」→ 收敛为纯展示元素。
 * 项目目录如今主要由**会话工作台**确定，输入框里的目录只剩回显价值，故从右下角操作组
 * 迁到 mode chip 的 hover 弹窗顶部作为其 title（chip 本身仍是单行「图标 + 模式名」）。
 *
 * 覆盖：
 * 1. title 随弹窗开关（未展开不渲染）；
 * 2. 展开后显示工作目录末段名 + 完整路径 title 属性，且 chip 上的模式名不受影响；
 * 3. title 自身无交互语义（SPAN / 无 role / 不可聚焦）；
 * 4. 未设置目录 → 回退 `input.projectDir` 文案（弱化态，无 is-set）。
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
function makeProps(
  projectDir: string,
  extra: Partial<Parameters<typeof ChatInputBar>[0]> = {},
): Parameters<typeof ChatInputBar>[0] {
  return {
    input: '',
    onInputChange: vi.fn(),
    onInputKeyDown: vi.fn(),
    textareaRef: createRef<HTMLTextAreaElement>(),
    imageInputRef: createRef<HTMLInputElement>(),
    isProcessing: false,
    pauseState: null,
    refineState: null,
    tokenUsage: null,
    mainTokenUsage: null,
    execTokenUsage: null,
    totalDurationMs: undefined,
    totalCalls: undefined,
    mood: 'idle',
    contextLimit: undefined,
    security: null,
    mode: 'leader',
    modelLabel: 'DeepSeek',
    modelName: 'deepseek-chat',
    effort: null,
    supportedEfforts: [],
    onEffortChange: vi.fn(),
    onModelSwitch: vi.fn(),
    onSend: vi.fn(),
    onFileSelect: vi.fn(),
    onImageAttach: vi.fn(),
    projectDir,
    hints: ['输入框提示'],
    hintIndex: 0,
    hintFade: false,
    ...extra,
  }
}

/** 渲染输入框（返回 render 结果，供 rerender 模拟「切换会话后父组件换 projectDir」） */
function renderInputBar(
  projectDir: string,
  extra: Partial<Parameters<typeof ChatInputBar>[0]> = {},
) {
  return render(<ChatInputBar {...makeProps(projectDir, extra)} />)
}

/** 展开 ctx 详情弹窗（hover 触发） */
function openCtxDetail() {
  const ctx = document.querySelector('.input-bar-ctx')
  if (!ctx) throw new Error('未渲染 .input-bar-ctx')
  fireEvent.mouseEnter(ctx)
}

/** 展开 mode hover 弹窗（点击 chip 切换 open；title 只在弹窗内渲染） */
function openModeMenu() {
  const chip = document.querySelector('.input-bar-mode-wrap .input-bar-chip')
  if (!chip) throw new Error('未渲染 .input-bar-mode-wrap .input-bar-chip')
  fireEvent.click(chip)
}

/** mode 弹窗顶部的 title（弹窗打开后文档内唯一持有该 class 的元素） */
function modeTitle(): HTMLElement {
  const el = document.querySelector('.input-bar-mode-title')
  if (!el) throw new Error('未渲染 .input-bar-mode-title')
  return el as HTMLElement
}

describe('mode 弹窗 title：纯展示当前项目文件夹', () => {
  it('弹窗未展开时不渲染 title', () => {
    renderInputBar(ACTIVE_DIR)

    expect(document.querySelector('.input-bar-mode-title')).toBeNull()
  })

  it('展开后显示工作目录末段名 + 完整路径 tooltip，chip 上的模式名不受影响', () => {
    renderInputBar(ACTIVE_DIR)
    openModeMenu()

    expect(modeTitle()).toHaveTextContent('A')
    expect(modeTitle()).toHaveAttribute('title', ACTIVE_DIR)
    expect(modeTitle()).toHaveClass('is-set')
    // title 在弹窗里、不在 chip 里：chip 仍是单行「图标 + 模式名」
    expect(document.querySelector('.input-bar-mode-text')?.textContent).toBe('LEADER')
  })

  it('title 自身无交互语义（SPAN / 无 role / 不可聚焦）', () => {
    renderInputBar(ACTIVE_DIR)
    openModeMenu()

    expect(modeTitle().tagName).toBe('SPAN')
    expect(modeTitle()).not.toHaveAttribute('role')
    expect(modeTitle()).not.toHaveAttribute('tabindex')
  })

  it('未设置目录 → 回退 input.projectDir 文案（弱化态）', () => {
    renderInputBar('')
    openModeMenu()

    expect(modeTitle()).toHaveTextContent('项目目录')
    expect(modeTitle()).toHaveAttribute('title', '项目目录')
    expect(modeTitle()).not.toHaveClass('is-set')
  })

  it('切换会话（父组件换 projectDir）后 title 与 tooltip 跟随更新', () => {
    const utils = renderInputBar(ACTIVE_DIR)
    openModeMenu()
    expect(modeTitle()).toHaveTextContent('A')

    // 模拟点击另一项目分组的会话：ChatPanel.switchProject 落盘（set_project_dir）后
    // setProjectDir(新路径) → 本组件作为受控组件重新收到 projectDir
    utils.rerender(<ChatInputBar {...makeProps('E:\\NUS\\B')} />)

    expect(modeTitle()).toHaveTextContent('B')
    expect(modeTitle()).toHaveAttribute('title', 'E:\\NUS\\B')
  })

  it('切换后目录变空 → 撤销 is-set 并回退文案（不残留上一个项目的名字）', () => {
    const utils = renderInputBar(ACTIVE_DIR)
    openModeMenu()
    expect(modeTitle()).toHaveClass('is-set')

    utils.rerender(<ChatInputBar {...makeProps('')} />)

    expect(modeTitle()).toHaveTextContent('项目目录')
    expect(modeTitle()).not.toHaveClass('is-set')
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
