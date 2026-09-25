import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../../locales'
import type { WorkflowStep } from '../../core/types'
import { WorkflowDebugPanel } from './WorkflowDebugPanel'
import { debugPreflight, parseTestValues } from './debugSession'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  runs: vi.fn(),
  read: vi.fn(),
  validate: vi.fn(),
}))
vi.mock('../../core/bridge', () => ({ invoke: (...args: unknown[]) => mocks.invoke(...args) }))
vi.mock('../lib/api', () => ({
  getLanguage: async () => '',
  wfTraceList: (...args: unknown[]) => mocks.runs(...args),
  wfTraceRead: (...args: unknown[]) => mocks.read(...args),
  wfValidate: (...args: unknown[]) => mocks.validate(...args),
}))
vi.mock('./ExecutionTraceViewer', () => ({
  ExecutionTraceViewer: ({
    onInvocationSelected,
    pinnedRunId,
  }: {
    onInvocationSelected?: (run: unknown, invocation: unknown) => void
    pinnedRunId?: string
  }) => (
    <button
      data-testid={pinnedRunId ? `trace-${pinnedRunId}` : 'history-picker'}
      onClick={() =>
        onInvocationSelected?.(
          { run_id: 'history', revision: 'v1' },
          {
            id: 7,
            variables_before: { wn: { window_id: 123 }, inputs: { name: 'Test' } },
            variables_after: { wn: { window_id: 456 } },
          },
        )
      }
    >
      Select evidence
    </button>
  ),
}))
const steps: WorkflowStep[] = [
  { id: 'before', name: 'Send message', do: { tool: 'send', with: {} } },
  { id: 'target', name: 'Selected', do: { tool: 'read', with: { id: '{{wn["window_id"]}}' } } },
  { id: 'after', name: 'Write file', do: { tool: 'write', with: {} } },
]
const props = {
  workflowId: 'wf',
  steps,
  inputs: [],
  selected: steps[1],
  blocked: false,
  runId: null,
  onRunStarted: vi.fn(),
  onClose: vi.fn(),
}
beforeEach(() => {
  vi.clearAllMocks()
  localStorage.setItem('nuphus_language', 'en')
  mocks.invoke.mockResolvedValue({ run_id: 'debug-run' })
  mocks.runs.mockResolvedValue([])
  mocks.read.mockResolvedValue({ output: 'slept 1s', error: null })
  mocks.validate.mockResolvedValue({ passed: true, errors: [], warnings: [] })
})
const mount = (overrides = {}) =>
  render(
    <LangProvider>
      <WorkflowDebugPanel {...props} {...overrides} />
    </LangProvider>,
  )

