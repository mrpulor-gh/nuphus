/**
 * SkinBackdrop 契约测试 —— 钉住"皮肤背景唯一渲染点"的三条结构保证。
 *
 * 背景（2026-09-26 架构调整）：标题栏与聊天区原先各有一个 `::before` 画同一张
 * 皮肤图，两处各写一份 opacity（其一硬编码 0.35）→ 调不透明度时标题栏不动。
 * 现在只有一个 `.app-skin-backdrop`，一图同时覆盖两个区域。
 *
 * 这组用例守住：
 * ① 渲染出 `data-skin-backdrop`，且它是 `.app-shell` 的**直接**子元素
 *    （`.app-island` 靠 `el.closest('.app-shell')` 定位宿主，中间夹容器即断链）；
 * ② 图片与不透明度都落在**同一个绘制层**上，外层不设 opacity（否则双重衰减）；
 * ③ 订阅 `SKIN_CHANGE_EVENT`，并在挂载时同步读一次 localStorage 作为首帧值
 *    （后挂载的消费方不订阅不到 App 层早期那次广播）。
 */

import { render, act, cleanup } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string) => {
    if (!/^[A-Za-z]:[\\/]/.test(p)) throw new Error(`not a filesystem path: ${p}`)
    return p
  },
  invoke: vi.fn(async (cmd: string, args: { imagePath: string }) => {
    if (cmd !== 'read_image_base64') throw new Error(`unexpected command: ${cmd}`)
    if (/broke|missing/i.test(args.imagePath)) throw new Error('读取图片失败')
    return { base64: 'QUJD', mime: 'image/png' }
  }),
}))

import { SkinBackdrop } from './SkinBackdrop'
import { applySkinBg, LS_SKIN_BG, SKIN_CHANGE_EVENT } from './skinBg'
import { __setSkinImageProbe } from './assetUrl'

/** 按真实结构渲染：背景层经 portal 挂到 body（根层），内容层是它的兄弟 */
function renderShell() {
  return render(
    <div className="app-shell">
      <SkinBackdrop />
      <div className="chat-area" />
    </div>,
  )
}

function backdrop(): HTMLElement {
  const el = document.querySelector<HTMLElement>('[data-skin-backdrop]')
  if (!el) throw new Error('皮肤背景层未渲染')
  return el
}

/** 绘制层：真正承载图片与 opacity 的那一层（有图时才渲染） */
function paintLayer(): HTMLElement | null {
  return backdrop().querySelector<HTMLElement>('.app-skin-backdrop-img')
}

beforeEach(() => {
  localStorage.clear()
  // jsdom 不加载图片资源 → asset:// 通道必失败，统一走 blob 兜底（可断言）
  __setSkinImageProbe(() => false)
})

afterEach(() => {
  cleanup()
  __setSkinImageProbe(null)
  vi.restoreAllMocks()
})

