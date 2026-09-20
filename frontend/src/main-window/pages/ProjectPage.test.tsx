import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ProjectCenter } from './ProjectPage'

/**
 * 项目中心回归（Phase 2 终稿：归档区撤下）。
 *
 * 归档文件夹**不出现在项目中心**：书签区只列未归档书签，且**没有**「已归档文件夹」区
 * —— 归档的恢复入口唯一收敛到会话工作台「项目 ⋯ → 恢复隐藏项目 (N)」
 * （行为断言见 src/test/session-rail-groups.test.tsx）。
 *
 * 但书签**表**仍是全量：新增/删除书签是整表提交，归档项必须原样带上，
 * 否则恢复入口的锚点记录会被静默抹掉。
 */
const getProjectDir = vi.fn()
const getProjectBookmarks = vi.fn()
const setProjectBookmarks = vi.fn()
const setProjectDir = vi.fn()

vi.mock('../lib/api', () => ({
  getProjectDir: () => getProjectDir(),
  getProjectBookmarks: () => getProjectBookmarks(),
  setProjectBookmarks: (list: unknown) => setProjectBookmarks(list),
  setProjectDir: (path: string) => setProjectDir(path),
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

describe('项目中心：归档文件夹不可见（恢复入口在会话工作台）', () => {
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
  })

  it('只渲染「当前项目目录」与「项目书签」两区：已归档文件夹区已撤下', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const titles = Array.from(document.querySelectorAll('.section-title')).map(e => e.textContent)
    expect(titles).toEqual(['当前项目目录', '项目书签'])
  })

  it('归档书签不出现在书签区，也不出现在页面任何位置（含「恢复」入口）', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    const active = within(section('项目书签'))
    expect(active.getByText('一号')).toBeInTheDocument()
    expect(active.queryByText('已归档目录')).not.toBeInTheDocument()

    expect(screen.queryByText('已归档目录')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '恢复' })).not.toBeInTheDocument()
  })

  it('删除书签时归档项仍在提交表内（不丢归档记录）', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    fireEvent.click(within(section('项目书签')).getByTitle('删除书签'))
    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))

    expect(setProjectBookmarks).toHaveBeenCalledWith([
      { name: '已归档目录', path: 'E:\\work\\Old', archived: true },
    ])
  })

  it('新增书签时归档项仍在提交表内（恢复锚点不被整表替换抹掉）', async () => {
    render(<ProjectCenter />)
    await waitFor(() => expect(screen.getByText('一号')).toBeInTheDocument())

    fireEvent.change(screen.getByPlaceholderText('输入项目目录路径'), {
      target: { value: 'E:\\work\\New' },
    })
    fireEvent.change(screen.getByPlaceholderText('书签名称（可选）'), {
      target: { value: '新书签' },
    })
    fireEvent.click(screen.getByRole('button', { name: '将当前目录加入书签' }))
    await waitFor(() => expect(setProjectBookmarks).toHaveBeenCalledTimes(1))

    expect(setProjectBookmarks).toHaveBeenCalledWith([
      { name: '一号', path: 'E:\\NUS\\1' },
      { name: '已归档目录', path: 'E:\\work\\Old', archived: true },
      { name: '新书签', path: 'E:\\work\\New' },
    ])
  })
})
