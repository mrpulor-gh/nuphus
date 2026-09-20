import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FilePreviewContent } from '../main-window/chat/PreviewOverlay'
import { convertFileSrc } from '@tauri-apps/api/core'
import { openPath, readFile, readFileBase64 } from '../main-window/lib/api'

// 回归背景（2026-09-20）：html/htm 曾被「非文本类型 → 系统默认程序打开」的早退分支
// 截走——从 preview:// 沙箱底座上线起，本文件末尾的 iframe 分支就不可达，点 HTML 交付物
// 只会看到「已请求系统默认程序打开」占位。这两条测试分别钉住「不再截走」与「不回归兜底」。

vi.mock('../main-window/lib/api', () => ({
  readFile: vi.fn(() => Promise.resolve('')),
  readFileBase64: vi.fn(() => Promise.resolve('')),
  openPath: vi.fn(() => Promise.resolve()),
  revealPath: vi.fn(() => Promise.resolve()),
}))

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: vi.fn((p: string, protocol = 'asset') => `http://${protocol}.localhost/${p}`),
}))

vi.mock('pdfjs-dist', () => ({ GlobalWorkerOptions: {}, getDocument: vi.fn() }))

describe('FilePreviewContent 类型分流', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it.each([
    ['html', String.raw`C:\Users\me\Nuphus\frontend\popout-test.html`],
    ['htm', '/Users/me/repo/demo.htm'],
  ])('%s 走 preview:// 沙箱 iframe，不调系统默认程序', (_ext, path) => {
    const { container } = render(<FilePreviewContent path={path} />)

    const iframe = container.querySelector('iframe.pv-iframe')
    expect(iframe).toBeInTheDocument()
    expect(iframe).toHaveAttribute('src', `http://preview.localhost/${path}`)
    expect(iframe).toHaveAttribute('sandbox', expect.stringContaining('allow-scripts'))
    expect(convertFileSrc).toHaveBeenCalledWith(path, 'preview')

    // 关键回归点：HTML 绝不能再落进「非文本 → openPath」那条路
    expect(openPath).not.toHaveBeenCalled()
    expect(readFile).not.toHaveBeenCalled()
    expect(readFileBase64).not.toHaveBeenCalled()
    expect(screen.queryByText('已请求系统默认程序打开')).not.toBeInTheDocument()
    expect(screen.queryByText('读取中…')).not.toBeInTheDocument()
  })

  it('docx 等真·非预览类型仍走系统默认程序打开', () => {
    const path = String.raw`C:\repo\out\报告.docx`
    render(<FilePreviewContent path={path} />)

    expect(openPath).toHaveBeenCalledWith(path)
    expect(screen.getByText('已请求系统默认程序打开')).toBeInTheDocument()
  })
})
