/**
 * 输入框键盘契约（ZPY 终审）：
 * - Enter（无修饰键）= 发送；
 * - Shift+Enter = 换行（原生）；
 * - Ctrl/Cmd+Enter = 换行（此前与 Enter 同为发送，现改为按光标插入换行）。
 *
 * 只断言真实可观察结果：`onSend` 是否被调用、textarea 的值与光标位置。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatPanel } from '../main-window/chat/ChatPanel'
import { ThemeProvider } from '../hooks/useTheme'

const onSend = vi.fn(async (_input: string) => ({ ok: true }) as never)

// api wrapper 全量 stub：ChatPanel 及其子组件挂载期会拉配置/会话/上下文限额等读数。
// 函数一律替换为返回 undefined 的 vi.fn；事件名等常量原样保留。
vi.mock('../main-window/lib/api', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>()
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(actual)) {
    out[key] = typeof value === 'function' ? vi.fn(async () => undefined) : value
  }
  return out
})

vi.mock('../core/bridge', () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}))

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string) => p,
  invoke: vi.fn(async () => undefined),
}))

// jsdom 无 DOMMatrix/canvas：PDF 渲染器在 ChatPanel 子链路里被 import，stub 掉
vi.mock('pdfjs-dist', () => ({
  GlobalWorkerOptions: {},
  getDocument: vi.fn(),
  version: 'stub',
}))

// ChatInputBar 直连的 Tauri 事件 / 窗口 API：jsdom 下没有 IPC（window.__TAURI_INTERNALS__ 不存在），
// 必须桩住 —— 否则 listen() 内部经 core.js 调 transformCallback 会抛未处理错误，
// vitest 会因此以非 0 退出（用例全绿但 CI 仍红）。惯例同 chat-input-bar-project-chip.test.tsx。
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
  }),
}))

// jsdom 未实现 Element.scrollTo：ChatPanel 的「滚动到底」effect 会调用它
Object.defineProperty(window.Element.prototype, 'scrollTo', { value: () => {}, writable: true })

function renderPanel() {
  render(
    <ThemeProvider>
      <ChatPanel
        messages={[]}
        executionStage="idle"
        onSend={onSend}
        startupStats={{ tools: 0, memories: 0 }}
        pendingRefine={null}
        setPendingRefine={vi.fn()}
      />
    </ThemeProvider>,
  )
  const ta = document.querySelector('.chat-input') as HTMLTextAreaElement
  expect(ta).toBeTruthy()
  return ta
}

/** 输入文本并把光标放到末尾（受控组件：走原生 setter + input 事件，贴近真实输入） */
function typeInto(ta: HTMLTextAreaElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, 'value')?.set
  setter?.call(ta, text)
  fireEvent.input(ta)
  ta.focus()
  ta.setSelectionRange(text.length, text.length)
}

describe('输入框键盘契约：Enter 发送 / Shift+Enter 换行 / Ctrl+Enter 换行', () => {
  beforeEach(() => {
    onSend.mockClear()
  })

  it('Enter 发送：调 onSend 且清空输入', () => {
    const ta = renderPanel()
    typeInto(ta, '你好')
    fireEvent.keyDown(ta, { key: 'Enter' })

    expect(onSend).toHaveBeenCalledTimes(1)
    expect(onSend.mock.calls[0][0]).toBe('你好')
    expect(ta.value).toBe('')
  })

  it('Shift+Enter 换行：不发送（换行本身是浏览器的原生默认行为，jsdom 不执行，由真机验证覆盖）', () => {
    const ta = renderPanel()
    typeInto(ta, '你好')
    fireEvent.keyDown(ta, { key: 'Enter', shiftKey: true })

    expect(onSend).not.toHaveBeenCalled()
    expect(ta.value).toBe('你好')
  })

  it('Ctrl+Enter 换行：不发送，在光标处插入换行并把光标落到换行后', async () => {
    const ta = renderPanel()
    typeInto(ta, 'abcd')
    ta.setSelectionRange(2, 2)
    fireEvent.keyDown(ta, { key: 'Enter', ctrlKey: true })

    expect(onSend).not.toHaveBeenCalled()
    expect(ta.value).toBe('ab\ncd')
    // 光标恢复在 requestAnimationFrame 里落地
    await waitFor(() => expect(ta.selectionStart).toBe(3))
  })

  it('Cmd+Enter 与 Ctrl+Enter 等价（macOS 主修饰键）', () => {
    const ta = renderPanel()
    typeInto(ta, 'abcd')
    fireEvent.keyDown(ta, { key: 'Enter', metaKey: true })

    expect(onSend).not.toHaveBeenCalled()
    expect(ta.value).toBe('abcd\n')
  })
})
