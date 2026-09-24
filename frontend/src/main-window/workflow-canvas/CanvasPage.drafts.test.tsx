import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowIR } from './types'
import type { CanvasLeaveGuard } from './useCanvasLeaveGuard'

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  validate: vi.fn(),
  run: vi.fn(),
  refresh: vi.fn(),
  rf: {
    fitView: vi.fn(),
    getViewport: () => ({ x: 0, y: 0, zoom: 1 }),
    getNodes: () => [],
    getZoom: () => 1,
    setCenter: vi.fn(),
    setViewport: vi.fn(),
    screenToFlowPosition: (p: unknown) => p,
  },
}))
const workflow: WorkflowIR = {
  id: 'wf',
  name: '手工编辑测试',
  status: 'Draft',
  steps: [
    { id: 'first', name: '第一步', do: { script: { runtime: 'python', code: 'print(1)' } } },
    { id: 'second', name: '第二步', do: { tool: 'unknown', with: { existing: true } } },
  ],
}
vi.mock('@xyflow/react', () => ({
  ReactFlowProvider: ({ children }: { children: React.ReactNode }) => children,
  ReactFlow: ({
    nodes,
    onNodeDoubleClick,
    children,
  }: {
    nodes: { id: string }[]
    onNodeDoubleClick: (e: unknown, n: unknown) => void
    children: React.ReactNode
  }) => (
    <div>
      {nodes.map(node => (
        <button key={node.id} onClick={e => onNodeDoubleClick(e, node)}>
          打开 {node.id}
        </button>
      ))}
      {children}
    </div>
  ),
  Background: () => null,
  Controls: () => null,
  BackgroundVariant: { Dots: 'dots' },
  MarkerType: { ArrowClosed: 'arrow' },
  useReactFlow: () => mocks.rf,
  applyNodeChanges: (_: unknown, nodes: unknown) => nodes,
}))
vi.mock('../lib/api', () => ({
  wfGetRaw: async () => structuredClone(workflow),
  wfLayoutGet: async () => null,
  wfLayoutSave: async () => null,
  wfSave: (...args: unknown[]) => mocks.save(...args),
  wfValidate: (...args: unknown[]) => mocks.validate(...args),
  wfRun: (...args: unknown[]) => mocks.run(...args),
  wfTools: async () => [],
  listModels: async () => [],
  wfScheduleHistoryGet: async () => null,
}))
vi.mock('../lib/useWorkflowGate', () => ({
  useWorkflowGate: () => ({ locked: false, refresh: mocks.refresh }),
}))
vi.mock('./runStatus', () => ({
  subscribeRunStatus: () => () => {},
  aggregateContainerBadges: () => new Map(),
}))
vi.mock('./ToolPalette', () => ({ ToolPalette: () => null, TOOL_DRAG_MIME: 'tool' }))
vi.mock('./ProblemsPanel', () => ({ ProblemsPanel: () => null }))
vi.mock('./OutlinePanel', () => ({ OutlinePanel: () => null }))
vi.mock('./IntentFormPanel', () => ({ IntentFormPanel: () => null }))
vi.mock('./EnhancedModeToggle', () => ({ EnhancedModeToggle: () => null }))
vi.mock('./WorkflowSwitcher', () => ({ WorkflowSwitcher: () => null }))
vi.mock('./WorkflowInputsEditor', () => ({
  WorkflowInputsEditor: ({
    open,
    onApply,
  }: {
    open: boolean
    onApply: (inputs: unknown[]) => void
  }) =>
    open ? (
      <button onClick={() => onApply([{ name: 'new_input', type: 'string' }])}>应用测试输入</button>
    ) : null,
}))
vi.mock('../workflow/WorkflowScheduleDialog', () => ({ WorkflowScheduleDialog: () => null }))
vi.mock('../workflow/WorkflowInputsForm', () => ({
  WorkflowInputsDialog: () => null,
  NO_INPUT_SPECS: [],
}))
import { CanvasPage } from './CanvasPage'

const panel = (id: string) =>
  within(document.querySelector(`[data-inspector-node="${id}"]`) as HTMLElement)
