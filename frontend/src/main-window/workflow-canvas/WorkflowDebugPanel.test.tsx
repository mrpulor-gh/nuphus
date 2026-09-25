import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../../locales'
import type { WorkflowStep } from '../../core/types'
import { WorkflowDebugPanel } from './WorkflowDebugPanel'
import { parseTestValues } from './debugSession'

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), runs: vi.fn() }))
vi.mock('../../core/bridge', () => ({ invoke: (...args: unknown[]) => mocks.invoke(...args) }))
vi.mock('../lib/api', () => ({
  getLanguage: async () => '',
  wfTraceList: (...args: unknown[]) => mocks.runs(...args),
}))
vi.mock('./ExecutionTraceViewer', () => ({
  ExecutionTraceViewer: ({
    onInvocationSelected,
  }: {
    onInvocationSelected?: (run: unknown, invocation: unknown) => void
  }) => (
    <button
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
    expect(await screen.findByRole('alert')).toHaveTextContent('JSON object required')
    expect(mocks.invoke).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('Test variables (JSON object)'), {
      target: { value: '{}' },
    })
    fireEvent.change(screen.getByLabelText('Execution scope'), { target: { value: 'through' } })
    expect(screen.getByRole('dialog')).toHaveTextContent('may repeat sends or writes')
    fireEvent.click(screen.getByRole('button', { name: 'Run through selected node' }))
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
})
