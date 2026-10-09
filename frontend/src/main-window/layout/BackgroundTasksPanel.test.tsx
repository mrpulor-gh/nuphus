import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BackgroundTaskView } from '../../core/types'
import { BackgroundTasksPanel } from './BackgroundTasksPanel'
import { countRetainedBackgroundTasks, killBackgroundTask, listBackgroundTasks } from '../lib/api'
import { showAppFeedback } from '../../ui/islandChannel'

vi.mock('../lib/api', () => ({
  countRetainedBackgroundTasks: vi.fn(async () => 0),
  listBackgroundTasks: vi.fn(async () => [] as unknown[]),
  killBackgroundTask: vi.fn(async () => 'ok'),
  backendErrorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}))
vi.mock('../../ui/islandChannel', () => ({ showAppFeedback: vi.fn() }))

const task = (over: Partial<BackgroundTaskView> = {}): BackgroundTaskView => ({
  id: 'bg-3',
  tool: 'system_shell',
  command: 'yarn build',
  pid: 42140,
  started_at_ms: 1_700_000_000_000,
  elapsed_ms: 751_000,
  retain: true,
  output_path: 'C:/data/background_tasks/bg-3.log',
  ...over,
})

function stubLedger(retained: number, rows: BackgroundTaskView[] = []) {
  vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(retained)
  vi.mocked(listBackgroundTasks).mockResolvedValue(rows)
}

