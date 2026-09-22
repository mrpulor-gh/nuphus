/**
 * island **落点**契约 —— 按优先级解析「当前可见落点」，任何时刻都有地方渲染：
 *   ① 全屏宿主锚点（设置中心 / 模型页标题栏的 .island-slot）—— 宿主是 fixed inset:0
 *      + z-index 2500，会把聊天区整个盖住，此时聊天区锚点虽在 DOM 里却不可见
 *   ② 聊天区 `.chat-header` 中央锚点
 *   ③ 都没有 → 窗口级顶部居中回落（岛常驻 App 根部）
 *
 * 这里用**真实 ChatPanel** 渲染聊天区 header、用与宿主同构的外壳渲染宿主标题栏，
 * 断言岛经 portal 落在正确的锚点内（DOM 归属 + 父子关系），以及锚点增减时的让位顺序。
 */
import { act, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatPanel } from '../main-window/chat/ChatPanel'
import { ThemeProvider } from '../hooks/useTheme'
import { AppIsland } from '../ui/AppIsland'
import {
  getAppIslandSnapshot,
  getIslandAnchor,
  setAppFocused,
  setIslandAnchor,
  setIslandHostAnchor,
  showAppFeedback,
  useIslandHostAnchor,
} from '../ui/islandChannel'

// ChatPanel 挂载期会拉配置 / 历史 / 限额等读数：api wrapper 全量 stub 隔离副作用
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
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
    setFocus: vi.fn(),
  }),
}))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string) => p,
  invoke: vi.fn(async () => undefined),
}))

/** jsdom 未实现：ChatPanel 的滚动到底 / 语音等链路会触达 */
Object.defineProperty(window.Element.prototype, 'scrollTo', { value: () => {}, writable: true })

/** 源码原文（不经转译）：核查宿主是否真的注册了落点锚点 */
const SOURCE_FILES = import.meta.glob('/src/**/*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const chatPanelProps = {
  messages: [],
  executionStage: 'idle' as const,
  onSend: vi.fn(async () => ({ ok: true })),
  startupStats: { tools: 0, memories: 0 },
  pendingRefine: null,
  setPendingRefine: vi.fn(),
}

/**
 * 全屏宿主外壳：与 `.settings-center-host` / `.models-page-host` 的关键结构同构
 * ——「固定全屏容器 + 标题栏 + 标题栏内的 .island-slot 锚点」。
 */
function HostShell({ id, title }: { id: string; title: string }) {
  const anchor = useIslandHostAnchor(id)
  return (
    <div className="host-fixture" data-host={id}>
      <div className="settings-center-bar">
        <span className="settings-center-title">{title}</span>
        <div className="island-slot" ref={anchor} />
      </div>
    </div>
  )
}

/** 渲染「聊天区 + 常驻岛」这一真实结构（App 的结构：岛常驻根、ChatPanel 在页面深处） */
function renderShell(withChatPanel: boolean) {
  return render(
    <ThemeProvider>
      <div className="app-shell">
        {withChatPanel && (
          <div className="chat-area">
            <ChatPanel {...chatPanelProps} />
          </div>
        )}
        <AppIsland />
      </div>
    </ThemeProvider>,
  )
}

/** 推一条提示（前台 → island） */
function push(message: string, type: 'info' | 'success' | 'warning' | 'error') {
  act(() => {
    setAppFocused(true)
    showAppFeedback(message, type)
  })
}

function island(): HTMLElement {
  const el = document.querySelector<HTMLElement>('.app-island')
  if (!el) throw new Error('island 播报区未渲染')
  return el
}

/** 排空队列（一条提示要吃掉「停留 + 退场」两段时间） */
function drainIsland() {
  for (let i = 0; i < 20 && getAppIslandSnapshot().toast !== null; i++) {
    act(() => vi.advanceTimersByTime(10_000))
  }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  drainIsland()
  // 锚点是模块级共享状态：清干净，避免跨用例串味
  act(() => {
    setIslandAnchor(null)
    setIslandHostAnchor('settings-center', null)
    setIslandHostAnchor('models-page', null)
  })
  vi.useRealTimers()
})

