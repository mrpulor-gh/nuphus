import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { Inspector } from './Inspector'
import { InspectorDraftContext, InspectorDraftStore } from './inspectorDrafts'
import { WorkflowTraceContext } from './WorkflowTraceContext'

vi.mock('../lib/api', () => ({
  wfTools: vi.fn(async () => [
    {
      name: 'desktop_test',
      description: '测试工具',
      input_schema: {
        type: 'object',
        properties: { message: { type: 'string', description: '消息内容' } },
      },
    },
  ]),
  listModels: vi.fn(async () => []),
  wfTraceList: vi.fn(async () => [
    {
      run_id: 'run',
      workflow_id: 'flow',
      debug: false,
      revision: 'revision',
      started_at: 'now',
      status: 'success',
      invocations: [
        {
          id: 1,
          workflow_id: 'flow',
          step_id: 'producer',
          step_name: 'producer',
          status: 'success',
        },
      ],
    },
  ]),
  wfTraceRead: vi.fn(async () => ({
    id: 1,
    workflow_id: 'flow',
    step_id: 'producer',
    status: 'success',
    output: '',
    error: null,
    variables_before: {},
    variables_after: { previous: { 'a.b': [4] } },
    verification: null,
    attempts: [],
  })),
}))

function mount(step: WorkflowStep) {
  const store = new InspectorDraftStore()
  let current = step
  const patched = vi.fn()
  function Harness() {
    const [value, setValue] = useState(step)
    current = value
    return (
      <WorkflowTraceContext.Provider value={{ workflowId: 'flow' }}>
        <InspectorDraftContext.Provider value={{ store, nodeId: step.id }}>
          <Inspector
            step={value}
            readOnly={false}
            idReferenced={false}
            onClose={vi.fn()}
            onPatch={patch => {
              patched(patch)
              setValue(old => ({ ...old, ...patch }))
            }}
            onPatchAction={action => setValue(old => ({ ...old, do: action }))}
            variableCatalog={{
              references: [
                { name: 'previous', source: 'capture', sourceLabel: '前一步', maybeUnset: false },
              ],
              captures: [],
            }}
          />
        </InspectorDraftContext.Provider>
      </WorkflowTraceContext.Provider>
    )
  }
  render(<Harness />)
  return { store, current: () => current, patched }
}

