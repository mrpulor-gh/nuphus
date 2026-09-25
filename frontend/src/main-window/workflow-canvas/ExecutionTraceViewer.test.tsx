import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ExecutionTraceViewer } from './ExecutionTraceViewer'
import { WorkflowTraceContext } from './WorkflowTraceContext'
import { VariablePicker } from './VariablePicker'
import {
  wfTraceList,
  wfTraceRead,
  type WorkflowInvocationTrace,
  type WorkflowRunTrace,
} from '../lib/api'

vi.mock('../lib/api', () => ({ wfTraceList: vi.fn(), wfTraceRead: vi.fn() }))

const trace: WorkflowInvocationTrace = {
  id: 7,
  parent_id: 2,
  workflow_id: 'flow',
  step_id: 'step',
  step_name: 'Repeated action',
  started_at: '2026-09-25T05:00:00Z',
  finished_at: '2026-09-25T05:00:01Z',
  status: 'success',
  definition: { id: 'step', capture: 'result' },
  variables_before: {},
  variables_after: {
    result: { 'a.b': [{ label: 'actual result' }] },
    outOfScope: { secret: 'hidden field' },
  },
  inputs: { query: 'actual input' },
  output: `${'complete output '.repeat(10000)}END OF FULL OUTPUT`,
  error: null,
  attempts: [{ attempt: 1, output: 'first attempt' }],
  verification: null,
}
const run: WorkflowRunTrace = {
  version: 1,
  run_id: 'past-run',
  workflow_id: 'flow',
  debug: false,
  revision: 'saved-definition-hash',
  started_at: trace.started_at,
  finished_at: trace.finished_at,
  status: 'success',
  invocations: [trace],
  storage_error: null,
}
const catalog = {
  references: [
    { name: 'result', source: 'capture' as const, sourceLabel: 'producer', maybeUnset: false },
  ],
  captures: [],
}

beforeEach(() => {
  vi.mocked(wfTraceList)
    .mockReset()
    .mockImplementation(async (_workflowId, debug) => (debug ? [] : [run]))
  vi.mocked(wfTraceRead).mockReset().mockResolvedValue(trace)
})