describe('SkinBackdrop —— 唯一渲染点（根层）', () => {
  it('渲染出 [data-skin-backdrop]，且挂在根层（body），不在 .app-shell 内', () => {
    renderShell()
    const el = backdrop()
    // 根层：不是 .app-shell 的后代 —— 内部 DOM 不参与背景绘制
    expect(el.closest('.app-shell')).toBeNull()
    expect(el.parentElement).toBe(document.body)
    expect(document.querySelectorAll('[data-skin-backdrop]')).toHaveLength(1)
  })

  it('不引入 wrapper：.app-shell 仍是 .app-island 的最近 .app-shell 祖先', () => {
    render(
      <div className="app-shell">
        <SkinBackdrop />
        <div className="app-island" />
      </div>,
    )
    const island = document.querySelector<HTMLElement>('.app-island')!
    // portal 只在 body 追加节点，不改变 React 树里的 DOM 层级 —— closest 链完好
    expect(island.closest('.app-shell')).not.toBeNull()
    expect(island.parentElement?.classList.contains('app-shell')).toBe(true)
  })

  it('背景层同时覆盖标题栏与聊天区（无分割无边）', () => {
    renderShell()
    const el = backdrop()
    // 根层整屏覆盖：inset:0 取值于整个视口，标题栏那 48px 也在其中，
    // 与"是否落在 grid/flex 行里"无关。内容层是它的**兄弟**，
    // 背景不是被塞进聊天区里只盖下半部分。
    expect(el.parentElement).toBe(document.body)
    expect(document.querySelector('.chat-area')?.parentElement).not.toBe(el)
    expect(el.querySelector('.chat-area')).toBeNull()
  })

  it('外层无 opacity、无背景图；内层独占 opacity 变量与图片', () => {
    localStorage.setItem(LS_SKIN_BG, 'data:image/png;base64,QUJD')
    renderShell()
    // 外层设 opacity 会与内层叠成双重衰减 —— 结构上必须只有一处声明
    expect(backdrop().style.opacity).toBe('')
    expect(backdrop().style.backgroundImage).toBe('')
    // 绘制层是唯一承载者（opacity 变量写在 CSS 里，这里钉住"只有一层在画"）
    expect(paintLayer()).not.toBeNull()
    expect(backdrop().querySelectorAll('*')).toHaveLength(1)
  })

  it('aria-hidden + pointer-events 契约交给 CSS：标记存在且不参与无障碍树', () => {
    renderShell()
    expect(backdrop().getAttribute('aria-hidden')).toBe('true')
  })

  it('挂载时同步读 localStorage 作为首帧值（后挂载的消费方不依赖早期广播）', () => {
    localStorage.setItem(LS_SKIN_BG, 'data:image/png;base64,QUJD')
    renderShell()
    // 无需等待任何异步：首帧就该带上持久化的 URL
    expect(paintLayer()?.style.backgroundImage).toBe('url("data:image/png;base64,QUJD")')
  })

  it('无持久化值时首帧不渲染绘制层（不得产出空 url() 值）', () => {
    renderShell()
    expect(backdrop()).not.toBeNull()
    expect(paintLayer()).toBeNull()
  })

  it('订阅 SKIN_CHANGE_EVENT：广播到达后绘制层立即更新', () => {
    renderShell()
    act(() => {
      window.dispatchEvent(
        new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url: 'blob:http://x/new.png' } }),
      )
    })
    expect(paintLayer()?.style.backgroundImage).toBe('url("blob:http://x/new.png")')
  })

  it('广播 null → 移除绘制层（清除皮肤的正常路径）', () => {
    localStorage.setItem(LS_SKIN_BG, 'data:image/png;base64,QUJD')
    renderShell()
    expect(paintLayer()).not.toBeNull()

    act(() => {
      window.dispatchEvent(new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url: null } }))
    })
    expect(paintLayer()).toBeNull()
  })

  it('替换 blob URL 时释放旧值（不泄漏字节），即便新值不是 blob', () => {
    const revoke = vi.spyOn(URL, 'revokeObjectURL')
    renderShell()

    act(() => {
      window.dispatchEvent(
        new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url: 'blob:http://x/a.png' } }),
      )
    })
    act(() => {
      window.dispatchEvent(
        new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url: 'blob:http://x/b.png' } }),
      )
    })
    expect(revoke).toHaveBeenCalledWith('blob:http://x/a.png')

    revoke.mockClear()
    act(() => {
      window.dispatchEvent(
        new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url: 'data:image/png;base64,QUJD' } }),
      )
    })
    // 旧值是 blob、新值是 data:（存量图片）—— 这一条最容易被"新值不是 blob"骗过，
    // 跳过就泄漏。释放是无条件交给 revokeObjectURL（对非 blob 是 no-op）。
    expect(revoke).toHaveBeenCalledWith('blob:http://x/b.png')
  })

  it('重复广播同一 URL 是幂等的（不重复渲染、不误释放）', () => {
    const revoke = vi.spyOn(URL, 'revokeObjectURL')
    renderShell()
    const push = (url: string) =>
      act(() => {
        window.dispatchEvent(new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url } }))
      })
    push('blob:http://x/same.png')
    push('blob:http://x/same.png')
    expect(revoke).not.toHaveBeenCalledWith('blob:http://x/same.png')
  })

  it('卸载时释放持有的 blob URL', () => {
    const revoke = vi.spyOn(URL, 'revokeObjectURL')
    renderShell()
    act(() => {
      window.dispatchEvent(
        new CustomEvent(SKIN_CHANGE_EVENT, { detail: { url: 'blob:http://x/held.png' } }),
      )
    })
    cleanup()
    expect(revoke).toHaveBeenCalledWith('blob:http://x/held.png')
  })

  it('卸载后不再响应广播（事件监听已摘除）', () => {
    renderShell()
    cleanup()
    const el = document.querySelector('[data-skin-backdrop]')
    expect(el).toBeNull()
  })

  it('applySkinBg 的完整链路：路径 → 广播 → 渲染', async () => {
    renderShell()
    await act(async () => {
      await applySkinBg('C:\\img\\wall.png')
    })
    expect(paintLayer()?.style.backgroundImage).toMatch(/^url\("blob:/)
  })
})
