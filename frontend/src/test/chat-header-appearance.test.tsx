/**
 * 聊天 header 右侧按钮列 + refine chip 移列 + 外观浮窗接线。
 *
 * 钉住三件事（jsdom 不做布局，故断言 DOM 结构与可见结果，几何交真机）：
 * ① header 右侧是**同一 DOM 纵向按钮列**：设置（齿轮）→ 外观（调色板）→
 *    浏览器（罗盘），兄弟关系而非各自 absolute（共用同一个
 *    `.chat-header-right` 父节点）；
 * ② refine chip 从 header 外移入该列后仍渲染，且 confirm 弹窗仍能打开
 *    （它的定位基准是 `.refine-pending-area`—— area 必须 position:relative，
 *     弹窗仍出现在 chip 旁）；
 * ③ 调色板按钮切换外观浮窗；外部打开请求（App 层 useModals.showThemes，
 *     即 Ctrl+K → 外观）同样能展开它，收起时回写解除请求。
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatPanel } from '../main-window/chat/ChatPanel'
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

// jsdom 未实现 Element.scrollTo：ChatPanel 的「滚动到底」effect 会调用它
Object.defineProperty(window.Element.prototype, 'scrollTo', { value: () => {}, writable: true })

/** 待处理提炼 chip 的最小形状（与 ChatPanel 的 pendingRefine 一致） */
const PENDING_REFINE = { usagePercent: 82, totalLimit: 100, skippedTurns: 3 }

function renderChat(overrides: Record<string, unknown> = {}) {
  const onAppearanceDismiss = vi.fn()
  const view = render(
    <ThemeProvider>
      <ChatPanel
        messages={[]}
        executionStage="idle"
        onSend={vi.fn(async () => ({ ok: true }))}
        startupStats={{ tools: 0, memories: 0 }}
        pendingRefine={null}
        setPendingRefine={vi.fn()}
        onAppearanceDismiss={onAppearanceDismiss}
        {...overrides}
      />
    </ThemeProvider>,
  )
  return { ...view, onAppearanceDismiss }
}

/** 聊天 header 右侧按钮列（两按钮 + refine chip 的共同父节点） */
const headerRight = () => document.querySelector('.chat-header-right') as HTMLElement

describe('聊天 header 右侧按钮列', () => {
  it('设置 / 外观 / 浏览器三按钮在同一 DOM 父子下纵向排布（兄弟节点）', () => {
    renderChat()

    const right = headerRight()
    const settings = screen.getByRole('button', { name: '控制面板' })
    const appearance = screen.getByRole('button', { name: '外观' })

    expect(settings.parentElement).toBe(right)
    expect(appearance.parentElement).toBe(right)
    // 外观按钮在设置按钮之后（列内顺序：设置 → 外观 → 浏览器）
    expect(settings.nextElementSibling).toBe(appearance)
    // 类名独立，不复用设置按钮的类
    expect(settings).toHaveClass('chat-header-settings-btn')
    expect(appearance).toHaveClass('chat-header-appearance-btn')
  })

  it('island 锚点仍在 header 内，且是按钮列的前一个兄弟', () => {
    renderChat()

    const slot = document.querySelector('.chat-header .island-slot')!
    expect(slot).toBeTruthy()
    expect(slot.nextElementSibling).toBe(headerRight())
    expect(slot.closest('.chat-header')).toBeTruthy()
  })

  it('点调色板 → 展开外观浮窗；再点 → 收起', () => {
    const view = renderChat()
    const appearance = screen.getByRole('button', { name: '外观' })

    fireEvent.click(appearance)
    expect(screen.getByRole('dialog', { name: '外观' })).toBeInTheDocument()

    fireEvent.click(appearance)
    expect(screen.queryByRole('dialog', { name: '外观' })).not.toBeInTheDocument()
    expect(view.container.querySelector('.apnp-panel')).toHaveAttribute('hidden')
  })

  it('外部打开请求（Ctrl+K → 外观）也能展开浮窗，并回写解除', () => {
    const view = renderChat({ appearanceOpen: true })

    // App 层把 useModals.showThemes 置 true → ChatPanel 同步展开
    expect(screen.getByRole('dialog', { name: '外观' })).toBeInTheDocument()

    // 收起时回写外部态：否则同值 setState 不再触发 effect，第二次入口失效
    fireEvent.mouseDown(document.body)
    expect(screen.queryByRole('dialog', { name: '外观' })).not.toBeInTheDocument()
    expect(view.onAppearanceDismiss).toHaveBeenCalledTimes(1)
  })
})

describe('refine chip 移入 header 右侧按钮列', () => {
  it('chip 是按钮列的最后一个子节点（自然下延，不再 absolute 到 header 外）', () => {
    renderChat({ pendingRefine: PENDING_REFINE })

    const chip = document.querySelector('.refine-pending-area')!
    expect(chip).toBeTruthy()
    expect(chip.parentElement).toBe(headerRight())
    // 顺序：设置 → 外观 → 浏览器 → chip。
    // 浏览器按钮唤起的是 Agent 浏览器（Nuphus 的 CDP 实例，独立 profile），
    // 不是应用内自建窗口；裸 URL 仍走系统浏览器。
    expect(headerRight().children).toHaveLength(4)
    expect(headerRight().children[3]).toBe(chip)
  })

  it('点 chip 打开 confirm 弹窗，仍挂在 chip 旁（同一父节点下）', () => {
    renderChat({ pendingRefine: PENDING_REFINE })

    fireEvent.click(screen.getByTitle('可提炼 (82%)'))

    const confirm = document.querySelector('.refine-pending-confirm')!
    expect(confirm).toBeTruthy()
    // 弹窗的定位基准仍是 chip 所在容器（不是 header、更不是 body）
    expect(confirm.parentElement).toBe(document.querySelector('.refine-pending-area'))
    expect(confirm.querySelector('.refine-confirm-actions')).toBeTruthy()
  })
})
