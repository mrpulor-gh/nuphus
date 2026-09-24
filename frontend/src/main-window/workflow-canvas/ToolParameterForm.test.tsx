import { useState } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { InspectorDraftContext, InspectorDraftStore } from './inspectorDrafts'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  ToolParameterForm,
  declaredParameterDefaults,
  parameterProperties,
  validateToolParameters,
  type ToolParameterFormProps,
} from './ToolParameterForm'

afterEach(cleanup)

function Harness(
  props: Omit<ToolParameterFormProps, 'onChange'> & {
    onChange?: ToolParameterFormProps['onChange']
  },
) {
  const [value, setValue] = useState(props.value)
  return (
    <ToolParameterForm
      {...props}
      value={value}
      onChange={next => {
        setValue(next)
        props.onChange?.(next)
      }}
    />
  )
}

const schema = {
  type: 'object',
  required: ['count'],
  properties: {
    text: { type: 'string', title: '文本', description: '要输入的文本' },
    count: { type: 'integer', title: '数量', minimum: 1 },
    enabled: { type: 'boolean', title: '启用' },
    mode: { type: 'string', title: '模式', enum: ['fast', 'slow'] },
    data: { type: 'object', title: '数据' },
  },
}

describe('ToolParameterForm', () => {
  it('allows replacing a string template with literal or mixed text', () => {
    const onChange = vi.fn()
    render(
      <Harness
        schema={schema}
        value={{ text: '{{inputs.topic}}', count: 2 }}
        onChange={onChange}
      />,
    )
    fireEvent.change(screen.getByLabelText('文本'), { target: { value: '新的文本' } })
    expect(onChange).toHaveBeenLastCalledWith({ text: '新的文本', count: 2 })
    fireEvent.change(screen.getByLabelText('文本'), { target: { value: '标题 {{inputs.topic}}' } })
    expect(onChange).toHaveBeenLastCalledWith({ text: '标题 {{inputs.topic}}', count: 2 })
  })

  it('explicit reset discards invalid fields hidden by a schema change', async () => {
    const store = new InspectorDraftStore()
    const onChange = vi.fn()
    const view = (resetVersion: number, newSchema: ToolParameterFormProps['schema'] = schema) => (
      <InspectorDraftContext.Provider value={{ store, nodeId: 'a' }}>
        <ToolParameterForm
          schema={newSchema}
          value={{}}
          resetVersion={resetVersion}
          onChange={onChange}
        />
      </InspectorDraftContext.Provider>
    )
    const { rerender } = render(view(0))
    fireEvent.change(screen.getByLabelText('数量'), { target: { value: 'bad' } })
    await act(async () => {
      expect(await store.flush()).not.toBeNull()
    })
    const emptySchema = { ...schema, required: [], properties: {} }
    rerender(view(1, emptySchema))
    await act(async () => {
      expect(await store.flush()).toBeNull()
    })
    expect(screen.queryByText(/有效 JSON 值/)).not.toBeInTheDocument()
    expect(store.dirty).toBe(false)
  })
  it('flushes unblurred form edits as one canvas draft and blocks invalid drafts', async () => {
    const store = new InspectorDraftStore()
    const onChange = vi.fn()
    render(
      <InspectorDraftContext.Provider value={{ store, nodeId: 'tool-1' }}>
        <Harness schema={schema} value={{ count: 2 }} onChange={onChange} />
      </InspectorDraftContext.Provider>,
    )
    fireEvent.change(screen.getByLabelText('数量'), { target: { value: '12' } })
    fireEvent.change(screen.getByLabelText('数量'), { target: { value: '123' } })
    expect(onChange).not.toHaveBeenCalled()
    expect(store.dirty).toBe(true)
    await act(async () => {
      expect(await store.flush()).toBeNull()
    })
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenLastCalledWith({ count: 123 })
    fireEvent.change(screen.getByLabelText('数量'), { target: { value: 'invalid' } })
    await act(async () => {
      expect(await store.flush()).toMatchObject({ nodeId: 'tool-1', field: '动作参数' })
    })
    expect(onChange).toHaveBeenCalledTimes(1)
  })

  it('recovers unfinished parameter drafts when the editor is remounted', () => {
    const store = new InspectorDraftStore()
    const props = { schema, value: { count: 2 } }
    const renderEditor = () => (
      <InspectorDraftContext.Provider value={{ store, nodeId: 'tool-1' }}>
        <Harness {...props} />
      </InspectorDraftContext.Provider>
    )
    const { unmount } = render(renderEditor())
    fireEvent.change(screen.getByLabelText('数量'), { target: { value: '-' } })
    unmount()
    render(renderEditor())
    expect(screen.getByLabelText('数量')).toHaveValue('-')
    expect(screen.getByLabelText('数量')).toHaveAttribute('aria-invalid', 'true')
  })
  it('preserves unknown keys and distinguishes empty text, false and omitted values', () => {
    const onChange = vi.fn()
    render(<Harness schema={schema} value={{ extra: null, count: 2 }} onChange={onChange} />)
    expect(screen.getByLabelText('启用')).toHaveValue('')
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('文本'), { target: { value: 'hello' } })
    fireEvent.change(screen.getByLabelText('文本'), { target: { value: '' } })
    expect(onChange).toHaveBeenLastCalledWith({ extra: null, count: 2, text: '' })
    fireEvent.change(screen.getByLabelText('启用'), { target: { value: 'false' } })
    expect(onChange).toHaveBeenLastCalledWith({ extra: null, count: 2, text: '', enabled: false })
    fireEvent.change(screen.getByLabelText('启用'), { target: { value: '' } })
    expect(onChange).toHaveBeenLastCalledWith({ extra: null, count: 2, text: '' })
    expect(screen.getByText(/保留未识别参数：extra/)).toBeInTheDocument()
  })

  it('retains invalid number drafts and does not emit zero from empty text', () => {
    const onChange = vi.fn()
    const onDraftChange = vi.fn()
    render(
      <Harness
        schema={schema}
        value={{ count: 2 }}
        onChange={onChange}
        onDraftChange={onDraftChange}
      />,
    )
    const count = screen.getByLabelText(/数量/)
    fireEvent.change(count, { target: { value: '' } })
    expect(count).toHaveAttribute('aria-invalid', 'true')
    expect(onChange).not.toHaveBeenCalled()
    expect(onDraftChange.mock.lastCall?.[0].fields.count).toBe('')
    expect(screen.getByRole('button', { name: 'JSON 编辑' })).toBeDisabled()
    fireEvent.change(count, { target: { value: '1.5' } })
    expect(screen.getByText('请输入有效整数')).toBeInTheDocument()
    fireEvent.change(count, { target: { value: '0' } })
    expect(screen.getByText('不能小于 1')).toBeInTheDocument()
    fireEvent.change(count, { target: { value: '3' } })
    expect(onChange).toHaveBeenLastCalledWith({ count: 3 })
  })

  it('allows expressions for typed parameters and retains typed values through mode switches', () => {
    const onChange = vi.fn()
    render(
      <Harness
        schema={schema}
        value={{ count: 2, enabled: false, extra: [null] }}
        onChange={onChange}
        variables={[{ name: 'inputs.count', label: '输入数量' }]}
      />,
    )
    fireEvent.change(screen.getByLabelText('为 数量 插入变量'), {
      target: { value: '{{inputs.count}}' },
    })
    expect(onChange).toHaveBeenLastCalledWith({
      count: '{{inputs.count}}',
      enabled: false,
      extra: [null],
    })
    fireEvent.click(screen.getByRole('button', { name: 'JSON 编辑' }))
    expect(JSON.parse((screen.getByLabelText('参数 JSON') as HTMLTextAreaElement).value)).toEqual({
      count: '{{inputs.count}}',
      enabled: false,
      extra: [null],
    })
    fireEvent.click(screen.getByRole('button', { name: '表单编辑' }))
    expect(screen.getByLabelText('数量')).toHaveValue('{{inputs.count}}')
    expect(screen.getByLabelText('启用')).toHaveValue('false')
  })

  it('edits enums as their declared JSON types and nested objects without deleting other keys', () => {
    const onChange = vi.fn()
    render(<Harness schema={schema} value={{ count: 1, extra: 'keep' }} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText('模式'), { target: { value: '"slow"' } })
    expect(onChange).toHaveBeenLastCalledWith({ count: 1, extra: 'keep', mode: 'slow' })
    fireEvent.change(screen.getByLabelText('数据'), {
      target: { value: '{"nested":[0,false,null]}' },
    })
    expect(onChange).toHaveBeenLastCalledWith({
      count: 1,
      extra: 'keep',
      mode: 'slow',
      data: { nested: [0, false, null] },
    })
  })

  it('does not discard invalid JSON when switching editors', () => {
    const onChange = vi.fn()
    render(<Harness schema={schema} value={{ count: 1 }} onChange={onChange} />)
    fireEvent.click(screen.getByRole('button', { name: 'JSON 编辑' }))
    fireEvent.change(screen.getByLabelText('参数 JSON'), { target: { value: '{"count":' } })
    expect(screen.getByRole('button', { name: '表单编辑' })).toBeDisabled()
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.change(screen.getByLabelText('参数 JSON'), {
      target: { value: '{"count":4,"extra":null}' },
    })
    fireEvent.click(screen.getByRole('button', { name: '表单编辑' }))
    expect(screen.getByLabelText(/数量/)).toHaveValue('4')
    expect(onChange).toHaveBeenLastCalledWith({ count: 4, extra: null })
  })

  it('falls back to raw JSON for unsupported schemas without coercing existing parameters', () => {
    const onChange = vi.fn()
    render(<Harness schema={{ oneOf: [] }} value={{ unknown: null }} onChange={onChange} />)
    expect(screen.getByLabelText('参数 JSON')).toHaveValue('{\n  "unknown": null\n}')
    expect(screen.queryByRole('button', { name: '表单编辑' })).not.toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('offers only explicitly declared defaults without applying them on mount', () => {
    const onChange = vi.fn()
    const defaultsSchema = {
      type: 'object',
      properties: {
        count: { type: 'integer', title: '数量', default: 0 },
        enabled: { type: 'boolean' },
        mode: { type: 'string', enum: ['first'] },
      },
    }
    expect(declaredParameterDefaults(defaultsSchema)).toEqual({ count: 0 })
    render(<Harness schema={defaultsSchema} value={{}} onChange={onChange} />)
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '使用默认值：0' }))
    expect(onChange).toHaveBeenLastCalledWith({ count: 0 })
  })

  it('preserves unsupported null values and reports their mismatch rather than converting them', () => {
    const onChange = vi.fn()
    render(<Harness schema={schema} value={{ count: null }} onChange={onChange} />)
    expect(screen.getByLabelText(/数量/)).toHaveValue('null')
    expect(onChange).not.toHaveBeenCalled()
    expect(screen.getByText(/此参数类型不接受 null/)).toBeInTheDocument()
  })

  it('exposes reusable validation while leaving unsupported complex schemas untouched', () => {
    expect(validateToolParameters({}, schema)).toEqual({ count: '请填写必填参数' })
    expect(validateToolParameters({ count: '{{inputs.count}}' }, schema)).toEqual({})
    expect(
      parameterProperties({
        type: 'object',
        properties: { x: { anyOf: [{ type: 'number' }, { type: 'null' }] } },
      }),
    ).not.toBeNull()
    expect(
      validateToolParameters(
        { x: null },
        { type: 'object', properties: { x: { anyOf: [{ type: 'number' }, { type: 'null' }] } } },
      ),
    ).toEqual({})
  })
})
