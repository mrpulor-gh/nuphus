import { describe, expect, it, vi, beforeEach } from 'vitest'
import {
  ATTACHMENT_CHUNK_BYTES,
  MAX_ATTACHMENT_BYTES,
  filesToAttachments,
  isImageFile,
  isImageName,
} from '../main-window/chat/attachments'

/**
 * 附件统一通道回归测试。
 *
 * 重构背景：拖拽与附件按钮统一为 HTML5 通道（图片/文档一律进引用栏）。
 * 非图片文档浏览器拿不到绝对路径，必须经后端分块落盘换路径：
 * `begin_dropped_file` → `append_dropped_file`（8MB/块）→ 失败时 `abort_dropped_file`。
 */

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

/** jsdom 的 FileReader 可用，这里用真实 File 以走通 readAsDataURL。 */
function makeFile(name: string, type: string, content: BlobPart = 'hello'): File {
  return new File([content], name, { type })
}

beforeEach(() => {
  invokeMock.mockReset()
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'begin_dropped_file') {
      return Promise.resolve({ path: 'C:\\cache\\dropped\\a.txt', name: 'a.txt' })
    }
    if (cmd === 'append_dropped_file') return Promise.resolve({ written: 0 })
    if (cmd === 'abort_dropped_file') return Promise.resolve()
    return Promise.resolve(null)
  })
})

describe('图片判定', () => {
  it('按 MIME 与扩展名双路判定图片', () => {
    expect(isImageFile(makeFile('x.png', 'image/png'))).toBe(true)
    expect(isImageFile(makeFile('x.jpg', 'image/jpeg'))).toBe(true)
    // 无 MIME 时靠扩展名兜底（剪贴板路径场景）
    expect(isImageFile(makeFile('x.JPEG', ''))).toBe(true)
    expect(isImageFile(makeFile('x.txt', 'text/plain'))).toBe(false)
    expect(isImageFile(makeFile('noext', 'text/plain'))).toBe(false)
  })

  it('isImageName 按文件名判定', () => {
    expect(isImageName('a.PNG')).toBe(true)
    expect(isImageName('a.webp')).toBe(true)
    expect(isImageName('a.txt')).toBe(false)
  })
})

describe('filesToAttachments · 图片', () => {
  it('图片转 dataUrl 并进引用栏（不落盘、不调后端）', async () => {
    const { entries, failures } = await filesToAttachments([
      makeFile('shot.png', 'image/png', 'PNGDATA'),
    ])
    expect(failures).toEqual([])
    expect(entries).toHaveLength(1)
    const entry = entries[0]
    expect(entry.kind).toBe('image')
    if (entry.kind === 'image') {
      expect(entry.image.name).toBe('shot.png')
      expect(entry.image.dataUrl.startsWith('data:image/png;base64,')).toBe(true)
    }
    // 图片不需要后端换路径
    expect(invokeMock).not.toHaveBeenCalled()
  })
})

describe('filesToAttachments · 文档', () => {
  it('非图片经后端分块落盘换真实路径进引用栏', async () => {
    const { entries, failures } = await filesToAttachments([
      makeFile('doc.txt', 'text/plain', 'CONTENT'),
    ])
    expect(failures).toEqual([])
    const commands = invokeMock.mock.calls.map(c => c[0])
    expect(commands).toEqual(['begin_dropped_file', 'append_dropped_file'])
    // 首块 offset 必须为 0，块内是解码后的原文
    const appendArgs = invokeMock.mock.calls[1][1] as {
      fileName: string
      chunkBase64: string
      offset: number
    }
    expect(appendArgs.fileName).toBe('a.txt')
    expect(appendArgs.offset).toBe(0)
    expect(atob(appendArgs.chunkBase64)).toBe('CONTENT')

    const entry = entries[0]
    expect(entry.kind).toBe('file')
    if (entry.kind === 'file') {
      expect(entry.file.path).toBe('C:\\cache\\dropped\\a.txt')
      expect(entry.file.name).toBe('a.txt')
    }
  })

  /** 关键回归：超过单块尺寸的文件必须被切成多块顺序追加，而不是被大小上限拦下。 */
  it('大文件按 8MB 分块顺序追加（offset 递增）', async () => {
    const size = ATTACHMENT_CHUNK_BYTES + 1024 // 8MB + 1KB
    const { entries, failures } = await filesToAttachments([
      makeFile('big.bin', 'application/octet-stream', new Uint8Array(size)),
    ])
    expect(failures).toEqual([])
    expect(entries).toHaveLength(1)
    const appends = invokeMock.mock.calls
      .filter(c => c[0] === 'append_dropped_file')
      .map(c => c[1] as { offset: number })
    expect(appends.map(a => a.offset)).toEqual([0, ATTACHMENT_CHUNK_BYTES])
  })

  /** 进度回调：每块完成后回报已写入字节数，供 HUD 显示。 */
  it('分块过程回报进度且最终 loaded 等于文件大小', async () => {
    const size = ATTACHMENT_CHUNK_BYTES + 10
    const seen: number[] = []
    await filesToAttachments(
      [makeFile('p.bin', 'application/octet-stream', new Uint8Array(size))],
      p => seen.push(p.loaded),
    )
    expect(seen).toEqual([ATTACHMENT_CHUNK_BYTES, size])
  })

  /** 超过 2GB 上限时必须在前端直接拒绝，不发起 begin（避免空跑 IPC）。 */
  it('超过上限的文件直接报错且不落盘', async () => {
    const file = makeFile('huge.bin', 'application/octet-stream', 'x')
    // jsdom 无法真的造 2GB 文件：改 size 描述符模拟
    Object.defineProperty(file, 'size', { value: MAX_ATTACHMENT_BYTES + 1 })
    const { entries, failures } = await filesToAttachments([file])
    expect(entries).toHaveLength(0)
    expect(failures[0]).toContain('2GB')
    expect(invokeMock).not.toHaveBeenCalled()
  })

  /** 中途失败必须清掉半截文件，不让残缺附件留在缓存目录。 */
  it('追加失败时 abort 清理半截文件', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'begin_dropped_file') {
        return Promise.resolve({ path: 'C:\\cache\\dropped\\a.txt', name: 'a.txt' })
      }
      if (cmd === 'append_dropped_file') return Promise.reject(new Error('写入文件失败'))
      return Promise.resolve(null)
    })
    const { entries, failures } = await filesToAttachments([
      makeFile('bad.txt', 'text/plain'),
      makeFile('good.png', 'image/png'),
    ])
    // 好的图片仍在
    expect(entries).toHaveLength(1)
    expect(entries[0].kind).toBe('image')
    // 失败被记录
    expect(failures).toHaveLength(1)
    expect(failures[0]).toContain('bad.txt')
    expect(invokeMock.mock.calls.map(c => c[0])).toContain('abort_dropped_file')
  })
})
