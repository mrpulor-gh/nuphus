/**
 * ShortcutsDialog（v1.5 快捷键编辑）组件测试：
 * 打开合并清单、行内录制往返、冲突拒绝（保留键 / 其它项）、仅修饰键忽略、Esc 取消、
 * 恢复全部默认、保存失败回滚、录制态广播（App 据此挂起全局分支）。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../../locales'
import { ShortcutsDialog } from './ShortcutsDialog'

const mocks = vi.hoisted(() => ({
  get: vi.fn(async () => ({}) as Record<string, string>),
  set: vi.fn(async (next: Record<string, string>) => next),
}))

vi.mock('../lib/api', () => ({
  getShortcutOverrides: mocks.get,
  setShortcutOverrides: mocks.set,
}))

interface DialogHandlers {
  onOverridesChange?: (overrides: Record<string, string>) => void
  onRecordingChange?: (recording: boolean) => void
}

function renderDialog(handlers: DialogHandlers = {}) {
  render(
    <LangProvider>
      <ShortcutsDialog open onClose={() => {}} {...handlers} />
    </LangProvider>,
  )
}

const rowByName = (name: RegExp) => screen.getByRole('button', { name })

describe('ShortcutsDialog 快捷键编辑', () => {
  beforeEach(() => {
    mocks.get.mockReset()
    mocks.get.mockResolvedValue({})
    mocks.set.mockReset()
    mocks.set.mockImplementation(async (next: Record<string, string>) => next)
  })

  it('打开读取覆盖表：默认 + 覆盖合并显示；分段为不可编辑在上、可编辑在下', async () => {
    mocks.get.mockResolvedValue({ palette: 'Ctrl+Shift+K' })
    renderDialog()

    await waitFor(() => expect(screen.getByText('Ctrl+Shift+K')).toBeInTheDocument())
    expect(screen.getByText('已自定义')).toBeInTheDocument()
    expect(
      screen.getByText('点击「可编辑」中的行，按下新的组合键即可修改（按 Esc 取消）'),
    ).toBeInTheDocument()
    // 默认集仍在（未覆盖项显示默认键位）
    expect(screen.getByText('Ctrl+F')).toBeInTheDocument()
    // 分段：不可编辑（输入交互键）在上，可编辑（命令键 + 全局热键）在下
    const titles = Array.from(document.querySelectorAll('.shortcuts-group-title')).map(
      el => el.textContent,
    )
    expect(titles).toEqual(['不可编辑', '可编辑'])
    expect(document.querySelectorAll('.shortcuts-row.is-editable').length).toBe(8)
  })

  it('录制往返：点击行 → 按下新组合键 → 落库并回传，行内显示新键位', async () => {
    const onOverridesChange = vi.fn()
    renderDialog({ onOverridesChange })
    await waitFor(() => expect(screen.getByText('Ctrl+K')).toBeInTheDocument())

    fireEvent.click(rowByName(/打开命令面板/))
    expect(screen.getByText('按下新组合键')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: 'j', ctrlKey: true, shiftKey: true })

    await waitFor(() => expect(mocks.set).toHaveBeenCalledWith({ palette: 'Ctrl+Shift+J' }))
    await waitFor(() => expect(screen.getByText('Ctrl+Shift+J')).toBeInTheDocument())
    expect(onOverridesChange).toHaveBeenCalledWith({ palette: 'Ctrl+Shift+J' })
    // 录制态退出，「已自定义」标记出现
    expect(screen.queryByText('按下新组合键')).not.toBeInTheDocument()
    expect(screen.getByText('已自定义')).toBeInTheDocument()
  })

  it('冲突（其它可编辑项）：提示并拒绝，保持录制等待下一个组合', async () => {
    renderDialog()
    await waitFor(() => expect(screen.getByText('Ctrl+K')).toBeInTheDocument())

    fireEvent.click(rowByName(/打开工作流列表/))
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true })

    expect(mocks.set).not.toHaveBeenCalled()
    expect(screen.getByRole('alert')).toHaveTextContent('该组合已被其他操作占用，请换一组')
    expect(screen.getByText('按下新组合键')).toBeInTheDocument()
  })

  it('冲突（系统保留键）：提示并拒绝', async () => {
    renderDialog()
    await waitFor(() => expect(screen.getByText('Ctrl+K')).toBeInTheDocument())

    fireEvent.click(rowByName(/打开命令面板/))
    fireEvent.keyDown(window, { key: 'c', ctrlKey: true })

    expect(mocks.set).not.toHaveBeenCalled()
    expect(screen.getByRole('alert')).toHaveTextContent('该组合为系统保留键，请换一组')
  })

  it('仅修饰键忽略（仍在录制）；Esc 取消录制且不落库', async () => {
    renderDialog()
    await waitFor(() => expect(screen.getByText('Ctrl+K')).toBeInTheDocument())

    fireEvent.click(rowByName(/打开命令面板/))
    fireEvent.keyDown(window, { key: 'Control', ctrlKey: true })
    expect(screen.getByText('按下新组合键')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByText('按下新组合键')).not.toBeInTheDocument()
    expect(mocks.set).not.toHaveBeenCalled()
  })

  it('「恢复全部默认」：清空覆盖表并回传，清空后按钮禁用', async () => {
    mocks.get.mockResolvedValue({ palette: 'Ctrl+Shift+K', newChat: 'Ctrl+Alt+N' })
    const onOverridesChange = vi.fn()
    renderDialog({ onOverridesChange })
    await waitFor(() => expect(screen.getByText('Ctrl+Shift+K')).toBeInTheDocument())

    const reset = screen.getByRole('button', { name: '恢复全部默认' })
    expect(reset).toBeEnabled()
    fireEvent.click(reset)

    await waitFor(() => expect(mocks.set).toHaveBeenCalledWith({}))
    await waitFor(() => expect(reset).toBeDisabled())
    expect(onOverridesChange).toHaveBeenCalledWith({})
    expect(screen.getByText('Ctrl+K')).toBeInTheDocument()
  })

  it('保存失败：回滚显示并提示（界面不得与落库背离）', async () => {
    mocks.set.mockRejectedValueOnce(new Error('backend down'))
    renderDialog()
    await waitFor(() => expect(screen.getByText('Ctrl+K')).toBeInTheDocument())

    fireEvent.click(rowByName(/打开命令面板/))
    fireEvent.keyDown(window, { key: 'j', ctrlKey: true })

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('backend down'))
    expect(screen.getByText('Ctrl+K')).toBeInTheDocument()
  })

  it('录制态广播：进入录制 → true，结束 → false（App 据此挂起全局分支）', async () => {
    const onRecordingChange = vi.fn()
    renderDialog({ onRecordingChange })
    await waitFor(() => expect(screen.getByText('Ctrl+K')).toBeInTheDocument())

    fireEvent.click(rowByName(/打开命令面板/))
    await waitFor(() => expect(onRecordingChange).toHaveBeenCalledWith(true))

    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(onRecordingChange).toHaveBeenCalledWith(false))
  })

  it('全局热键：拒绝无修饰键组合（保持录制并提示）', async () => {
    renderDialog()
    await waitFor(() => expect(screen.getByText('Ctrl+1')).toBeInTheDocument())

    fireEvent.click(rowByName(/全局截图/))
    fireEvent.keyDown(window, { key: '5' })

    expect(mocks.set).not.toHaveBeenCalled()
    expect(screen.getByRole('alert')).toHaveTextContent(
      '全局热键需要包含 Ctrl / Alt / Shift 修饰键',
    )
    expect(screen.getByText('按下新组合键')).toBeInTheDocument()
  })

  it('全局热键：带修饰键改键即落库并回传', async () => {
    const onOverridesChange = vi.fn()
    renderDialog({ onOverridesChange })
    await waitFor(() => expect(screen.getByText('Ctrl+1')).toBeInTheDocument())

    fireEvent.click(rowByName(/全局截图/))
    fireEvent.keyDown(window, { key: 'F9', ctrlKey: true })

    await waitFor(() =>
      expect(mocks.set).toHaveBeenCalledWith({ 'global.tool.screenshot': 'Ctrl+F9' }),
    )
    await waitFor(() => expect(screen.getByText('Ctrl+F9')).toBeInTheDocument())
    expect(onOverridesChange).toHaveBeenCalledWith({ 'global.tool.screenshot': 'Ctrl+F9' })
  })
})