describe('后台任务账本面板（可见 + 可结束）', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    stubLedger(0)
  })
  afterEach(() => {
    cleanup()
  })

  it('有保留任务时入口出现，并显示正确数量', async () => {
    stubLedger(2)
    render(<BackgroundTasksPanel executionStage="idle" />)
    const entry = await screen.findByRole('button', { name: /后台任务/ })
    expect(entry).toHaveTextContent('2')
  })

  it('无保留任务时入口完全不出现（不占界面、不进 tab 序）', async () => {
    stubLedger(0)
    const { container } = render(<BackgroundTasksPanel executionStage="idle" />)
    await act(async () => {
      await Promise.resolve()
    })
    expect(screen.queryByRole('button', { name: /后台任务/ })).not.toBeInTheDocument()
    expect(container.querySelector('.bgt-entry')).toBeNull()
    expect(container.querySelector('.bgt-panel')).toBeNull()
  })

  it('列表未打开时不发起列表查询（关闭态零 invoke）', async () => {
    stubLedger(1, [task()])
    render(<BackgroundTasksPanel executionStage="idle" />)
    await screen.findByRole('button', { name: /后台任务/ })
    await act(async () => {
      await Promise.resolve()
    })
    expect(listBackgroundTasks).not.toHaveBeenCalled()
    expect(countRetainedBackgroundTasks).toHaveBeenCalled()
  })

  it('打开面板后逐行呈现命令 / PID / 时长 / 保留徽标 / 输出路径', async () => {
    stubLedger(1, [task()])
    render(<BackgroundTasksPanel executionStage="idle" />)
    fireEvent.click(await screen.findByRole('button', { name: /后台任务/ }))

    expect(await screen.findByText('yarn build')).toBeInTheDocument()
    expect(screen.getByText('PID 42140')).toBeInTheDocument()
    // 751000ms → 12m31s（本地格式化，不自算起点）
    expect(screen.getByText(/12m31s/)).toBeInTheDocument()
    expect(screen.getByText('后台保留')).toBeInTheDocument()
    expect(screen.getByText('C:/data/background_tasks/bg-3.log')).toBeInTheDocument()
  })

  it('结束需二次确认，确认后带 id 调用 kill 并刷新列表', async () => {
    stubLedger(1, [task()])
    vi.mocked(killBackgroundTask).mockResolvedValue(
      '后台任务 bg-3（yarn build，PID 42140）已终止（含子孙进程）',
    )
    render(<BackgroundTasksPanel executionStage="idle" />)
    fireEvent.click(await screen.findByRole('button', { name: /后台任务/ }))
    await screen.findByText('yarn build')

    // 第一次点击只是打开确认框，绝不能直接杀
    fireEvent.click(screen.getAllByRole('button', { name: '结束' })[0])
    expect(killBackgroundTask).not.toHaveBeenCalled()
    expect(await screen.findByText(/子孙进程/)).toBeInTheDocument()

    const listCallsBeforeKill = vi.mocked(listBackgroundTasks).mock.calls.length
    fireEvent.click(screen.getAllByRole('button', { name: '结束' })[1])

    await waitFor(() => expect(killBackgroundTask).toHaveBeenCalledTimes(1))
    expect(killBackgroundTask).toHaveBeenCalledWith('bg-3')
    // 成功后立即刷新（计数 + 列表）
    await waitFor(() =>
      expect(vi.mocked(listBackgroundTasks).mock.calls.length).toBeGreaterThan(listCallsBeforeKill),
    )
    // 事实句原样透出，不被本地化模板覆盖
    expect(showAppFeedback).toHaveBeenCalledWith(
      '后台任务 bg-3（yarn build，PID 42140）已终止（含子孙进程）',
      'success',
    )
  })

  it('后端结束失败时给出可读错误，不静默', async () => {
    stubLedger(1, [task()])
    vi.mocked(killBackgroundTask).mockRejectedValue(new Error('后台任务 bg-3 不存在或已结束'))
    render(<BackgroundTasksPanel executionStage="idle" />)
    fireEvent.click(await screen.findByRole('button', { name: /后台任务/ }))
    await screen.findByText('yarn build')

    fireEvent.click(screen.getAllByRole('button', { name: '结束' })[0])
    await screen.findByText(/子孙进程/)
    fireEvent.click(screen.getAllByRole('button', { name: '结束' })[1])

    await waitFor(() =>
      expect(showAppFeedback).toHaveBeenCalledWith(
        expect.stringContaining('结束后台任务失败'),
        'error',
      ),
    )
    expect(showAppFeedback).toHaveBeenCalledWith(
      expect.stringContaining('后台任务 bg-3 不存在或已结束'),
      'error',
    )
  })

  it('列表查询失败时显示错误，而不是伪装成空清单', async () => {
    stubLedger(1, [task()])
    vi.mocked(listBackgroundTasks).mockRejectedValue(new Error('IPC down'))
    render(<BackgroundTasksPanel executionStage="idle" />)
    fireEvent.click(await screen.findByRole('button', { name: /后台任务/ }))
    expect(await screen.findByRole('alert')).toHaveTextContent('IPC down')
  })

  it('面板打开时轮询列表，关闭后立刻停止（关闭态不烧 invoke）', async () => {
    vi.useFakeTimers()
    try {
      stubLedger(1, [task()])
      render(<BackgroundTasksPanel executionStage="idle" />)
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1)
      })

      fireEvent.click(screen.getByRole('button', { name: /后台任务/ }))
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1)
      })
      const callsOnOpen = vi.mocked(listBackgroundTasks).mock.calls.length
      expect(callsOnOpen).toBeGreaterThan(0)

      // 打开状态下按周期继续拉
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2100)
      })
      expect(vi.mocked(listBackgroundTasks).mock.calls.length).toBeGreaterThan(callsOnOpen)

      // 关闭后一次都不该再有
      const beforeClose = vi.mocked(listBackgroundTasks).mock.calls.length
      fireEvent.click(screen.getByRole('button', { name: '关闭' }))
      await act(async () => {
        await vi.advanceTimersByTimeAsync(6000)
      })
      expect(vi.mocked(listBackgroundTasks).mock.calls.length).toBe(beforeClose)
    } finally {
      vi.useRealTimers()
    }
  })

  it('执行态转空闲时刷新保留项数量（长任务超时的唯一可靠触发点）', async () => {
    stubLedger(0)
    const { rerender } = render(<BackgroundTasksPanel executionStage="running" />)
    await act(async () => {
      await Promise.resolve()
    })
    expect(screen.queryByRole('button', { name: /后台任务/ })).not.toBeInTheDocument()

    // 这一轮执行刚结束：长任务超时形成的保留项此刻才可能出现
    stubLedger(1)
    rerender(<BackgroundTasksPanel executionStage="idle" />)
    expect(await screen.findByRole('button', { name: /后台任务/ })).toBeInTheDocument()
  })
})