beforeEach(() => {
  vi.clearAllMocks()
  mocks.refresh.mockResolvedValue({ locked: false })
  mocks.save.mockResolvedValue({ saved: true, report: { passed: true, issues: [] } })
})
async function open(id = 'first') {
  render(<CanvasPage workflowId="wf" onClose={() => {}} />)
  fireEvent.click(await screen.findByRole('button', { name: `打开 ${id}` }))
}
function shortcut() {
  fireEvent.keyDown(window, { key: 's', ctrlKey: true })
}

describe('Canvas save coordination', () => {
  it('saves inputs edited while awaiting the runtime gate from the latest IR', async () => {
    let resolve!: (value: unknown) => void
    mocks.refresh.mockReturnValue(
      new Promise(r => {
        resolve = r
      }),
    )
    await open()
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '同时修改' } })
    shortcut()
    await waitFor(() => expect(mocks.refresh).toHaveBeenCalled())
    fireEvent.click(screen.getByRole('button', { name: '外部输入' }))
    fireEvent.click(screen.getByRole('button', { name: '应用测试输入' }))
    await act(async () => {
      resolve({ locked: false })
    })
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].inputs).toEqual([{ name: 'new_input', type: 'string' }])
  })
  it('serializes repeated save shortcuts while draft flushing is pending', async () => {
    await open()
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '新名称' } })
    act(() => {
      shortcut()
      shortcut()
      shortcut()
    })
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
  })
  it('retains all pending fields when the step ID changes before save', async () => {
    await open()
    fireEvent.change(panel('first').getByLabelText(/^步骤标识 ID/), {
      target: { value: 'renamed' },
    })
    fireEvent.change(panel('first').getByDisplayValue('print(1)'), {
      target: { value: 'print(3)' },
    })
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].steps[0]).toMatchObject({
      id: 'renamed',
      do: { script: { code: 'print(3)' } },
    })
  })
  it('registers one leave confirmation for unfinished drafts', async () => {
    let guard: CanvasLeaveGuard | null = null
    render(
      <CanvasPage
        workflowId="wf"
        onClose={() => {}}
        registerLeaveGuard={next => {
          guard = next
        }}
      />,
    )
    fireEvent.click(await screen.findByRole('button', { name: '打开 first' }))
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '没保存' } })
    let result!: Promise<boolean>
    act(() => {
      result = guard!()
    })
    expect(screen.getByText(/离开将丢弃/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await expect(result).resolves.toBe(false)
  })
  it('Ctrl+S flushes unblurred text and preserves multiple field patches', async () => {
    await open()
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '新名称' } })
    fireEvent.change(panel('first').getByDisplayValue('print(1)'), {
      target: { value: 'print(2)' },
    })
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].steps[0]).toMatchObject({
      name: '新名称',
      do: { script: { code: 'print(2)' } },
    })
  })
  it('keeps an invalid draft on another node and refuses to save stale JSON', async () => {
    await open('second')
    fireEvent.change(panel('second').getByRole('textbox', { name: /参数 JSON/ }), {
      target: { value: '{broken' },
    })
    fireEvent.click(screen.getByRole('button', { name: '打开 first' }))
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '保留草稿' } })
    shortcut()
    await screen.findByText(/请先修正/)
    expect(mocks.save).not.toHaveBeenCalled()
    expect(panel('second').getByRole('textbox', { name: /参数 JSON/ })).toHaveValue('{broken')
  })
  it('does not clear dirty when edits arrive while saving', async () => {
    let resolve!: (value: unknown) => void
    mocks.save.mockReturnValue(
      new Promise(r => {
        resolve = r
      }),
    )
    await open()
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '已提交' } })
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '还没保存' } })
    await act(async () => {
      resolve({ saved: true, report: { passed: true, issues: [] } })
    })
    expect(screen.getByText(/后续编辑尚未保存/)).toBeInTheDocument()
    expect(panel('first').getByLabelText(/^名称/)).toHaveValue('还没保存')
    expect(screen.getByRole('button', { name: /保存/ })).not.toBeDisabled()
  })
})
