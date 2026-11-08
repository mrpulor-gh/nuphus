import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AppContextMenu from '../ui/AppContextMenu'
import { invoke } from '../core/bridge'

const { saveDialog } = vi.hoisted(() => ({ saveDialog: vi.fn() }))

vi.mock('../core/bridge', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: saveDialog }))

describe('文件条目右键菜单', () => {
  const writeText = vi.fn(() => Promise.resolve())

  beforeEach(() => {
    vi.clearAllMocks()
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
  })

  it('文件条目显示复制路径、在文件夹中显示与询问入口', async () => {
    const filePath = String.raw`C:\repo\src\main.rs`
    render(
      <>
        <AppContextMenu />
        <button data-file-path={filePath}>main.rs</button>
      </>,
    )
    fireEvent.contextMenu(screen.getByText('main.rs'), { clientX: 20, clientY: 20 })
    expect(screen.getByRole('button', { name: '复制路径' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /另存为/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '问问 Nuphus' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '在文件夹中显示' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('reveal_path', { path: filePath }))
  })

  it('复制路径只复制完整解析路径', async () => {
    const filePath = '/Users/me/repo/src/main.rs'
    render(
      <>
        <AppContextMenu />
        <button data-file-path={filePath}>main.rs</button>
      </>,
    )
    fireEvent.contextMenu(screen.getByText('main.rs'))
    fireEvent.click(screen.getByRole('button', { name: '复制路径' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(filePath))
  })

  it('另存为：保存对话框选定目标后调用后端复制', async () => {
    const filePath = String.raw`C:\shots\out.png`
    const dest = String.raw`D:\backup\out.png`
    saveDialog.mockResolvedValue(dest)
    render(
      <>
        <AppContextMenu />
        <button data-file-path={filePath}>out.png</button>
      </>,
    )
    fireEvent.contextMenu(screen.getByText('out.png'), { clientX: 20, clientY: 20 })
    fireEvent.click(screen.getByRole('button', { name: /另存为/ }))
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('copy_file_as', { src: filePath, dest }),
    )
    expect(saveDialog).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: 'out.png' }))
  })

  it('普通文本仍显示原复制与询问菜单', () => {
    render(
      <>
        <AppContextMenu />
        <p>普通文本内容</p>
      </>,
    )
    fireEvent.contextMenu(screen.getByText('普通文本内容'), { clientX: 20, clientY: 20 })
    expect(screen.getByRole('button', { name: '复制' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '问问 Nuphus' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '在文件夹中显示' })).not.toBeInTheDocument()
  })
})