describe('node debug controls', () => {
  it('uses the debug command with explicit scope and manual values, without implicit retry', async () => {
    mount()
    expect(screen.getByRole('dialog')).not.toHaveTextContent(/[\u4e00-\u9fff]/)
    fireEvent.change(screen.getByLabelText('Test variables (JSON object)'), {
      target: { value: '{"wn":{"window_id":123}}' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Test selected node' }))
    await waitFor(() => expect(props.onRunStarted).toHaveBeenCalledWith('debug-run'))
    expect(mocks.invoke).toHaveBeenCalledTimes(1)
    expect(mocks.invoke).toHaveBeenCalledWith('wf_debug_run', {
      request: expect.objectContaining({
        selected_step_id: 'target',
        mode: 'node',
        variables: { wn: { window_id: 123 } },
        use_retry_policy: false,
      }),
    })
  })
  it('requires valid JSON objects and exposes inclusive run-through semantics', async () => {
    mount()
    fireEvent.change(screen.getByLabelText('Test variables (JSON object)'), {
      target: { value: '[]' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Test selected node' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Test variables must be a JSON object',
    )
    expect(screen.getByLabelText('Test variables (JSON object)')).toHaveFocus()
    expect(mocks.invoke).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('Test variables (JSON object)'), {
      target: { value: '{}' },
    })
    fireEvent.change(screen.getByLabelText('Execution scope'), { target: { value: 'through' } })
    expect(screen.getByRole('dialog')).toHaveTextContent('may repeat sends or writes')
    fireEvent.click(screen.getByRole('button', { name: 'Restart from beginning' }))
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith('wf_debug_run', {
        request: expect.objectContaining({ mode: 'through' }),
      }),
    )
  })
  it('copies history only after an explicit choice and labels edited provenance', async () => {
    mount()
    fireEvent.click(screen.getByText('Choose existing execution data'))
    fireEvent.click(await screen.findByRole('button', { name: 'Select evidence' }))
    expect(screen.getByLabelText('Test variables (JSON object)')).toHaveValue('{}')
    fireEvent.click(screen.getByRole('button', { name: 'Use variables before this invocation' }))
    expect(
      JSON.parse(
        (screen.getByLabelText('Test variables (JSON object)') as HTMLTextAreaElement).value,
      ),
    ).toEqual({ wn: { window_id: 123 } })
    fireEvent.change(screen.getByLabelText('Workflow inputs (JSON object)'), {
      target: { value: '{"name":"Changed"}' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Test selected node' }))
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith('wf_debug_run', {
        request: expect.objectContaining({
          source: {
            kind: 'history',
            run_id: 'history',
            invocation_id: 7,
            revision: 'v1',
            phase: 'before',
            modified: true,
          },
        }),
      }),
    )
  })
  it('scopes resume and stop to the active run and does not rerun the node', async () => {
    mocks.runs.mockResolvedValue([{ run_id: 'debug-run', status: 'paused' }])
    mount({ runId: 'debug-run', blocked: true })
    fireEvent.click(await screen.findByRole('button', { name: 'Continue remaining steps' }))
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith('wf_debug_control', {
        workflowId: 'wf',
        runId: 'debug-run',
        action: 'resume',
      }),
    )
    expect(mocks.invoke).not.toHaveBeenCalledWith('wf_debug_run', expect.anything())
  })
  it('preserves JSON nulls, arrays and string values without guessing types', () => {
    expect(parseTestValues('{"n":null,"s":"003","a":[1]}')).toEqual({ n: null, s: '003', a: [1] })
    expect(() => parseTestValues('null')).toThrow()
  })
  it('keeps the complete form visible and focuses invalid workflow inputs', async () => {
    mount()
    expect(screen.getByLabelText('Execution scope')).toBeVisible()
    expect(screen.getByLabelText('Test variables (JSON object)')).toBeVisible()
    expect(screen.getByLabelText('Workflow inputs (JSON object)')).toBeVisible()
    expect(screen.getByLabelText(/Use node retry policies/)).toBeVisible()
    fireEvent.change(screen.getByLabelText('Workflow inputs (JSON object)'), {
      target: { value: 'null' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Test selected node' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Workflow inputs must be a JSON object',
    )
    expect(screen.getByLabelText('Workflow inputs (JSON object)')).toHaveFocus()
    expect(mocks.invoke).not.toHaveBeenCalled()
  })
  it('puts completion evidence before the full form and pins details to the exact debug run', async () => {
    mocks.runs.mockResolvedValue([
      { run_id: 'unrelated-newest', status: 'error', invocations: [] },
      {
        run_id: 'debug-run',
        status: 'success',
        started_at: '2026-09-25T10:00:00Z',
        finished_at: '2026-09-25T10:00:01.004Z',
        invocations: [{ id: 1, step_id: 'target', step_name: 'Selected' }],
      },
    ])
    mount({ runId: 'debug-run' })
    const summary = screen.getByRole('region', { name: 'Current debug summary' })
    expect(await within(summary).findByText(/slept 1s/)).toBeVisible()
    expect(within(summary).getByRole('status')).toHaveTextContent('Completed')
    expect(summary).toHaveTextContent('1004 ms')
    expect(
      summary.compareDocumentPosition(screen.getByLabelText('Test variables (JSON object)')) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy()
    expect(screen.getByTestId('trace-debug-run')).toBeInTheDocument()
    expect(screen.queryByTestId('trace-unrelated-newest')).not.toBeInTheDocument()
    expect(mocks.read).toHaveBeenCalledWith('wf', 'debug-run', true, 1)
    expect(screen.getByRole('button', { name: 'Retry selected node' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Close panel' })).toBeEnabled()
    expect(
      screen.queryByRole('button', { name: 'Close panel (keep running)' }),
    ).not.toBeInTheDocument()
  })
  it('does not display an earlier success as the result of a failed restart', async () => {
    mocks.runs.mockResolvedValue([
      { run_id: 'debug-run', status: 'success', invocations: [{ id: 1, step_id: 'target' }] },
    ])
    mocks.invoke.mockRejectedValue(new Error('IPC invoke wf_debug_run failed: runtime unavailable'))
    mount({ runId: 'debug-run' })
    expect(await screen.findByText(/slept 1s/)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Retry selected node' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('This debug run could not start')
    expect(screen.queryByText(/slept 1s/)).not.toBeInTheDocument()
    expect(screen.queryByTestId('trace-debug-run')).not.toBeInTheDocument()
    const alert = screen.getByRole('alert')
    expect(alert.querySelector('details')).not.toHaveAttribute('open')
    expect(
      within(screen.getByRole('region', { name: 'Current debug summary' })).getByRole('status'),
    ).toHaveTextContent('Not started')
    expect(mocks.invoke).toHaveBeenCalledTimes(1)
  })
  it('offers a precise edit target for invalid node settings without executing', async () => {
    const onLocateIssue = vi.fn()
    const invalid = { id: 'target', name: 'Wait', do: { sleep: 0 } }
    mount({ selected: invalid, steps: [invalid], onLocateIssue })
    fireEvent.click(screen.getByRole('button', { name: 'Test selected node' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(/duration|greater than|positive/i)
    fireEvent.click(screen.getByRole('button', { name: 'Edit node settings' }))
    expect(onLocateIssue).toHaveBeenCalledWith('target', '/do/sleep')
    expect(mocks.invoke).not.toHaveBeenCalled()
  })
  it('preflights only the supplied execution scope and does not treat historical variables as errors', () => {
    expect(debugPreflight([steps[1]])).toBeUndefined()
    expect(debugPreflight([{ id: 'sleep', name: 'Wait', do: { sleep: 0 } }])).toMatchObject({
      stepId: 'sleep',
      fieldPath: '/do/sleep',
    })
    expect(debugPreflight([{ id: 'sleep', name: '', do: { sleep: 1 } }])).toMatchObject({
      stepId: 'sleep',
      fieldPath: '/name',
    })
  })
  it('uses structured validation for tool parameters and links to the exact field', async () => {
    const onLocateIssue = vi.fn()
    mocks.validate.mockResolvedValue({
      passed: false,
      errors: ['missing path'],
      warnings: [],
      diagnostics: [
        {
          code: 'required',
          severity: 'error',
          category: 'missing',
          step_id: 'target',
          field_path: '/do/with/path',
          detail: 'missing path',
        },
      ],
    })
    mount({ onLocateIssue })
    fireEvent.click(screen.getByRole('button', { name: 'Test selected node' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(/required/i)
    expect(mocks.validate).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'wf', status: 'Draft', steps: [steps[1]], inputs: [] }),
    )
    expect(mocks.invoke).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Edit node settings' }))
    expect(onLocateIssue).toHaveBeenCalledWith('target', '/do/with/path')
  })
  it('does not block debug data on warnings and does not re-label a completed run when scope changes', async () => {
    mocks.validate.mockResolvedValue({
      passed: true,
      errors: [],
      warnings: ['external variable'],
      diagnostics: [
        {
          code: 'variable',
          severity: 'warning',
          category: 'variable',
          detail: 'external variable',
        },
      ],
    })
    mocks.runs.mockResolvedValue([
      { run_id: 'debug-run', status: 'success', source: { mode: 'through' }, invocations: [] },
    ])
    mount({ runId: 'debug-run' })
    const summary = screen.getByRole('region', { name: 'Current debug summary' })
    await waitFor(() => expect(summary).toHaveTextContent('From start through selected node'))
    fireEvent.change(screen.getByLabelText('Execution scope'), { target: { value: 'node' } })
    expect(summary).toHaveTextContent('From start through selected node')
    fireEvent.click(screen.getByRole('button', { name: 'Retry selected node' }))
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith('wf_debug_run', expect.anything()),
    )
  })
})
