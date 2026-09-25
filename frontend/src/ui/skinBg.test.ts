/**
 * skinBg 回归测试 —— 钉住「皮肤背景刷新/HMR 后丢失」那一类层级与渲染通道错误。
 *
 * 背景（2026-09-25 两个真实 bug）：
 * ① 恢复逻辑曾写在 ThemesPage 的挂载 effect 里，而它在 `<CompactModal open>` 内、
 *    关闭态 `return null` 不在树上 → 聊天界面刷新时无人恢复，背景必丢；
 * ② 前端跑在 Vite dev server（`http://localhost:5174`）时 `convertFileSrc` 原样
 *    返回裸路径，浏览器拿 `C:\...` 当 URL 请求 → ERR_CONNECTION_REFUSED，
 *    背景静默不显示。故渲染统一走「Rust 读文件 → dataURL」（resolveLocalImageUrl）。
 *
 * 这三组用例守住：值→CSS 变量的映射、存量 dataURL 兼容、失败不摧毁现状。
 */

import { describe, it, expect, beforeEach, vi } from 'vitest'

// jsdom 下既无 Tauri 运行时也无 asset 协议：convertFileSrc 仅对盘符路径“成功”，
// invoke('read_image_base64') 模拟 Rust 读文件。两者都由被测链路内部 dynamic import。
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

import { applySkinBg, readSkinBg, LS_SKIN_BG } from './skinBg'

const CSS_VAR = '--app-skin-bg'

function cssVar(): string {
  return document.documentElement.style.getPropertyValue(CSS_VAR).trim()
}

describe('皮肤背景（--app-skin-bg）', () => {
  beforeEach(() => {
    localStorage.clear()
    document.documentElement.style.removeProperty(CSS_VAR)
  })

  it('localStorage 键名固定（与 ChatPanel 的历史硬编码一致，不得漂移）', () => {
    expect(LS_SKIN_BG).toBe('nuphus_skin_bg')
  })

  it('readSkinBg：无值返回空串', () => {
    expect(readSkinBg()).toBe('')
    localStorage.setItem(LS_SKIN_BG, 'C:\\img\\a.png')
    expect(readSkinBg()).toBe('C:\\img\\a.png')
  })

  it('本地路径 → 经 Rust 读出后写入 CSS 变量（坏了就等于背景不显示）', async () => {
    await applySkinBg('C:\\img\\wall.jpg')
    expect(cssVar()).toBe('url(data:image/jpeg;base64,QUJD)')
  })

  it('存量 dataURL 原样生效（升级前存 base64，不能因架构变更丢背景）', async () => {
    const dataUrl = 'data:image/png;base64,iVBORw0KGgo='
    await applySkinBg(dataUrl)
    expect(cssVar()).toBe(`url(${dataUrl})`)
  })

  it('读取失败时**绝不清除**正在显示的背景', async () => {
    await applySkinBg('C:\\img\\good.png')
    const good = cssVar()
    expect(good).not.toBe('')

    // 早先「空值」与「解析失败」合成一种语义都 removeProperty，后果是：
    // 用户看着一张好背景，只因为新选的图没能渲染，眼前这张被连带清掉。
    await applySkinBg('C:\\img\\broke.png')
    expect(cssVar()).toBe(good)
  })

  it('显式空串才移除变量（清除背景的正常路径）', async () => {
    await applySkinBg('C:\\img\\a.png')
    expect(cssVar()).not.toBe('')

    await applySkinBg('')
    expect(cssVar()).toBe('')
  })

  it('幂等：同一路径重复应用结果一致（打开主题弹窗不会二次改坏）', async () => {
    await applySkinBg('C:\\img\\a.png')
    const first = cssVar()
    await applySkinBg('C:\\img\\a.png')
    expect(cssVar()).toBe(first)
  })
})
