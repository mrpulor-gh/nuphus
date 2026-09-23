import { describe, expect, it } from 'vitest'
import { extractImagePaths, isImagePath } from './imagePath'

/** 取出识别到的路径原文（断言可读性优先） */
function pick(text: string): string[] {
  return extractImagePaths(text).map(range => text.slice(range.start, range.end))
}

describe('extractImagePaths', () => {
  it('命中 Windows / UNC / Unix 绝对路径（含带空格路径）', () => {
    expect(pick(String.raw`验证截图：E:\NUS\_settings_popup_wide.png`)).toEqual([
      String.raw`E:\NUS\_settings_popup_wide.png`,
    ])
    expect(pick(String.raw`共享目录 \\nas\shots\login.bmp 已同步`)).toEqual([
      String.raw`\\nas\shots\login.bmp`,
    ])
    expect(pick('见 /tmp/shots/home.PNG 完成')).toEqual(['/tmp/shots/home.PNG'])
    expect(pick(String.raw`C:\Program Files\shot\a.jpg 已保存`)).toEqual([
      String.raw`C:\Program Files\shot\a.jpg`,
    ])
  })

  it('同一行多条路径各自命中（扩展名命中处截断，不互相吞并）', () => {
    const text = String.raw`对比：E:\out\before.png 与 E:\out\after.webp`
    expect(pick(text)).toEqual([String.raw`E:\out\before.png`, String.raw`E:\out\after.webp`])
  })

  it('非图片扩展名 / 无扩展名不识别（保持纯文本）', () => {
    const texts = [
      String.raw`日志：E:\NUS\notes.txt`,
      '/tmp/report/noext',
      String.raw`备份 E:\out\shot.png.bak`,
      String.raw`校验值见 C:\out\a.md5`,
      '改动 src/mobile/api.ts 与 src/main.tsx',
    ]
    for (const text of texts) expect(pick(text)).toEqual([])
  })

  it('URL 中的图片不识别（含 URL query 里的盘符片段）', () => {
    const texts = [
      'https://example.com/a.png',
      'http://192.168.1.5:5174/static/logo.png',
      String.raw`https://relay.example.com/d/dev/file?path=E:\NUS\a.png`,
    ]
    for (const text of texts) expect(pick(text)).toEqual([])
  })

  it('markdown 链接与行内代码中的路径不误伤', () => {
    expect(pick('[截图](https://example.com/a.png)')).toEqual([])
    expect(pick('见 `E:\\NUS\\a.png` 已保存')).toEqual([])
    // 行内代码之外的同名路径仍命中（说明保护范围只限代码片段本身）
    expect(pick('`E:\\NUS\\a.png` 对应 E:\\NUS\\b.png')).toEqual(['E:\\NUS\\b.png'])
  })

  it('URL 与后续本机路径同行时只命中路径', () => {
    const text = String.raw`见https://x.com/a.png结束，产物 C:\out\c.png`
    expect(pick(text)).toEqual([String.raw`C:\out\c.png`])
  })

  it('返回区间按出现顺序且互不重叠', () => {
    const text = String.raw`E:\a\1.png/NUS 混排 E:\b\2.png`
    const ranges = extractImagePaths(text)
    expect(ranges.map(r => text.slice(r.start, r.end))).toEqual([
      String.raw`E:\a\1.png`,
      String.raw`E:\b\2.png`,
    ])
    expect(ranges[0].end).toBeLessThanOrEqual(ranges[1].start)
  })
})

describe('isImagePath', () => {
  it('整串即一条本机图片路径时命中（段落内代码片段场景）', () => {
    expect(isImagePath(String.raw`E:\NUS\_settings_popup_wide.png`)).toBe(true)
    expect(isImagePath('/tmp/shots/a.PNG')).toBe(true)
    expect(isImagePath(String.raw`  \\nas\shots\login.bmp  `)).toBe(true)
  })

  it('相对路径 / 非图片类型 / 混合文本一律不命中', () => {
    const texts = [
      '',
      'shots/a.png',
      String.raw`E:\NUS\notes.txt`,
      'https://example.com/a.png',
      String.raw`E:\NUS\a.png 和 E:\NUS\b.png`,
      String.raw`打开 E:\NUS\a.png 查看`,
    ]
    for (const text of texts) expect(isImagePath(text)).toBe(false)
  })
})