describe('锚点解析（优先级）', () => {
  it('宿主锚点 > 聊天 header 锚点 > null（全局回落）', () => {
    const chat = document.createElement('div')
    const host = document.createElement('div')

    expect(getIslandAnchor()).toBeNull()

    act(() => setIslandAnchor(chat))
    expect(getIslandAnchor()).toBe(chat)

    // 宿主打开（压在聊天区之上）→ 让位给宿主锚点
    act(() => setIslandHostAnchor('settings-center', host))
    expect(getIslandAnchor()).toBe(host)

    // 第二个宿主打开 → 取最后打开的那个（与 App 层宿主 DOM 顺序一致）
    const models = document.createElement('div')
    act(() => setIslandHostAnchor('models-page', models))
    expect(getIslandAnchor()).toBe(models)

    // 后开的宿主关闭 → 回到仍打开的那个
    act(() => setIslandHostAnchor('models-page', null))
    expect(getIslandAnchor()).toBe(host)

    // 宿主全关 → 回聊天 header 锚点
    act(() => setIslandHostAnchor('settings-center', null))
    expect(getIslandAnchor()).toBe(chat)

    // 聊天视图也卸载 → null（岛回落窗口级定位）
    act(() => setIslandAnchor(null))
    expect(getIslandAnchor()).toBeNull()
  })

  it('宿主与聊天视图都真实挂载时：岛渲染进宿主标题栏（DOM 归属）', () => {
    render(
      <ThemeProvider>
        <div className="app-shell">
          <div className="chat-area">
            <ChatPanel {...chatPanelProps} />
          </div>
          <HostShell id="settings-center" title="设置" />
          <AppIsland />
        </div>
      </ThemeProvider>,
    )
    push('已保存', 'success')

    const host = document.querySelector('[data-host="settings-center"]')!
    const hostSlot = host.querySelector('.island-slot')!
    const chatSlot = document.querySelector('.chat-header .island-slot')!

    // 聊天锚点确实在（且被宿主压住），但岛挂在宿主锚点里 —— 这正是「让位」的证据
    expect(chatSlot).not.toBeNull()
    expect(hostSlot.contains(island())).toBe(true)
    expect(island().parentElement).toBe(hostSlot)
    expect(chatSlot.contains(island())).toBe(false)
    // 层级不靠提高 z-index，靠跟随宿主层叠
    expect(island().className).toContain('app-island--anchored')
  })

  it('宿主关闭 → 岛让回聊天 header 锚点，提示不中断', () => {
    const view = (hostOpen: boolean) => (
      <ThemeProvider>
        <div className="app-shell">
          <div className="chat-area">
            <ChatPanel {...chatPanelProps} />
          </div>
          {hostOpen && <HostShell id="models-page" title="模型" />}
          <AppIsland />
        </div>
      </ThemeProvider>
    )
    const { rerender } = render(view(true))
    push('执行完成', 'success')
    expect(document.querySelector('[data-host="models-page"]')!.contains(island())).toBe(true)

    rerender(view(false))

    expect(document.querySelector('[data-host="models-page"]')).toBeNull()
    const chatSlot = document.querySelector('.chat-header .island-slot')!
    expect(chatSlot.contains(island())).toBe(true)
    expect(document.querySelector('.app-pill-text')?.textContent).toBe('执行完成')
  })
})

describe('island 落点：聊天区 .chat-header 中间', () => {
  it('聊天锚点存在 → 岛渲染在 .chat-header 内，且锚点夹在 left / right 之间', () => {
    renderShell(true)
    push('已保存', 'success')

    const header = document.querySelector('.chat-header')
    const slot = document.querySelector('.chat-header .island-slot')
    expect(header).not.toBeNull()
    expect(slot).not.toBeNull()

    // ① DOM 父子关系：岛真的挂在 header 里（不是靠 fixed 视觉对齐）
    expect(header!.contains(island())).toBe(true)
    expect(island().parentElement).toBe(slot)
    expect(island().className).toContain('app-island--anchored')

    // ② 锚点位于「左侧留白」与「右侧设置按钮」之间 —— 中央留白位置
    expect(slot!.previousElementSibling?.className).toBe('chat-header-left')
    expect(slot!.nextElementSibling?.className).toBe('chat-header-right')

    // ③ 落在 header 内时提示照常可见（live region 内容不变）
    expect(document.querySelector('.app-pill-text')?.textContent).toBe('已保存')
  })

  it('聊天视图未挂载（启动屏 / 错误屏）→ 回落窗口级定位，提示不丢', () => {
    renderShell(false)
    expect(document.querySelector('.chat-header')).toBeNull()

    push('已中断', 'warning')

    const el = island()
    expect(el.className).not.toContain('app-island--anchored')
    // 仍挂在 App 根部（未被 portal 搬走），即改造前的窗口级定位
    expect(el.closest('.app-shell')).not.toBeNull()
    expect(document.querySelector('.app-pill-text')?.textContent).toBe('已中断')
  })

  it('聊天锚点随 ChatPanel 卸载消失 → 岛原地回落，正在展示的提示不中断', () => {
    const { rerender } = renderShell(true)
    push('已保存', 'success')
    expect(document.querySelector('.chat-header')!.contains(island())).toBe(true)

    rerender(
      <ThemeProvider>
        <div className="app-shell">
          <AppIsland />
        </div>
      </ThemeProvider>,
    )

    expect(document.querySelector('.chat-header')).toBeNull()
    expect(island().className).not.toContain('app-island--anchored')
    expect(document.querySelector('.app-pill-text')?.textContent).toBe('已保存')
  })
})

describe('宿主锚点接线（源码级：防止宿主改造后漏注册）', () => {
  it('设置中心与模型页各自注册宿主锚点，并渲染 .island-slot', () => {
    const cases: Array<[string, string]> = [
      ['src/main-window/pages/SettingsCenter.tsx', "'settings-center'"],
      ['src/main-window/App.tsx', "'models-page'"],
    ]

    for (const [path, anchorId] of cases) {
      const code = SOURCE_FILES[`/${path}`]
      expect(code, `${path} 未纳入源码扫描`).toBeTruthy()
      expect(code).toContain('useIslandHostAnchor(')
      expect(code).toContain(anchorId)
      expect(code).toContain('className="island-slot"')
    }
  })
})
