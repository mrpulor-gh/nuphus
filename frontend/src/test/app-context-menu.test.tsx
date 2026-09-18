import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AppContextMenu from '../ui/AppContextMenu'
import { invoke } from '../core/bridge'

vi.mock('../core/bridge', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }))

describe('文件条目右键菜单', () => {
  const writeText = vi.fn(() => Promise.resolve())

  beforeEach(() => {
    vi.clearAllMocks()
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
  })

  it('文件条目显示复制路径与在文件夹中显示', async () => {
    const filePath = String.raw`C:\repo\src\main.rs`
    render(
      <>
        <AppContextMenu />
        <button data-file-path={filePath}>main.rs</button>
      </>,
    )
    fireEvent.contextMenu(screen.getByText('main.rs'), { clientX: 20, clientY: 20 })
    expect(screen.getByRole('button', { name: '复制路径' })).toBeInTheDocument()
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
