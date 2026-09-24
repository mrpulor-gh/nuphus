import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatPanel } from '../main-window/chat/ChatPanel'
import { ThemeProvider } from '../hooks/useTheme'
import MessageList from '../mobile/components/MessageList'
import { initialChatState } from '../mobile/store'
import type { ChatMessage } from '../core/types'

vi.mock('../main-window/lib/api', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>()
  return Object.fromEntries(
    Object.entries(actual).map(([key, value]) => [
      key,
      typeof value === 'function' ? vi.fn(async () => undefined) : value,
    ]),
  )
})
vi.mock('../core/bridge', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
  invoke: vi.fn(async () => undefined),
}))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => path,
  invoke: vi.fn(async () => undefined),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onDragDropEvent: vi.fn(async () => () => {}) }),
}))
vi.mock('pdfjs-dist', () => ({ GlobalWorkerOptions: {}, getDocument: vi.fn(), version: 'stub' }))
Object.defineProperty(window.Element.prototype, 'scrollTo', { value: () => {}, writable: true })

const records: ChatMessage[] = [
  { id: 'user', role: 'user', content: '完成临时文件任务。', timestamp: 1 },
  {
    id: 'p1',
    reply_id: 'draft',
    message_id: 'p1',
    role: 'assistant',
    kind: 'progress',
    content: '先检查文件。',
    timestamp: 2,
  },
  {
    id: 'p2',
    reply_id: 'draft',
    message_id: 'p2',
    role: 'assistant',
    kind: 'progress',
    content: '正在验证结果。',
    timestamp: 3,
  },
  { id: 'draft', role: 'assistant', content: '任务已完成。', timestamp: 4, runtime: 'done' },
]
const body = '先检查文件。\n\n正在验证结果。\n\n任务已完成。'
const desktop = (messages: ChatMessage[]) => (
  <ThemeProvider>
    <ChatPanel
      messages={messages}
      executionStage="idle"
      mode="workflow"
      onSend={vi.fn(async () => ({ ok: true }) as never)}
      startupStats={{ tools: 0, memories: 0 }}
      pendingRefine={null}
      setPendingRefine={vi.fn()}
    />
  </ThemeProvider>
)

describe('single workflow reply bubble', () => {
  it('desktop renders one bubble and one set of controls, and copies the whole reply', async () => {
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    const view = render(desktop(records))
    expect(view.container.querySelectorAll('.message-bubble.assistant')).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: '复制' })).toHaveLength(1)
    expect(screen.getAllByRole('button', { name: '点评' })).toHaveLength(1)
    fireEvent.click(screen.getByRole('button', { name: '复制' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(body))
  })
  it('desktop retains the same bubble while more progress arrives and hides an empty stopped draft', () => {
    const emptyDraft = { ...records[3], content: '' }
    const view = render(desktop([records[0], records[1], emptyDraft]))
    const bubble = view.container.querySelector('.message-bubble.assistant')
    view.rerender(desktop([records[0], records[1], records[2], emptyDraft]))
    expect(view.container.querySelector('.message-bubble.assistant')).toBe(bubble)
    expect(view.container.querySelectorAll('.message-bubble.assistant')).toHaveLength(1)
    expect(bubble).toHaveTextContent('正在验证结果。')
    view.rerender(desktop([records[0], emptyDraft]))
    expect(view.container.querySelectorAll('.message-bubble.assistant')).toHaveLength(0)
  })
  it('mobile renders one reply and sends the entire reply to rating and clipboard', async () => {
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    Object.defineProperty(window, 'isSecureContext', { configurable: true, value: true })
    const rate = vi.fn()
    const view = render(
      <MessageList
        messages={records.map(({ traceItems, ...record }) => record)}
        activity={initialChatState.activity}
        onRateMessage={rate}
      />,
    )
    expect(view.container.querySelectorAll('.mobile-msg-final')).toHaveLength(1)
    expect(view.container.querySelectorAll('.mobile-msg-actions')).toHaveLength(1)
    const actions = view.container.querySelectorAll('.mobile-msg-actions button')
    fireEvent.click(actions[0])
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(body))
    fireEvent.click(actions[1])
    expect(rate).toHaveBeenCalledWith(expect.objectContaining({ content: body }))
  })
})
