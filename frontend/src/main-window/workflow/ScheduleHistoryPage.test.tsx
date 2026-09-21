import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ScheduleHistoryPage } from './ScheduleHistoryPage'
import * as api from '../lib/api'

vi.mock('../lib/api', async importOriginal => ({
  ...(await importOriginal<typeof import('../lib/api')>()),
  listWorkflows: vi.fn(),
  wfScheduleHistoryList: vi.fn(),
  wfScheduleHistoryDelete: vi.fn(),
}))

describe('ScheduleHistoryPage', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(api.listWorkflows).mockResolvedValue([
      { id: 'wf-1', title: '日报', status: 'active', steps: [], tags: [], created_at: 0, updated_at: 0, run_count: 0 },
    ])
    vi.mocked(api.wfScheduleHistoryList).mockResolvedValue({
      total: 1,
      page: 0,
      page_size: 50,
      runs: [{
        run_id: 'run-1', workflow_id: 'wf-1', workflow_title: '日报',
        started_at: '2026-09-21T01:00:00Z', finished_at: '2026-09-21T01:00:02Z',
        status: 'Success', steps: [],
      }],
    })
  })

  it('显示运行状态并从记录打开画布回放', async () => {
    const onOpenReplay = vi.fn()
    render(<ScheduleHistoryPage onOpenReplay={onOpenReplay} />)
    expect((await screen.findAllByText('日报')).length).toBeGreaterThan(0)
    expect(screen.getAllByText('成功').length).toBeGreaterThan(0)
    expect(screen.getByText('2.0 s')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /查看回放/ }))
    expect(onOpenReplay).toHaveBeenCalledWith('wf-1', 'run-1')
  })

  it('筛选工作流和状态后从第一页重新查询', async () => {
    render(<ScheduleHistoryPage onOpenReplay={() => {}} />)
    await screen.findAllByText('日报')
    fireEvent.change(screen.getByLabelText('工作流'), { target: { value: 'wf-1' } })
    fireEvent.change(screen.getByLabelText('状态'), { target: { value: 'error' } })
    await waitFor(() => expect(api.wfScheduleHistoryList).toHaveBeenLastCalledWith(expect.objectContaining({ workflow_id: 'wf-1', status: 'error', page: 0 })))
  })
})
