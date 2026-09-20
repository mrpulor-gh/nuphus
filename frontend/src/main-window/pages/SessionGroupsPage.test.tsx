import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { SessionGroupsPage } from './SessionGroupsPage'

/**
 * 会话工作台分组设置页（折叠上限）回归：
 * - 读数来自 `list_shelf_sessions.collapsed_limit`（后端唯一读数入口，无专用 get 命令）；
 * - 写入 `set_session_group_collapsed_limit`（后端拒绝 0 → 前端先校验，不发无效 IPC）；
 * - 保存成功给出可感知反馈（已保存徽标 + 输入回填后端生效值）。
 */
const listShelfSessions = vi.fn()
const setSessionGroupCollapsedLimit = vi.fn()

vi.mock('../lib/api', () => ({
  listShelfSessions: () => listShelfSessions(),
  setSessionGroupCollapsedLimit: (limit: number) => setSessionGroupCollapsedLimit(limit),
}))

function shelfResponse(collapsed_limit: number) {
  return {
    can_switch: true,
    items: [],
    projects: [],
    archived_projects: [],
    collapsed_limit,
  }
}

describe('SessionGroupsPage 折叠上限设置', () => {
  beforeEach(() => {
    listShelfSessions.mockReset().mockImplementation(async () => shelfResponse(8))
    setSessionGroupCollapsedLimit.mockReset().mockImplementation(async (n: number) => n)
  })

  it('加载时回填后端生效值', async () => {
    render(<SessionGroupsPage />)
    const input = (await screen.findByRole('spinbutton')) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe('8'))
  })

  it('非法输入（0）不发 IPC，给出前端校验提示（后端拒绝 0）', async () => {
    render(<SessionGroupsPage />)
    const input = (await screen.findByRole('spinbutton')) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe('8'))

    fireEvent.change(input, { target: { value: '0' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(await screen.findByText('请输入 1 以上的整数')).toBeInTheDocument()
    expect(setSessionGroupCollapsedLimit).not.toHaveBeenCalled()
  })

  it('合法输入写盘并给出可感知反馈', async () => {
    render(<SessionGroupsPage />)
    const input = (await screen.findByRole('spinbutton')) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe('8'))

    fireEvent.change(input, { target: { value: '3' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(setSessionGroupCollapsedLimit).toHaveBeenCalledWith(3))
    expect(await screen.findByText('已保存')).toBeInTheDocument()
  })

  it('写盘失败显式提示，不静默吞掉', async () => {
    setSessionGroupCollapsedLimit.mockRejectedValue('invalid_limit')
    render(<SessionGroupsPage />)
    const input = (await screen.findByRole('spinbutton')) as HTMLInputElement
    await waitFor(() => expect(input.value).toBe('8'))

    fireEvent.change(input, { target: { value: '5' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(await screen.findByText('保存失败')).toBeInTheDocument()
  })
})
