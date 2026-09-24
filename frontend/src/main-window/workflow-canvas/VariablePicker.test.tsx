import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { VariablePicker } from './VariablePicker'
import type { VariableCatalog } from './variableCatalog'

const catalog: VariableCatalog = {
  references: [
    { name: 'inputs.topic', source: 'input', sourceLabel: '工作流输入 · topic', maybeUnset: false },
    { name: 'reply', source: 'capture', sourceLabel: '生成回复', maybeUnset: true },
  ],
  captures: [{ name: 'reply', source: 'capture', sourceLabel: '生成回复', maybeUnset: false }],
}

describe('VariablePicker', () => {
  it('opens a portal on click and selects a bare namespaced reference', () => {
    const onChange = vi.fn()
    const { container } = render(
      <VariablePicker value="" onChange={onChange} mode="reference" catalog={catalog} />,
    )
    fireEvent.click(screen.getByRole('combobox'))
    expect(container.querySelector('[role="listbox"]')).toBeNull()
    fireEvent.click(screen.getByRole('option', { name: /inputs.topic/ }))
    expect(onChange).toHaveBeenCalledWith('inputs.topic')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })
  it('shows explicit new capture and warns when updating an existing name', () => {
    const { rerender } = render(
      <VariablePicker value="new_result" onChange={vi.fn()} mode="capture" catalog={catalog} />,
    )
    fireEvent.click(screen.getByRole('combobox'))
    expect(screen.getByRole('option', { name: '＋新建变量 new_result' })).toBeInTheDocument()
    rerender(<VariablePicker value="reply" onChange={vi.fn()} mode="capture" catalog={catalog} />)
    expect(screen.getByText('将更新已有变量 · 生成回复')).toBeInTheDocument()
  })
  it('allows changing an already selected variable without clearing its name first', () => {
    render(<VariablePicker value="reply" onChange={vi.fn()} mode="reference" catalog={catalog} />)
    fireEvent.click(screen.getByRole('combobox'))
    expect(screen.getByRole('option', { name: /inputs.topic/ })).toBeInTheDocument()
  })
  it('retains unknown references rather than creating an output and links input configuration', () => {
    const configure = vi.fn()
    render(
      <VariablePicker
        value="inputs.missing"
        onChange={vi.fn()}
        mode="reference"
        catalog={catalog}
        onConfigureInput={configure}
      />,
    )
    expect(screen.getByText(/未找到来源/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '设置工作流输入' }))
    expect(configure).toHaveBeenCalledWith('missing')
  })
  it('shows conditional availability and does not accept a composing Enter', () => {
    const onChange = vi.fn()
    render(<VariablePicker value="reply" onChange={onChange} mode="reference" catalog={catalog} />)
    expect(screen.getByText(/可能未赋值，请检查/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('combobox'))
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter', isComposing: true })
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.keyDown(screen.getByRole('combobox'), { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith('reply')
  })
})
