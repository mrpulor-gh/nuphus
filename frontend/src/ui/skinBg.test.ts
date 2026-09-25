/**
 * skinBg 契约测试 —— 钉住"广播可渲染 URL"这条新链路，以及**CSS 变量已彻底退场**。
 *
 * 背景（2026-09-26 架构调整，取代 2026-09-25 的 `--app-skin-bg` 方案）：
 * 早先 `applySkinBg` 把 `url(data:...;base64,...)` 写进 `<html>` 内联样式，由
 * `.chat-area::before` / `.title-bar::before` 两个伪元素各自消费。该形态有三处
 * 结构性问题：两处 opacity 不同源（其一硬编码 0.35）、图片字节撞 CSS 值 2 MiB
 * 上限（>1.5 MB 的图静默不显示）、新区域要背景就得再加一条伪元素规则。
 *
 * 现在 `applySkinBg` 只做两件事：解析出可渲染 URL → 广播 `SKIN_CHANGE_EVENT`。
 * 渲染由唯一的 `<SkinBackdrop>` 承担（见 SkinBackdrop.test.tsx）。
 *
 * 这组用例守住：值→广播的映射、存量 dataURL 兼容、**失败不摧毁现状**、
 * 以及"任何路径都不再写 `--app-skin-bg`"。
 */

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

// jsdom 下既无 Tauri 运行时也无 asset 协议：convertFileSrc 仅对盘符路径"成功"，
// invoke('read_image_base64') 模拟 Rust 读文件。两者都由被测链路内部引用。
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

import { applySkinBg, readSkinBg, releaseSkinBg, LS_SKIN_BG, SKIN_CHANGE_EVENT } from './skinBg'
import { __setSkinImageProbe } from './assetUrl'

/** 已广播的变更序列（null = 清除） */
function recorded(): Array<string | null> {
  return broadcasts.map(e => e.url)
}

let broadcasts: Array<{ url: string | null }> = []
let onSkinChange: (e: Event) => void

beforeEach(() => {
  localStorage.clear()
  broadcasts = []
  document.documentElement.style.removeProperty('--app-skin-bg')
  // jsdom 不加载图片资源（onload 永不触发）→ asset:// 通道必失败，走 blob 兜底
  __setSkinImageProbe(() => false)
  onSkinChange = (e: Event) => {
    broadcasts.push((e as CustomEvent<{ url: string | null }>).detail)
  }
  window.addEventListener(SKIN_CHANGE_EVENT, onSkinChange)
})

afterEach(() => {
  window.removeEventListener(SKIN_CHANGE_EVENT, onSkinChange)
  __setSkinImageProbe(null)
  vi.restoreAllMocks()
})

describe('皮肤背景（广播链路）', () => {
  it('localStorage 键名固定（与 ChatPanel 的历史硬编码一致，不得漂移）', () => {
    expect(LS_SKIN_BG).toBe('nuphus_skin_bg')
  })

  it('readSkinBg：无值返回空串', () => {
    expect(readSkinBg()).toBe('')
    localStorage.setItem(LS_SKIN_BG, 'C:\\img\\a.png')
    expect(readSkinBg()).toBe('C:\\img\\a.png')
  })

  it('本地路径 → 解析为 blob URL 后广播（不再是 dataURL 拼进 CSS 值）', async () => {
    await applySkinBg('C:\\img\\wall.jpg')
    expect(broadcasts).toHaveLength(1)
    expect(broadcasts[0].url).toMatch(/^blob:/)
    expect(broadcasts[0].url).not.toContain('base64')
  })

  it('存量 dataURL 原样广播（升级前存 base64，不能因架构变更丢背景）', async () => {
    const dataUrl = 'data:image/png;base64,iVBORw0KGgo='
    await applySkinBg(dataUrl)
    expect(recorded()).toEqual([dataUrl])
  })

  it('显式空串才广播清除（清除背景的正常路径）', async () => {
    await applySkinBg('C:\\img\\a.png')
    expect(broadcasts).toHaveLength(1)

    await applySkinBg('')
    expect(broadcasts[broadcasts.length - 1].url).toBeNull()
  })

  it('解析失败时**什么都不广播**（保留正在显示的背景）', async () => {
    await applySkinBg('C:\\img\\good.png')
    expect(broadcasts).toHaveLength(1)

    // 早先「空值」与「解析失败」合成一种语义都清空，后果是：
    // 用户看着一张好背景，只因为新选的图没能渲染，眼前这张被连带清掉。
    await applySkinBg('C:\\img\\broke.png')
    expect(broadcasts).toHaveLength(1) // 没有第二条（尤其不是清除）
  })

  it('幂等：同一路径重复应用，每次都拿到描述该图的 blob（打开主题弹窗不会二次改坏）', async () => {
    await applySkinBg('C:\\img\\a.png')
    await applySkinBg('C:\\img\\a.png')
    expect(broadcasts).toHaveLength(2)
    // 两次都必须是可渲染 URL，且都不是裸路径/base64 —— 「改坏」的具体形态就是这两类
    for (const b of broadcasts) {
      expect(b.url).toMatch(/^blob:/)
      expect(b.url).not.toContain('base64')
      expect(b.url).not.toBe('C:\\img\\a.png')
    }
  })

  it('任何路径都不再触碰 CSS 变量 --app-skin-bg（2 MiB 上限问题从根上消失）', async () => {
    const setSpy = vi.spyOn(document.documentElement.style, 'setProperty')
    await applySkinBg('C:\\img\\big.jpg')
    await applySkinBg('data:image/png;base64,QUJD')
    await applySkinBg('')
    await applySkinBg('C:\\img\\broke.png')

    const touched = setSpy.mock.calls.filter(([k]) => k === '--app-skin-bg')
    expect(touched).toHaveLength(0)
    // 内联样式里也不得残留
    expect(document.documentElement.style.getPropertyValue('--app-skin-bg')).toBe('')
  })
})

describe('releaseSkinBg —— 转发到 assetUrl 的生命周期工具', () => {
  it('转交给 URL.revokeObjectURL；空值不动作', () => {
    const revoke = vi.spyOn(URL, 'revokeObjectURL')
    releaseSkinBg('blob:http://localhost:5174/a')
    releaseSkinBg(null)
    expect(revoke).toHaveBeenCalledTimes(1)
    expect(revoke).toHaveBeenCalledWith('blob:http://localhost:5174/a')
  })
})
