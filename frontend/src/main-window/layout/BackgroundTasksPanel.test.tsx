import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { listen } from '../../core/bridge'
import type { BackgroundTaskView } from '../../core/types'
import { BackgroundTasksPanel } from './BackgroundTasksPanel'
import { countRetainedBackgroundTasks, killBackgroundTask, listBackgroundTasks } from '../lib/api'
import { showAppFeedback } from '../../ui/islandChannel'

vi.mock('../../core/bridge', () => ({ listen: vi.fn(async () => () => {}) }))

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
  title: null,
  pid: 42140,
  started_at_ms: 1_700_000_000_000,
  elapsed_ms: 751_000,
  max_lifetime_secs: 3600,
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

  it('打开面板后逐行呈现标题 / PID / 时长 / 状态 / 输出路径', async () => {
    stubLedger(1, [task()])
    const { container } = render(<BackgroundTasksPanel executionStage="idle" />)
    fireEvent.click(await screen.findByRole('button', { name: /后台任务/ }))

    // 无 title → 主行回落命令（截断排版，全量在 title 属性里）
    expect(await screen.findByText('yarn build')).toBeInTheDocument()
    // 751000ms → 12m31s（本地格式化，不自算起点）
    expect(screen.getByText(/12m31s/)).toBeInTheDocument()
    // 状态词 + 相对时长同层可见（2026-10-26：删掉与状态词重复的「后台保留」徽标）
    expect(screen.getByText('保留在后台')).toBeInTheDocument()
    expect(screen.getByText(/已运行/)).toBeInTheDocument()
    // 次要行：PID 与输出路径降为小字一行，全量（含起于时刻）在 hover 里
    expect(screen.getByText('PID 42140')).toBeInTheDocument()
    expect(screen.getByText('C:/data/background_tasks/bg-3.log')).toBeInTheDocument()
    const sub = container.querySelector('.bgt-sub')
    expect(sub?.getAttribute('title')).toContain(new Date(task().started_at_ms).toLocaleString())
  })

  it('填了标题时主行显示标题（普通用户看得懂），命令降为辅助行', async () => {
    // 2026-10-26 大王要求：后台任务对普通用户要「一看即知在干什么」。
    // 调用方填了 title → 主行显示它；裸命令退居辅助行；上限/状态一并可见。
    stubLedger(1, [task({ title: '依赖全量检查' })])
    render(<BackgroundTasksPanel executionStage="idle" />)
    fireEvent.click(await screen.findByRole('button', { name: /后台任务/ }))

    const headline = await screen.findByText('依赖全量检查')
    expect(headline.className).toContain('bgt-headline')
    // 命令仍可见（辅助行），但已不是主行
    expect(await screen.findByText('yarn build')).toBeInTheDocument()
    // 「最长存活多久」必须明示（到点系统兜底结束）
    expect(await screen.findByText(/剩余约/)).toBeInTheDocument()
    // 状态词与运行时长同时可见
    expect(await screen.findByText('保留在后台')).toBeInTheDocument()
    expect(await screen.findByText(/已运行/)).toBeInTheDocument()
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

  it('面板开着时列表轮询把胶囊数字对齐到列表（任务消失则入口收回）', async () => {
    // 缺陷复现（2026-10-11 修复）：数字与列表是两路独立真相。列表每 2s 刷新，
    // 数字只在事件驱动点刷新 → 保留任务自然结束时列表少了它、胶囊数字仍停在旧值，
    // 面板也不会自动关。修复：轮询用同源列表直接派生数字。
    stubLedger(1, [task({ id: 'bg-1', retain: true })])
    render(<BackgroundTasksPanel executionStage="idle" />)
    const entry = await screen.findByRole('button', { name: /后台任务/ })
    expect(entry).toHaveTextContent('1')

    // 打开面板 → 后端账本里该任务已自然退出（列表空，但计数命令仍是旧值）
    fireEvent.click(entry)
    await screen.findByRole('dialog')

    // 保留计数命令仍报 1（第二路真相，与列表不一致）——数字必须听列表的
    vi.mocked(countRetainedBackgroundTasks).mockResolvedValue(1)
    vi.mocked(listBackgroundTasks).mockResolvedValue([])

    await waitFor(
      () => {
        expect(screen.queryByRole('button', { name: /后台任务/ })).not.toBeInTheDocument()
      },
      { timeout: 4000 },
    )
    await waitFor(
      () => {
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      },
      { timeout: 4000 },
    )
  })

  it('列表拉取失败时数字保持不动（不把读失败误判成「没有后台任务」）', async () => {
    stubLedger(1, [task({ id: 'bg-1', retain: true })])
    render(<BackgroundTasksPanel executionStage="idle" />)
    const entry = await screen.findByRole('button', { name: /后台任务/ })
    fireEvent.click(entry)
    await screen.findByRole('dialog')

    // 后端不可达：列表报 error，数字**不得**被清零（清零 = 谎报「没有任务」）
    vi.mocked(listBackgroundTasks).mockRejectedValue(new Error('后端不可达'))
    // 超时须放宽：失败要等下一轮 2s 轮询才发生（findByRole 默认仅 1s）
    await screen.findByRole('alert', undefined, { timeout: 4000 })
    expect(screen.getByRole('button', { name: /后台任务/ })).toHaveTextContent('1')
  })

  it('订阅后端账本变更事件：面板关着也能即时出现胶囊（无需刷新页面）', async () => {
    // 缺陷复现（2026-10-11 方案 B）：retain 翻转发生在工具执行中，而「执行转空闲」
    // 信号常在其后——两者错过即数字永远停在 0，只能靠刷新页面的挂载校准。
    // 后端 register/set_retain/unregister 时广播 background_tasks_changed。
    stubLedger(0)
    render(<BackgroundTasksPanel executionStage="idle" />)
    expect(screen.queryByRole('button', { name: /后台任务/ })).not.toBeInTheDocument()

    // 拿到面板注册到 nuphus-event 的 handler
    const call = vi.mocked(listen).mock.calls.find(c => c[0] === 'nuphus-event')
    expect(call).toBeTruthy()
    const handler = call![1] as (p: { seq: number; event: { type: string } }) => void

    // 后端登记了保留项 → 事件到达 → 数字刷新 → 入口出现
    stubLedger(1)
    await act(async () => {
      handler({ seq: 1, event: { type: 'background_tasks_changed' } })
    })
    expect(await screen.findByRole('button', { name: /后台任务/ })).toHaveTextContent('1')

    // 无关事件不得触发重拉（避免每条事件都打一次 IPC）
    const before = vi.mocked(countRetainedBackgroundTasks).mock.calls.length
    await act(async () => {
      handler({ seq: 2, event: { type: 'llm_text_delta' } })
    })
    expect(vi.mocked(countRetainedBackgroundTasks).mock.calls.length).toBe(before)
  })

  it('卸载时退订事件监听（不留悬挂订阅）', async () => {
    const unlisten = vi.fn()
    vi.mocked(listen).mockResolvedValueOnce(unlisten as unknown as () => void)
    const { unmount } = render(<BackgroundTasksPanel executionStage="idle" />)
    // 等 listen 的 promise 落定（取消函数已挂上）
    await act(async () => {
      await Promise.resolve()
    })
    unmount()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
