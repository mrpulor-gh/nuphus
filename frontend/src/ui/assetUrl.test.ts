/**
 * resolveSkinImageUrl / releaseSkinImageUrl 契约测试。
 *
 * 背景（2026-09-26 架构调整）：皮肤背景原先走 `resolveLocalImageUrl`（Rust 读文件
 * → base64 dataURL），而 dataURL 一旦成为 CSS 值就要撞 Chromium/WebView2 对单个
 * CSS 值 2 MiB 的硬上限 —— 图片 > ~1.5 MB 必然静默不显示。本函数改为
 * asset:// 优先 + blob: 兜底，两条通道都不把字节写进 CSS 值。
 *
 * 这组用例守住三件事：
 * ① URL 原样返回（存量 dataURL / 已是 blob 的值不得被二次处理）；
 * ② asset:// 通道**必须回读校验**——dev server 下 convertFileSrc 会静默返回裸路径，
 *    只看"解出来没有"会重演 2026-09-25 那次「变量有值、背景不加载」；
 * ③ blob fallback 的 URL 可被 releaseSkinImageUrl 释放，且只释放 blob:。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (p: string) => {
    if (!/^[A-Za-z]:[\\/]/.test(p)) throw new Error(`not a filesystem path: ${p}`)
    return `asset://localhost/${encodeURIComponent(p)}`
  },
  invoke: vi.fn(async (cmd: string, args: { imagePath: string }) => {
    if (cmd !== 'read_image_base64') throw new Error(`unexpected command: ${cmd}`)
    if (/broke|missing/i.test(args.imagePath)) throw new Error('读取图片失败')
    // 'QUJD' = "ABC"；mime 刻意返回 png（后端对 gif/webp 一律如此，见 dict_ocr.rs）
    return { base64: 'QUJD', mime: 'image/png' }
  }),
}))

import {
  resolveSkinImageUrl,
  releaseSkinImageUrl,
  __setSkinImageProbe,
} from './assetUrl'
import { convertFileSrc } from '@tauri-apps/api/core'

const revoked: string[] = []
const made: string[] = []

beforeEach(() => {
  revoked.length = 0
  made.length = 0
  let n = 0
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation((u: string) => {
    revoked.push(u)
  })
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => {
    const url = `blob:http://localhost:5174/skin-${++n}`
    made.push(url)
    return url
  })
  // 默认：asset:// 可用（发布版 WebView2 的真实环境）
  __setSkinImageProbe(() => true)
})

afterEach(() => {
  __setSkinImageProbe(null)
  vi.restoreAllMocks()
})

describe('resolveSkinImageUrl —— 空值与既有 URL', () => {
  it('空值返回 null', async () => {
    expect(await resolveSkinImageUrl('')).toBeNull()
    expect(await resolveSkinImageUrl(null)).toBeNull()
    expect(await resolveSkinImageUrl(undefined)).toBeNull()
  })

  it('存量 dataURL 原样返回（升级前存 base64，不能因架构变更丢背景）', async () => {
    const dataUrl = 'data:image/png;base64,iVBORw0KGgo='
    expect(await resolveSkinImageUrl(dataUrl)).toBe(dataUrl)
  })

  it('已是 blob: 的值原样返回（不重复创建、不重复持有）', async () => {
    const blob = 'blob:http://localhost:5174/abc'
    expect(await resolveSkinImageUrl(blob)).toBe(blob)
  })

  it('http(s) 原样返回（远程壁纸不需要本地读文件）', async () => {
    const url = 'https://example.com/wall.webp'
    expect(await resolveSkinImageUrl(url)).toBe(url)
  })
})

describe('resolveSkinImageUrl —— asset:// 优先 + 回读校验', () => {
  it('asset:// 可解码 → 用它（字节不进 JS 堆与 CSS 值）', async () => {
    const url = await resolveSkinImageUrl('C:\\img\\wall.gif')
    expect(url).toBe(convertFileSrc('C:\\img\\wall.gif'))
    expect(made).toHaveLength(0) // 未走 blob 兜底
  })

  it('asset:// 回读失败（dev server 常态）→ 回落 blob:，不把裸路径交出去', async () => {
    __setSkinImageProbe(() => false)
    const url = await resolveSkinImageUrl('C:\\img\\wall.gif')
    expect(url).toBe('blob:http://localhost:5174/skin-1')
    // 关键：绝不能是那条会被浏览器当 URL 去请求的裸路径
    expect(url).not.toBe('C:\\img\\wall.gif')
  })

  it('blob 兜底的 mime 按扩展名自推：gif 得到 image/gif（否则动图变静态）', async () => {
    __setSkinImageProbe(() => false)
    const blobSpy = vi.spyOn(URL, 'createObjectURL')
    await resolveSkinImageUrl('C:\\img\\anim.gif')
    const blob = blobSpy.mock.calls[0][0] as Blob
    expect(blob.type).toBe('image/gif')
  })

  it('读文件失败 → null（调用方据此保留现状，不清空正在显示的背景）', async () => {
    __setSkinImageProbe(() => false)
    expect(await resolveSkinImageUrl('C:\\img\\broke.png')).toBeNull()
  })
})

describe('releaseSkinImageUrl —— blob 生命周期', () => {
  it('空值不动作（清除背景路径会传 null）', () => {
    releaseSkinImageUrl(null)
    releaseSkinImageUrl(undefined)
    releaseSkinImageUrl('')
    expect(revoked).toHaveLength(0)
  })

  it('释放时不做 blob: 前缀预判：旧 blob 被 data:/http(s): 替换时同样要释放', () => {
    // 预判会跳过这一条 —— 旧 blob 的字节随即活到页面关闭（真实泄漏）。
    // URL.revokeObjectURL 对非 blob URL 本就是 no-op，交给浏览器判定即可。
    releaseSkinImageUrl('data:image/png;base64,AAAA')
    releaseSkinImageUrl('blob:http://localhost:5174/a')
    expect(revoked).toEqual(['data:image/png;base64,AAAA', 'blob:http://localhost:5174/a'])
  })

  it('解析出的 blob 能被释放（不泄漏字节）', async () => {
    __setSkinImageProbe(() => false)
    const url = await resolveSkinImageUrl('C:\\img\\wall.png')
    expect(url).not.toBeNull()
    releaseSkinImageUrl(url)
    expect(revoked).toEqual([url])
  })
})
