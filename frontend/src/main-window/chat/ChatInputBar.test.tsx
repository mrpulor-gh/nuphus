import { createRef } from 'react'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInputBar } from './ChatInputBar'

// 输入栏挂载即触发 IPC（执行闸门 / 追加队列）、Tauri 事件订阅与麦克风探测；
// 本测试只关心「底栏最左端设置入口」的结构与回调，故全部打桩。
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('../lib/api', () => ({
  listCustomAgents: vi.fn(async () => []),
  getActiveCustomAgent: vi.fn(async () => null),
  setActiveCustomAgent: vi.fn(async () => {}),
  isBusy: vi.fn(async () => false),
  getAppendQueue: vi.fn(async () => []),
  removeAppendQueueItem: vi.fn(async () => []),
  wfGateStatus: vi.fn(async () => ({ locked: false, reason: 'idle' })),
  getProjectBookmarks: vi.fn(async () => []),
}))
vi.mock('./VoiceButton', () => ({ VoiceButton: () => <div data-testid="voice-button" /> }))
vi.mock('../../ui/sound', () => ({ playUiSound: vi.fn(), playPopupSound: vi.fn() }))
vi.mock('../layout/StatusBar', () => ({ MOOD_COLORS: { idle: '#888888' } }))

type BarProps = React.ComponentProps<typeof ChatInputBar>

const baseProps: BarProps = {
  input: '',
  onInputChange: () => {},
  onInputKeyDown: () => {},
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
  contextLimit: 0,
  security: null,
  mode: 'leader',
  modelLabel: 'deepseek-chat',
  modelName: 'deepseek-chat',
  effort: null,
  supportedEfforts: [],
  onEffortChange: () => {},
  onModelSwitch: () => {},
  onSend: () => {},
  onFileSelect: () => {},
  onImageAttach: () => {},
  projectDir: '',
  onOpenProjectDir: () => {},
  hints: [],
  hintIndex: 0,
  hintFade: false,
}

const renderBar = (props: Partial<BarProps> = {}) => render(<ChatInputBar {...baseProps} {...props} />)

describe('ChatInputBar 设置中心入口', () => {
  it('位于 .input-bar-left 最左端（先于 mode chip），点击触发 onOpenSettings', () => {
    const onOpenSettings = vi.fn()
    renderBar({ onOpenSettings })

    const left = document.querySelector('.input-bar-left')
    expect(left).not.toBeNull()

    const entry = left!.firstElementChild as HTMLElement
    expect(entry).toHaveClass('input-bar-settings-btn')
    // 齿轮图标为 SVG（非 emoji）+ 无障碍名沿用 app.settings（zh 回退 = 设置）
    expect(entry.querySelector('svg')).not.toBeNull()
    expect(entry).toHaveAttribute('aria-label', '设置')
    // mode chip 紧跟在入口之后 → 证明是「最左端」而不是插在既有 chip 中间
    expect(entry.nextElementSibling).toHaveClass('input-bar-mode-wrap')

    fireEvent.click(entry)
    expect(onOpenSettings).toHaveBeenCalledTimes(1)
  })

  it('未注入 onOpenSettings 时按钮仍渲染，点击不抛错（可选 prop 兼容）', () => {
    renderBar()

    const entry = screen.getByRole('button', { name: '设置' })
    expect(() => fireEvent.click(entry)).not.toThrow()
  })
})