describe('Inspector manual workflow editing', () => {
  it('flushes typed values before blur and preserves invalid numeric drafts', async () => {
    const { store, current } = mount({ id: 'sleep', name: '暂停', do: { sleep: 1 } })
    fireEvent.change(screen.getByLabelText('名称（必填）'), { target: { value: '等待应用' } })
    expect(store.dirty).toBe(true)
    await act(async () => {
      expect(await store.flush()).toBeNull()
    })
    expect(current().name).toBe('等待应用')
    fireEvent.change(screen.getByLabelText('时长（秒）'), { target: { value: 'abc' } })
    await act(async () => {
      expect((await store.flush())?.field).toBe('/do/sleep')
    })
    expect(current().do).toEqual({ sleep: 1 })
    expect(screen.getByText('请输入有效数值')).toBeInTheDocument()
  })

  it('uses actual runtime names and preserves unsupported historical values', async () => {
    mount({ id: 'script', name: '脚本', do: { script: { runtime: 'shell', code: 'echo hi' } } })
    expect(screen.getByRole('option', { name: /shell（当前执行器不支持/ })).toBeInTheDocument()
    expect(screen.getByRole('option', { name: /AutoHotkey/ })).toHaveValue('ahk')
    expect(screen.getByRole('option', { name: /PowerShell/ })).toHaveValue('pwsh')
    expect(screen.getByText(/失败后怎么办/)).toBeInTheDocument()
    await waitFor(() => expect(screen.getByLabelText('运行环境 runtime')).toHaveValue('shell'))
  })

  it('shows a false fixed condition accurately and retains operands on compatible changes', async () => {
    const { current } = mount({
      id: 'if',
      name: '条件',
      do: { if: { condition: { always: false }, then: [] } },
    })
    expect(screen.getByLabelText('固定结果')).toHaveValue('false')
    fireEvent.change(screen.getByLabelText('条件'), { target: { value: 'equals' } })
    const inputs = screen.getAllByPlaceholderText('字面量值')
    fireEvent.change(inputs[0], { target: { value: 'hello' } })
    fireEvent.blur(inputs[0])
    fireEvent.change(inputs[1], { target: { value: 'world' } })
    fireEvent.blur(inputs[1])
    await waitFor(() =>
      expect(current().do).toMatchObject({ if: { condition: { equals: ['hello', 'world'] } } }),
    )
    fireEvent.change(screen.getByLabelText('条件'), { target: { value: 'contains' } })
    expect(current().do).toMatchObject({ if: { condition: { contains: ['hello', 'world'] } } })
  })

  it('retains loop mode drafts and nested steps when changing modes', () => {
    const child = { id: 'child', name: '子步骤', do: { sleep: 1 } }
    const { current } = mount({
      id: 'loop',
      name: '循环',
      do: { loop: { repeat: 7, max: 9, do: [child] } },
    })
    fireEvent.change(screen.getByLabelText('循环方式'), { target: { value: 'until' } })
    expect(screen.getByLabelText('固定结果')).toHaveValue('false')
    fireEvent.change(screen.getByLabelText('循环方式'), { target: { value: 'repeat' } })
    expect(current().do).toMatchObject({ loop: { repeat: 7, max: 9, do: [child] } })
  })

  it('opens the tool list on click in a portal and supports keyboard choice', async () => {
    const { current } = mount({ id: 'tool', name: '工具', do: { tool: '' } })
    const input = screen.getByRole('combobox', { name: '工具名' })
    fireEvent.click(input)
    const option = await screen.findByRole('option', { name: /desktop_test/ })
    expect(option.closest('aside')).toBeNull()
    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(current().do).toEqual({ tool: 'desktop_test' }))
  })

  it('does not commit while IME composing Enter', () => {
    const { patched } = mount({ id: 'sleep', name: '暂停', do: { sleep: 1 } })
    const input = screen.getByLabelText('名称（必填）')
    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: '输入中' } })
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    expect(patched).not.toHaveBeenCalled()
  })

  it('commits a selected historical field as a typed loop variable reference', async () => {
    const { store, current } = mount({
      id: 'loop',
      name: 'loop',
      do: { loop: { for_each: { items: { var: '' }, as: 'item' }, do: [] } },
    })
    fireEvent.click(screen.getByRole('button', { name: '从历史运行选择字段' }))
    fireEvent.click(await screen.findByRole('button', { name: 'previous', expanded: false }))
    fireEvent.click(screen.getByRole('button', { name: '插入引用: previous["a.b"]' }))
    await act(async () => {
      expect(await store.flush()).toBeNull()
    })
    expect(current().do).toMatchObject({
      loop: { for_each: { items: { var: 'previous["a.b"]' } } },
    })
    expect(screen.queryByText(/未找到来源/)).not.toBeInTheDocument()
  })

  it('inserts historical fields into text templates without losing the surrounding draft', async () => {
    const { store, current } = mount({
      id: 'script',
      name: 'script',
      do: { script: { runtime: 'pwsh', code: 'prefix ' } },
    })
    const code = screen.getByRole('textbox', { name: /脚本代码 code/ }) as HTMLTextAreaElement
    code.setSelectionRange(code.value.length, code.value.length)
    fireEvent.click(screen.getByRole('button', { name: '从历史运行选择字段' }))
    fireEvent.click(await screen.findByRole('button', { name: 'previous', expanded: false }))
    fireEvent.click(screen.getByRole('button', { name: '插入引用: previous["a.b"]' }))
    await act(async () => {
      expect(await store.flush()).toBeNull()
    })
    expect(current().do).toMatchObject({ script: { code: 'prefix {{previous["a.b"]}}' } })
  })
})
