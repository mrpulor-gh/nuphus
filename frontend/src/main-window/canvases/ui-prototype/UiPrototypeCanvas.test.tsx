import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { UiPrototypeCanvas } from './UiPrototypeCanvas'
import { DEFAULT_THEME, type Doc } from './lib/tokens'
import { readProject } from './lib/project'
import { paletteOf } from './lib/tokens'
import { toPng } from 'html-to-image'
import { invoke } from '@tauri-apps/api/core'

vi.mock('html-to-image', () => ({
  getFontEmbedCSS: vi.fn(async () => ''),
  toPng: vi.fn(async () => 'data:image/png;base64,dGVzdA=='),
}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => 'E:/test/prototype.png'),
}))

vi.mock('../../lib/useWorkflowGate', () => ({
  useWorkflowGate: () => ({ locked: false, refresh: async () => ({ locked: false }) }),
}))
vi.mock('./lib/theme', async importOriginal => ({
  ...(await importOriginal<typeof import('./lib/theme')>()),
  ensureFontLoaded: vi.fn(),
  ensureLangFontLoaded: vi.fn(),
}))

const DOC_KEY = 'nuphus.ui_proto.doc.v2'
const savedDoc = (): Doc => JSON.parse(localStorage.getItem(DOC_KEY)!) as Doc
const toggle = () => screen.getByRole('button', { name: '切换浅色／深色' })

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  localStorage.setItem('nuphus.ui_proto.ui.v2', JSON.stringify({ lang: 'zh' }))
  document.documentElement.setAttribute('data-theme', 'tech')
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  )
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  )
})

afterEach(() => {
  cleanup()
  localStorage.clear()
  document.documentElement.removeAttribute('data-theme')
  vi.unstubAllGlobals()
})

describe('原型明暗快捷切换', () => {
  it('整页切换并保存主题，重开保持选择且不改变作品布局或全局主题', async () => {
    const { container, unmount } = render(<UiPrototypeCanvas />)
    await waitFor(() => expect(savedDoc().theme?.dark).toBe(true))
    const before = savedDoc()
    expect(toggle()).toHaveAttribute('aria-pressed', 'true')
    expect(toggle()).toHaveTextContent('深色')
    expect(container.querySelector('.app-root')).toHaveStyle({ colorScheme: 'dark' })

    fireEvent.click(toggle())
    expect(toggle()).toHaveAttribute('aria-pressed', 'false')
    expect(toggle()).toHaveTextContent('浅色')
    expect(container.querySelector('.app-root')).toHaveStyle({ colorScheme: 'light' })
    await waitFor(() => expect(savedDoc().theme?.dark).toBe(false))
    expect(savedDoc().groups).toEqual(before.groups)
    expect(savedDoc().frames).toEqual(before.frames)
    expect(document.documentElement).toHaveAttribute('data-theme', 'tech')
    expect(document.documentElement.style.colorScheme).toBe('')

    // The exported project is the same Doc; importing it must preserve the theme.
    const restored = await readProject({ text: async () => JSON.stringify(savedDoc()) } as File)
    expect(restored?.theme?.dark).toBe(false)
    unmount()
    render(<UiPrototypeCanvas />)
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-pressed', 'false'))
    fireEvent.click(toggle())
    await waitFor(() => expect(savedDoc().theme?.dark).toBe(true))
  }, 15000)

  it.each([false, true])('已有作品的 dark=%s 不被默认深色覆盖', async dark => {
    localStorage.setItem(
      DOC_KEY,
      JSON.stringify({
        groups: [],
        frames: [{ id: 'f', name: 'Test', x: 0, y: 0 }],
        paletteKey: 'brand',
        theme: { ...DEFAULT_THEME, dark },
      }),
    )
    const { container } = render(<UiPrototypeCanvas />)
    await waitFor(() => expect(toggle()).toHaveAttribute('aria-pressed', String(dark)))
    expect(container.querySelector('.app-root')).toHaveStyle({
      colorScheme: dark ? 'dark' : 'light',
    })
  })

  it('顶部按钮与已有配色面板双向同步', async () => {
    render(<UiPrototypeCanvas />)
    await waitFor(() => expect(toggle()).toBeVisible())
    fireEvent.click(screen.getByRole('button', { name: '配色' }))
    const brightness = () => screen.getByRole('button', { name: /明暗/ })
    expect(brightness()).toHaveAttribute('aria-pressed', 'true')
    fireEvent.click(toggle())
    expect(brightness()).toHaveAttribute('aria-pressed', 'false')
    fireEvent.click(brightness())
    expect(toggle()).toHaveAttribute('aria-pressed', 'true')
  })

  it('窄屏工具栏同样提供主题切换入口', async () => {
    vi.mocked(window.matchMedia).mockImplementation(
      query =>
        ({
          matches: query.includes('max-width: 840px'),
          media: query,
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
        }) as unknown as MediaQueryList,
    )
    render(<UiPrototypeCanvas />)
    await waitFor(() => expect(toggle()).toBeVisible())
    fireEvent.click(toggle())
    await waitFor(() => expect(savedDoc().theme?.dark).toBe(false))
    expect(toggle()).toHaveAttribute('aria-pressed', 'false')
  })

  it.each([false, true])('dark=%s 的预览与 PNG 导出继续使用作品主题', async dark => {
    render(<UiPrototypeCanvas />)
    await waitFor(() => expect(toggle()).toBeVisible())
    if (!dark) fireEvent.click(toggle())
    fireEvent.click(screen.getByRole('button', { name: '预览 (P)' }))
    await waitFor(() => expect(screen.getByTitle('关闭 (Esc)')).toBeVisible())
    fireEvent.click(screen.getByTitle('关闭 (Esc)'))
    fireEvent.click(screen.getByRole('button', { name: '提示词' }))
    const doc = savedDoc()
    const palette = paletteOf(doc.paletteKey, doc.customPalette, doc.theme)
    let exportBackground = ''
    vi.mocked(toPng).mockImplementationOnce(async node => {
      exportBackground = node.style.background
      return 'data:image/png;base64,dGVzdA=='
    })
    fireEvent.click(screen.getByRole('button', { name: '下载原型图' }))
    await waitFor(() => expect(toPng).toHaveBeenCalledOnce())
    const expected = document.createElement('div')
    expected.style.background = palette.surface
    expect(exportBackground).toBe(expected.style.background)
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('save_prototype_png', expect.any(Object)),
    )
    await waitFor(() => expect(toggle()).toBeVisible())
    expect(savedDoc().theme?.dark).toBe(dark)
  })
})
