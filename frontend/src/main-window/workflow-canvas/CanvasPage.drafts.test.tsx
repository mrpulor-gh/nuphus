import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowIR } from './types'
import type { CanvasLeaveGuard } from './useCanvasLeaveGuard'
import { LangProvider, useLanguage } from '../../locales'

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
    onNodeClick,
    children,
  }: {
    nodes: { id: string }[]
    onNodeDoubleClick: (e: unknown, n: unknown) => void
    onNodeClick: (e: unknown, n: unknown) => void
    children: React.ReactNode
  }) => (
    <div>
      {nodes.map(node => (
        <span key={node.id}>
          <button onClick={e => onNodeDoubleClick(e, node)}>打开 {node.id}</button>
          <button onClick={e => onNodeClick(e, node)}>Select {node.id}</button>
        </span>
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
  getLanguage: async () => '',
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
vi.mock('./ProblemsPanel', () => ({
  ProblemsPanel: ({ backendReport }: { backendReport: unknown }) => (
    <output data-testid="backend-check">{JSON.stringify(backendReport)}</output>
  ),
}))
vi.mock('./OutlinePanel', () => ({ OutlinePanel: () => null }))
vi.mock('./IntentFormPanel', () => ({ IntentFormPanel: () => null }))
vi.mock('./ScopedEditDialog', () => ({
  ScopedEditDialog: ({
    selectedIds,
    steps,
    onApply,
    onClose,
  }: {
    selectedIds: string[]
    steps: WorkflowIR['steps']
    onApply: (steps: WorkflowIR['steps']) => void
    onClose: () => void
  }) => (
    <div role="dialog">
      Selected: {selectedIds.join(',')}
      <button
        onClick={() => {
          onApply(
            steps.map(step =>
              selectedIds.includes(step.id) ? { ...step, name: `Changed ${step.id}` } : step,
            ),
          )
          onClose()
        }}
      >
        Apply scoped proposal
      </button>
    </div>
  ),
}))
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
  it('groups primary and editing actions into separate rows and provides a localized new-node name', async () => {
    await open()
    const primary = document.querySelector('.wfc-toolbar-row--primary') as HTMLElement
    const actions = screen.getByLabelText('画布操作')
    expect(within(primary).getByRole('button', { name: '保存' })).toBeInTheDocument()
    expect(within(primary).getByRole('button', { name: '运行' })).toBeInTheDocument()
    expect(within(actions).getByRole('button', { name: '节点调试' })).toBeInTheDocument()
    expect(document.querySelector('.wfc-title')).toHaveAttribute(
      'title',
      expect.stringContaining(workflow.name),
    )
    fireEvent.click(within(actions).getByRole('button', { name: '添加' }))
    fireEvent.click(screen.getByRole('button', { name: '延时等待' }))
    expect(await screen.findByDisplayValue('等待 1 秒')).toBeInTheDocument()
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(
      mocks.save.mock.calls[0][0].steps.find((step: { name: string }) => step.name === '等待 1 秒'),
    ).toMatchObject({ do: { sleep: 1 } })
    expect(mocks.save.mock.calls[0][0].steps[0].name).toBe(workflow.steps[0].name)
  })
  it('clears a saved notice as soon as another field is edited', async () => {
    await open()
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '保存一次' } })
    shortcut()
    expect(await screen.findByText('已保存')).toBeInTheDocument()
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '继续编辑' } })
    await waitFor(() => expect(screen.queryByText('已保存')).not.toBeInTheDocument())
  })
  it('ignores validation responses for a definition edited while checking', async () => {
    let resolve!: (value: unknown) => void
    mocks.validate.mockReturnValue(
      new Promise(r => {
        resolve = r
      }),
    )
    await open()
    fireEvent.click(screen.getByText('更多'))
    fireEvent.click(screen.getByRole('button', { name: '检查' }))
    await waitFor(() => expect(mocks.validate).toHaveBeenCalledTimes(1))
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '检查后修改' } })
    await act(async () => {
      resolve({ passed: false, issues: [] })
    })
    expect(screen.queryByText('后端校验发现错误，详见问题面板')).not.toBeInTheDocument()
    expect(screen.getByTestId('backend-check')).toHaveTextContent('null')
  })
  it('invalidates an existing check as soon as a draft changes', async () => {
    mocks.validate.mockResolvedValue({ passed: false, issues: ['old failure'] })
    await open()
    fireEvent.click(screen.getByText('更多'))
    fireEvent.click(screen.getByRole('button', { name: '检查' }))
    await waitFor(() =>
      expect(screen.getByTestId('backend-check')).toHaveTextContent('old failure'),
    )
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '已修正' } })
    await waitFor(() => expect(screen.getByTestId('backend-check')).toHaveTextContent('null'))
  })
  it('applies selected-group AI edits as one undoable transaction', async () => {
    await open()
    fireEvent.click(screen.getByRole('button', { name: 'Select first' }))
    fireEvent.click(screen.getByRole('button', { name: 'Select second' }), { shiftKey: true })
    fireEvent.click(screen.getByRole('button', { name: 'AI 局部修改' }))
    expect(await screen.findByRole('dialog')).toHaveTextContent('Selected: first,second')
    fireEvent.click(screen.getByRole('button', { name: 'Apply scoped proposal' }))
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].steps.map((step: { name: string }) => step.name)).toEqual([
      'Changed first',
      'Changed second',
    ])
    fireEvent.keyDown(window, { key: 'z', ctrlKey: true })
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(2))
    expect(mocks.save.mock.calls[1][0].steps).toEqual(workflow.steps)
  })
  it('previews capture renaming and updates expressions together, including undo', async () => {
    const original = structuredClone(workflow.steps)
    workflow.steps[0].capture = 'result'
    workflow.steps[1].do = { tool: 'echo', with: { value: '{{result["title"]}}' } }
    try {
      await open()
      fireEvent.change(panel('first').getByRole('combobox', { name: '保存输出到变量' }), {
        target: { value: 'renamed' },
      })
      shortcut()
      expect(await screen.findByRole('dialog')).toHaveTextContent('{{renamed["title"]}}')
      expect(mocks.save).not.toHaveBeenCalled()
      fireEvent.click(screen.getByRole('button', { name: '一次应用全部修改' }))
      await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
      expect(mocks.save.mock.calls[0][0].steps[0].capture).toBe('renamed')
      expect(mocks.save.mock.calls[0][0].steps[1].do.with.value).toBe('{{renamed["title"]}}')
      fireEvent.keyDown(window, { key: 'z', ctrlKey: true })
      shortcut()
      await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(2))
      expect(mocks.save.mock.calls[1][0].steps).toEqual(workflow.steps)
    } finally {
      workflow.steps = original
    }
  })
  it('switches menu and inspector types without losing or saving uncommitted drafts', async () => {
    localStorage.setItem('nuphus_language', 'zh')
    function LanguageControl() {
      const { setLang } = useLanguage()
      return <button onClick={() => setLang('en')}>Switch English</button>
    }
    const view = render(
      <LangProvider>
        <LanguageControl />
        <CanvasPage workflowId="wf" onClose={() => {}} />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: '打开 first' }))
    fireEvent.change(panel('first').getByRole('combobox', { name: '保存输出到变量' }), {
      target: { value: 'unsaved_result' },
    })
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '用户草稿' } })
    fireEvent.click(screen.getByRole('button', { name: 'Switch English' }))
    expect(panel('first').getByText('Run script')).toBeInTheDocument()
    expect(panel('first').getByRole('combobox', { name: 'Save output to variable' })).toHaveValue(
      'unsaved_result',
    )
    expect(panel('first').getByLabelText(/^Name/)).toHaveValue('用户草稿')
    expect(mocks.save).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    expect(screen.getByRole('button', { name: 'Tool call' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Delay' })).toBeInTheDocument()
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].steps[0]).toMatchObject({
      name: '用户草稿',
      capture: 'unsaved_result',
      do: { script: { runtime: 'python' } },
    })
    view.unmount()
    localStorage.removeItem('nuphus_language')
  })
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
