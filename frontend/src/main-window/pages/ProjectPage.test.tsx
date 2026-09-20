import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ProjectCenter } from './ProjectPage'

/**
 * 项目中心「文件夹归档 / 恢复」回归（Phase 2 修订）。
 *
 * 归档的**恢复入口**从会话工作台头部菜单迁到项目中心：
 * - 「项目书签」区只列**未归档**书签，「已归档文件夹」区只列归档项（分区只是展示切分，
 *   书签表本身仍是全量：删除/新增书签时归档项必须原样保留）；
 * - 恢复 = `set_project_folder_archived(path, false)` + 回填返回的最新书签表；
 * - **恢复 ≠ 切换**：不调用 set_project_dir（当前工作目录保持不变）；
 * - 空态有文案；失败显式提示（不静默吞错）。
 */
const getProjectDir = vi.fn()
const getProjectBookmarks = vi.fn()
const setProjectBookmarks = vi.fn()
const setProjectDir = vi.fn()
const setProjectFolderArchived = vi.fn()

vi.mock('../lib/api', () => ({
  getProjectDir: () => getProjectDir(),
  getProjectBookmarks: () => getProjectBookmarks(),
  setProjectBookmarks: (list: unknown) => setProjectBookmarks(list),
  setProjectDir: (path: string) => setProjectDir(path),
  setProjectFolderArchived: (path: string, archived: boolean) =>
    setProjectFolderArchived(path, archived),
}))

// 目录选择对话框（@tauri-apps/plugin-dialog）与被测逻辑无关，隔离掉免触碰真实 IPC
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

/** 后端书签形状（archived 缺省 = 未归档） */
function bookmark(name: string, path: string, archived = false) {
  return archived ? { name, path, archived: true } : { name, path }
}

/** 分区容器：Section 渲染为 <section> + <h3 class="section-title">，用标题反查 */
function section(title: string): HTMLElement {
  return screen.getByText(title).closest('section') as HTMLElement
}

describe('项目中心：文件夹归档分区与恢复', () => {
  beforeEach(() => {
    getProjectDir.mockReset().mockResolvedValue({ path: '', name: '', tag: 'default' })
    getProjectBookmarks
      .mockReset()
      .mockResolvedValue([
        bookmark('一号', 'E:\\NUS\\1'),
        bookmark('已归档目录', 'E:\\work\\Old', true),
      ])
    setProjectBookmarks.mockReset().mockResolvedValue([])
    setProjectDir.mockReset().mockResolvedValue({ path: '', name: '', tag: 'default' })
    setProjectFolderArchived.mockReset().mockResolvedValue([])
  })

  it('未归档书签只出现在书签区，archived 只出现在已归档区', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const active = within(section('项目书签'))
    expect(active.getByText('一号')).toBeInTheDocument()
    expect(active.queryByText('已归档目录')).not.toBeInTheDocument()

    const archived = within(section('已归档文件夹'))
    expect(archived.getByText('已归档目录')).toBeInTheDocument()
    expect(archived.queryByText('一号')).not.toBeInTheDocument()
  })

  it('点「恢复」调用 set_project_folder_archived(path,false)，回填后回到书签区', async () => {
    setProjectFolderArchived.mockResolvedValue([
      bookmark('一号', 'E:\\NUS\\1'),
      bookmark('已归档目录', 'E:\\work\\Old'),
    ])
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('已归档目录')).toBeInTheDocument())

    fireEvent.click(within(section('已归档文件夹')).getByRole('button', { name: '恢复' }))
    await waitFor(() =>
      expect(setProjectFolderArchived).toHaveBeenCalledWith('E:\\work\\Old', false),
    )

    // 立即刷新：文件回到书签区，已归档区回到空态（不重新拉后端，直接用返回表回填）
    await waitFor(() =>
      expect(within(section('项目书签')).getByText('已归档目录')).toBeInTheDocument(),
    )
    expect(within(section('已归档文件夹')).queryByText('已归档目录')).not.toBeInTheDocument()
    expect(within(section('已归档文件夹')).getByText('暂无已归档文件夹')).toBeInTheDocument()
  })

  it('恢复 ≠ 切换：只改归档标记，不调用 set_project_dir', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('已归档目录')).toBeInTheDocument())

    fireEvent.click(within(section('已归档文件夹')).getByRole('button', { name: '恢复' }))
    await waitFor(() => expect(setProjectFolderArchived).toHaveBeenCalledTimes(1))

    expect(setProjectDir).not.toHaveBeenCalled()
  })

  it('无归档项时给出空态文案，且不渲染「恢复」按钮', async () => {
    getProjectBookmarks.mockResolvedValue([bookmark('一号', 'E:\\NUS\\1')])
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const archived = within(section('已归档文件夹'))
    expect(archived.getByText('暂无已归档文件夹')).toBeInTheDocument()
    expect(archived.queryByRole('button', { name: '恢复' })).not.toBeInTheDocument()
  })

  it('恢复失败显式提示（恢复失败），不静默吞掉', async () => {
    // 后端不可读错误形态（原始 IPC 文本）→ 走调用方兜底文案
    setProjectFolderArchived.mockRejectedValue(
      new Error('IPC invoke set_project_folder_archived failed'),
    )
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('已归档目录')).toBeInTheDocument())

    fireEvent.click(within(section('已归档文件夹')).getByRole('button', { name: '恢复' }))
    await waitFor(() => expect(screen.getByText('恢复失败')).toBeInTheDocument())
  })

  it('分区只是展示切分：删除书签时归档项仍在提交表内（不丢归档记录）', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    fireEvent.click(within(section('项目书签')).getByTitle('删除书签'))
    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))

    expect(setProjectBookmarks).toHaveBeenCalledWith([
      { name: '已归档目录', path: 'E:\\work\\Old', archived: true },
    ])
  })
})
