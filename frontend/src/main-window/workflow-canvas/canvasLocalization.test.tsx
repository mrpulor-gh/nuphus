import { fireEvent, render, screen, within } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { NodeProps } from '@xyflow/react'
import { LangProvider, useLanguage } from '../../locales'
import { VariablePicker } from './VariablePicker'
import { NodeKindBadge } from './NodeKindBadge'
import { StepNode, type StepNodeFlow } from './nodes/StepNode'
import { ContainerNode, type ContainerNodeFlow } from './nodes/ContainerNode'
import { buildVariableCatalog } from './variableCatalog'

vi.mock('../lib/api', () => ({ getLanguage: async () => '' }))
vi.mock('@xyflow/react', async importOriginal => ({
  ...(await importOriginal<object>()),
  Handle: () => <span data-testid="handle" />,
}))

function SwitchLanguage() {
  const { setLang } = useLanguage()
  return (
    <>
      <button onClick={() => setLang('en')}>English</button>
      <button onClick={() => setLang('zh')}>中文</button>
    </>
  )
}
beforeEach(() => localStorage.setItem('nuphus_language', 'zh'))
afterEach(() => localStorage.removeItem('nuphus_language'))

describe('localized workflow UI', () => {
  it('switches an open portal in place, keeps user text and never displays a variable type', () => {
    const catalog = buildVariableCatalog(
      [
        {
          id: 'launch',
          name: '[唤起微信] 后台/托盘态唤起主窗口 $&',
          capture: 'wn',
          do: { tool: 'launch' },
        },
        { id: 'read', name: 'read', do: { sleep: 1 } },
      ],
      [{ name: 'topic' }],
      'read',
    )
    const onChange = vi.fn()
    render(
      <LangProvider>
        <SwitchLanguage />
        <VariablePicker value="wn" onChange={onChange} mode="reference" catalog={catalog} />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('combobox'))
    expect(screen.getAllByText('变量名:')).toHaveLength(2)
    fireEvent.click(screen.getByRole('button', { name: 'English' }))
    const list = screen.getByRole('listbox', { name: 'Variable suggestions' })
    expect(within(list).getAllByText('Variable:')).toHaveLength(2)
    expect(within(list).getAllByText('Source:')).toHaveLength(2)
    expect(within(list).getByText('Workflow input · topic')).toBeInTheDocument()
    expect(within(list).getByText('[唤起微信] 后台/托盘态唤起主窗口 $&')).toBeInTheDocument()
    expect(list.textContent).not.toMatch(/类型|Type|运行时确定/)
    expect(screen.getByRole('combobox')).toHaveValue('wn')
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.click(within(list).getByRole('option', { name: /inputs.topic/ }))
    expect(onChange).toHaveBeenCalledWith('inputs.topic')
  })

  it('translates new-variable, missing-source, update and conditional messages', () => {
    localStorage.setItem('nuphus_language', 'en')
    const catalog = buildVariableCatalog(
      [
        {
          id: 'launch',
          name: 'Keep $& name',
          capture: 'wn',
          on_error: 'skip',
          do: { tool: 'launch' },
        },
        { id: 'read', name: 'read', do: { sleep: 1 } },
      ],
      [],
      'read',
    )
    const { rerender } = render(
      <LangProvider>
        <VariablePicker value="new_result" onChange={vi.fn()} mode="capture" catalog={catalog} />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('combobox'))
    expect(screen.getByRole('option', { name: '+ Create variable new_result' })).toBeInTheDocument()
    rerender(
      <LangProvider>
        <VariablePicker value="wn" onChange={vi.fn()} mode="capture" catalog={catalog} />
      </LangProvider>,
    )
    expect(screen.getByText('Will update the existing variable · Keep $& name')).toBeInTheDocument()
    rerender(
      <LangProvider>
        <VariablePicker value="wn" onChange={vi.fn()} mode="reference" catalog={catalog} />
      </LangProvider>,
    )
    expect(
      screen.getByText('May be unset. Check branches, loops or input defaults.'),
    ).toBeInTheDocument()
    rerender(
      <LangProvider>
        <VariablePicker
          value="inputs.unknown"
          onChange={vi.fn()}
          mode="reference"
          catalog={catalog}
          onConfigureInput={vi.fn()}
        />
      </LangProvider>,
    )
    expect(screen.getByRole('button', { name: 'Configure workflow input' })).toBeInTheDocument()
    expect(
      screen.getByText('Source not found; the manual expression is preserved.'),
    ).toBeInTheDocument()
  })

  it('updates leaf, container and preview labels without changing names or draft values', () => {
    const step = {
      data: {
        canvas: {
          id: 'tool',
          name: '用户节点名称',
          kind: 'tool',
          category: 'leaf',
          lane: 'main',
          capture: 'wn',
        },
      },
    } as NodeProps<StepNodeFlow>
    const container = {
      data: {
        canvas: {
          id: 'seq',
          name: '用户容器名称',
          kind: 'seq',
          category: 'container',
          lane: 'main',
          childCount: 1,
        },
        childrenPreview: { total: 1, items: [{ name: '等待窗口', kind: 'sleep' }] },
      },
    } as NodeProps<ContainerNodeFlow>
    function Draft() {
      const [value, setValue] = useState('')
      return (
        <VariablePicker
          mode="capture"
          catalog={{ references: [], captures: [] }}
          value={value}
          onChange={setValue}
        />
      )
    }
    render(
      <LangProvider>
        <SwitchLanguage />
        <StepNode {...step} />
        <ContainerNode {...container} />
        <NodeKindBadge kind="wait" />
        <Draft />
      </LangProvider>,
    )
    expect(screen.getByText('工具调用')).toBeInTheDocument()
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'unsaved_name' } })
    fireEvent.click(screen.getByRole('button', { name: 'English' }))
    expect(screen.getByText('Tool call')).toHaveAttribute(
      'title',
      expect.stringContaining('(tool)'),
    )
    expect(screen.getByText('Sequence')).toBeInTheDocument()
    expect(screen.getByText('Delay')).toBeInTheDocument()
    expect(screen.getByText('Confirm')).toBeInTheDocument()
    expect(screen.getByText('用户节点名称')).toBeInTheDocument()
    expect(screen.getByTitle('Save output to wn')).toBeInTheDocument()
    expect(screen.getByRole('combobox')).toHaveValue('unsaved_name')
    expect(screen.getAllByTestId('handle')).toHaveLength(4)
    fireEvent.click(screen.getByRole('button', { name: '中文' }))
    expect(screen.getByText('工具调用')).toBeInTheDocument()
    expect(screen.getByRole('combobox')).toHaveValue('unsaved_name')
  })
})
