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
  vi.mocked(wfTraceList).mockReset().mockResolvedValue([run])
  vi.mocked(wfTraceRead).mockReset().mockResolvedValue(trace)
})

describe('ExecutionTraceViewer', () => {
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
    render(<ExecutionTraceViewer workflowId="flow" stepId="step" />)
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