describe('ExecutionTraceViewer', () => {
  it('invalidates previously adoptable values while a different invocation is loading', async () => {
    vi.mocked(wfTraceList).mockImplementation(async (_id, debug) =>
      debug ? [] : [run, { ...run, run_id: 'other' }],
    )
    const selected = vi.fn()
    const unavailable = vi.fn()
    render(
      <ExecutionTraceViewer
        workflowId="flow"
        onInvocationSelected={selected}
        onSelectionUnavailable={unavailable}
      />,
    )
    await waitFor(() => expect(selected).toHaveBeenCalledWith(run, trace))
    unavailable.mockClear()
    vi.mocked(wfTraceRead).mockImplementation(() => new Promise(() => {}))
    fireEvent.change(screen.getByLabelText('运行'), { target: { value: 'other' } })
    await waitFor(() => expect(unavailable).toHaveBeenCalled())
    expect(selected).toHaveBeenCalledTimes(1)
  })
  it('loads full invocation data lazily and shows complete output with run and invocation provenance', async () => {
    render(<ExecutionTraceViewer workflowId="flow" stepId="step" />)
    const output = await screen.findByText(/END OF FULL OUTPUT/)
    expect(output.textContent).toBe(trace.output)
    expect(wfTraceRead).toHaveBeenCalledWith('flow', 'past-run', false, 7)
    expect(screen.getByText('saved-definition-hash')).toBeInTheDocument()
    expect(screen.getByText('flow/step')).toBeInTheDocument()
    expect(screen.getByText(/耗时: 1000 ms/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('tab', { name: '效果验证' }))
    expect(screen.getByText(/未记录效果验证/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('tab', { name: '实际输入' }))
    expect(screen.getByText(/actual input/)).toBeInTheDocument()
  })

  it('selects canonical fields only from actual values and supports changing normal/debug history', async () => {
    const select = vi.fn()
    render(<ExecutionTraceViewer workflowId="flow" catalog={catalog} onSelectReference={select} />)
    fireEvent.click(await screen.findByRole('button', { name: 'result', expanded: false }))
    fireEvent.click(screen.getByRole('button', { name: 'result["a.b"]', expanded: false }))
    fireEvent.click(screen.getByRole('button', { name: 'result["a.b"][0]', expanded: false }))
    fireEvent.click(screen.getByRole('button', { name: '插入引用: result["a.b"][0]["label"]' }))
    expect(select).toHaveBeenCalledWith('result["a.b"][0]["label"]')
    expect(screen.queryByRole('button', { name: /outOfScope/ })).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('运行类型'), { target: { value: 'true' } })
    await waitFor(() => expect(wfTraceList).toHaveBeenCalledWith('flow', true))
  })

  it('merges both partitions latest first and reads the selected record from its actual partition', async () => {
    const debugRun = {
      ...run,
      run_id: 'debug-run',
      debug: true,
      started_at: '2026-09-25T06:00:00Z',
    }
    vi.mocked(wfTraceList).mockImplementation(async (_workflowId, debug) =>
      debug ? [debugRun] : [run],
    )
    render(<ExecutionTraceViewer workflowId="flow" />)
    const picker = await screen.findByLabelText('运行')
    expect(screen.getByLabelText('运行类型')).toHaveValue('all')
    expect(picker).toHaveValue('debug-run')
    expect(picker.querySelectorAll('option')[0]).toHaveTextContent('调试运行')
    expect(picker.querySelectorAll('option')[1]).toHaveTextContent('正式运行')
    await waitFor(() => expect(wfTraceRead).toHaveBeenLastCalledWith('flow', 'debug-run', true, 7))
    fireEvent.change(picker, { target: { value: 'past-run' } })
    await waitFor(() => expect(wfTraceRead).toHaveBeenLastCalledWith('flow', 'past-run', false, 7))
    vi.mocked(wfTraceList).mockImplementation(async (_workflowId, debug) =>
      debug
        ? [{ ...debugRun, run_id: 'newer-debug-run', started_at: '2026-09-25T07:00:00Z' }, debugRun]
        : [run],
    )
    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    await waitFor(() => expect(wfTraceList).toHaveBeenCalledTimes(4))
    expect(screen.getByLabelText('运行')).toHaveValue('past-run')
  })

  it('does not silently substitute another run if an explicit selection is temporarily unavailable', async () => {
    const newer = { ...run, run_id: 'newer-run', started_at: '2026-09-25T07:00:00Z' }
    vi.mocked(wfTraceList).mockImplementation(async (_workflowId, debug) =>
      debug ? [] : [newer, run],
    )
    render(<ExecutionTraceViewer workflowId="flow" />)
    fireEvent.change(await screen.findByLabelText('运行'), { target: { value: 'past-run' } })
    await waitFor(() => expect(wfTraceRead).toHaveBeenLastCalledWith('flow', 'past-run', false, 7))
    vi.mocked(wfTraceList).mockImplementation(async (_workflowId, debug) => (debug ? [] : [newer]))
    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    await screen.findByText(/当前列表中未找到所选运行/)
    expect(screen.queryByText(/END OF FULL OUTPUT/)).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '查看最新运行' }))
    await waitFor(() => expect(wfTraceRead).toHaveBeenLastCalledWith('flow', 'newer-run', false, 7))
  })

  it('never falls back to another run while waiting for the pinned current run', async () => {
    const { rerender } = render(
      <ExecutionTraceViewer workflowId="flow" pinnedRunId="current-run" />,
    )
    await screen.findByText('正在等待本次运行的记录，可稍后刷新。')
    expect(wfTraceRead).not.toHaveBeenCalled()
    expect(screen.queryByLabelText('运行')).not.toBeInTheDocument()
    vi.mocked(wfTraceList).mockImplementation(async (_workflowId, debug) =>
      debug ? [{ ...run, run_id: 'current-run', debug: true }] : [run],
    )
    rerender(<ExecutionTraceViewer workflowId="flow" pinnedRunId="current-run" refreshKey={1} />)
    await waitFor(() => expect(wfTraceRead).toHaveBeenCalledWith('flow', 'current-run', true, 7))
    expect(screen.queryByLabelText('运行类型')).not.toBeInTheDocument()
  })

  it('keeps the successful partition visible and reports partial failure without an empty-history claim', async () => {
    vi.mocked(wfTraceList).mockImplementation(async (_workflowId, debug) => {
      if (debug) throw new Error('history unavailable')
      return [run]
    })
    render(<ExecutionTraceViewer workflowId="flow" />)
    await screen.findByText(/END OF FULL OUTPUT/)
    expect(screen.getByRole('alert')).toHaveTextContent('调试运行: Error: history unavailable')
    expect(screen.queryByText('暂无可查看的真实运行记录。')).not.toBeInTheDocument()
  })

  it('reports total loading failure without claiming no history exists', async () => {
    vi.mocked(wfTraceList).mockRejectedValue(new Error('offline'))
    render(<ExecutionTraceViewer workflowId="flow" />)
    await screen.findByRole('alert')
    expect(screen.queryByText('暂无可查看的真实运行记录。')).not.toBeInTheDocument()
  })

  it('ignores late responses after switching workflows', async () => {
    let resolveOld: (value: WorkflowInvocationTrace) => void = () => {}
    vi.mocked(wfTraceRead).mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveOld = resolve
        }),
    )
    const { rerender } = render(<ExecutionTraceViewer workflowId="flow" />)
    await waitFor(() => expect(wfTraceRead).toHaveBeenCalledTimes(1))
    vi.mocked(wfTraceList).mockResolvedValue([])
    rerender(<ExecutionTraceViewer workflowId="other-flow" />)
    await screen.findByText('暂无可查看的真实运行记录。')
    await act(async () => resolveOld(trace))
    expect(screen.queryByText(/END OF FULL OUTPUT/)).not.toBeInTheDocument()
  })

  it('separates repeated invocations with the same step ID and scopes matching by workflow', async () => {
    vi.mocked(wfTraceList).mockResolvedValue([
      {
        ...run,
        invocations: [
          { ...trace, id: 1 },
          { ...trace, id: 4, workflow_id: 'child-workflow' },
          trace,
        ],
      },
    ])
    render(<ExecutionTraceViewer workflowId="flow" stepId="step" debug={false} />)
    const picker = await screen.findByLabelText('步骤调用')
    expect(picker.querySelectorAll('option')).toHaveLength(2)
    fireEvent.change(picker, { target: { value: '1' } })
    await waitFor(() => expect(wfTraceRead).toHaveBeenLastCalledWith('flow', 'past-run', false, 1))
  })

  it('integrates historical selection into references and closes the dialog after insertion', async () => {
    const change = vi.fn()
    render(
      <WorkflowTraceContext.Provider value={{ workflowId: 'flow', refreshKey: 1 }}>
        <VariablePicker value="" onChange={change} mode="reference" catalog={catalog} />
      </WorkflowTraceContext.Provider>,
    )
    const trigger = screen.getByRole('button', { name: '从历史运行选择字段' })
    fireEvent.click(trigger)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    fireEvent.click(await screen.findByRole('button', { name: '插入引用: result' }))
    expect(change).toHaveBeenCalledWith('result')
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()
  })
})
