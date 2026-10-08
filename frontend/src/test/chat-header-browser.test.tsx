/**
 * 聊天 header 右侧浏览器入口。
 *
 * 2026-10-08 架构切换（docs/browser-shell-arch.md §六）：主窗口的控制条已删除，
 * header 按钮从「切显隐」变成「直达窗口」——
 *   · 已有浏览窗口 → 唤出最早那扇（plugin:window|set_focus，主窗口 label 在
 *     capabilities/default.json 的 windows 列表里，调用合法）
 *   · 一个都没有 → 用起始页 browser_open 开新窗（壳页面随后聚焦地址栏等输入）
 * 输入区 dock 里不再有 .chat-browser-slot（壳+内容的 44px 控制器在独立窗口内）。
 * 壳页面自身契约由 src/browser-frame/BrowserFrame.test.tsx 覆盖。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatPanel } from '../main-window/chat/ChatPanel'
import * as api from '../main-window/lib/api'
import { ThemeProvider } from '../hooks/useTheme'

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
  emit: vi.fn(async () => undefined),
  invoke: vi.fn(async () => undefined),
}))

// jsdom 无 DOMMatrix/canvas：PDF 渲染器在 ChatPanel 子链路里被 import，stub 掉
vi.mock('pdfjs-dist', () => ({
  GlobalWorkerOptions: {},
  getDocument: vi.fn(),
  version: 'stub',
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}))

// 窗口 API：getAllWindows 决定「唤出已有 or 开新窗」；Window 类只被 setFocus 用到
const windowApi = vi.hoisted(() => ({
  existingWindows: [] as string[],
  focusMock: vi.fn(async () => undefined),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
    setFocus: vi.fn(),
  }),
  // 与 @tauri-apps/api 一致：返回 Window 实例而非 label 字符串
  getAllWindows: async () => windowApi.existingWindows.map(label => ({ label })),
  Window: class {
    label: string
    setFocus = windowApi.focusMock
    constructor(label: string) {
      this.label = label
    }
  },
}))

// jsdom 未实现 Element.scrollTo：ChatPanel 的「滚动到底」effect 会调用它
Object.defineProperty(window.Element.prototype, 'scrollTo', { value: () => {}, writable: true })

// mock 调用计数跨用例会残留（「不开新窗」断言的是本周期内的零调用），
// 每个用例前统一清空（clearAllMocks 只清调用记录，不动实现）
beforeEach(() => {
  vi.clearAllMocks()
  windowApi.existingWindows = []
})

function renderChat() {
  return render(
    <ThemeProvider>
      <ChatPanel
        messages={[]}
        executionStage="idle"
        onSend={vi.fn(async () => ({ ok: true }))}
        startupStats={{ tools: 0, memories: 0 }}
        pendingRefine={null}
        setPendingRefine={vi.fn()}
        onAppearanceDismiss={vi.fn()}
      />
    </ThemeProvider>,
  )
}

const browserEntry = () => screen.getByRole('button', { name: '应用内浏览器' })

describe('聊天 header 浏览器入口', () => {
  it('入口按钮在 .chat-header-right 内，独立类名，紧跟外观按钮', () => {
    renderChat()

    const button = browserEntry()
    expect(button).toHaveClass('chat-header-browser-btn')
    // 与设置/外观同列（同一个 .chat-header-right 父节点），列内顺序：
    // 设置 → 外观 → 浏览器
    expect(button.parentElement).toBe(document.querySelector('.chat-header-right'))
    expect(button.previousElementSibling).toHaveClass('chat-header-appearance-btn')
  })

  it('无浏览窗口时点击 → 用起始页 browser_open 开新窗', async () => {
    windowApi.existingWindows = []
    renderChat()

    fireEvent.click(browserEntry())

    await waitForBrowserOpen()
    expect(vi.mocked(api.browserOpen)).toHaveBeenCalledWith(api.BROWSER_START_URL)
    expect(windowApi.focusMock).not.toHaveBeenCalled()
  })

  it('已有浏览窗口时点击 → 唤出最早那扇，不开新窗', async () => {
    windowApi.existingWindows = ['main', 'browser-2', 'browser-1']
    renderChat()

    fireEvent.click(browserEntry())

    await waitFor(() => expect(windowApi.focusMock).toHaveBeenCalledTimes(1))
    expect(vi.mocked(api.browserOpen)).not.toHaveBeenCalled()
  })

  it('主窗口不再挂浏览器控制条（slot / 面板均已移除）', () => {
    renderChat()

    expect(document.querySelector('.chat-browser-slot')).toBeNull()
    expect(document.querySelector('.bf-root')).toBeNull()
    expect(document.querySelector('.bp-root')).toBeNull()
  })
})

/** browser_open 是异步派发的（IIFE 内 await getAllWindows），等一轮微任务 */
async function waitForBrowserOpen() {
  await waitFor(() => expect(vi.mocked(api.browserOpen)).toHaveBeenCalled())
}
