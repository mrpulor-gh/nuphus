import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../../../locales'
import {
  ConditionRow,
  JsonField,
  KeyValueRows,
  NumberField,
  PathField,
  Segmented,
  Switch,
} from './controls'

/* ── Segmented ── */

describe('Segmented', () => {
  const options = [
    { value: 'text', label: '文本' },
    { value: 'image', label: '图像' },
    { value: 'video', label: '视频' },
  ]

  it('radiogroup + radio aria-checked，active 项带 is-active', () => {
    const { container } = render(
      <LangProvider>
        <Segmented options={options} value="image" onChange={vi.fn()} />
      </LangProvider>,
    )
    expect(screen.getByRole('radiogroup')).toBeInTheDocument()
    const radios = screen.getAllByRole('radio')
    expect(radios).toHaveLength(3)
    expect(screen.getByRole('radio', { name: '图像' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('radio', { name: '文本' })).toHaveAttribute('aria-checked', 'false')
    expect(container.querySelectorAll('.wfp-seg__item.is-active')).toHaveLength(1)
  })

  it('点击选项回调 onChange（value 原样透传）', () => {
    const onChange = vi.fn()
    render(
      <LangProvider>
        <Segmented options={options} value="text" onChange={onChange} />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('radio', { name: '视频' }))
    expect(onChange).toHaveBeenCalledWith('video')
  })

  it('disabled 时所有选项禁用', () => {
    render(
      <LangProvider>
        <Segmented options={options} value="text" onChange={vi.fn()} disabled />
      </LangProvider>,
    )
    for (const radio of screen.getAllByRole('radio')) {
      expect(radio).toBeDisabled()
    }
  })
})

/* ── Switch ── */

describe('Switch', () => {
  it('role=switch + aria-checked 反映 on/off，is-on 类同步', () => {
    const { container, rerender } = render(
      <LangProvider>
        <Switch checked={false} onChange={vi.fn()} />
      </LangProvider>,
    )
    const sw = screen.getByRole('switch')
    expect(sw).toHaveAttribute('aria-checked', 'false')
    expect(container.querySelector('.wfp-sw')).not.toHaveClass('is-on')

    rerender(
      <LangProvider>
        <Switch checked onChange={vi.fn()} />
      </LangProvider>,
    )
    expect(screen.getByRole('switch')).toHaveAttribute('aria-checked', 'true')
    expect(container.querySelector('.wfp-sw')).toHaveClass('is-on')
  })

  it('点击回调取反后的目标态', () => {
    const onChange = vi.fn()
    render(
      <LangProvider>
        <Switch checked={false} onChange={onChange} label="失败继续" />
      </LangProvider>,
    )
    expect(screen.getByText('失败继续')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('switch'))
    expect(onChange).toHaveBeenCalledWith(true)
  })
})

/* ── NumberField ── */

describe('NumberField', () => {
  it('inputMode 随 integer 切换，单位后缀渲染', () => {
    render(
      <LangProvider>
        <NumberField value="30" onCommit={vi.fn()} integer unit="秒" />
      </LangProvider>,
    )
    expect(screen.getByRole('textbox')).toHaveAttribute('inputmode', 'numeric')
    expect(screen.getByText('秒')).toHaveClass('wfp-num__unit')
  })

  it('小数场景 inputmode=decimal', () => {
    render(
      <LangProvider>
        <NumberField value="0.7" onCommit={vi.fn()} />
      </LangProvider>,
    )
    expect(screen.getByRole('textbox')).toHaveAttribute('inputmode', 'decimal')
  })

  it('blur 提交草稿；空值原样提交（默认语义交调用方）', () => {
    const onCommit = vi.fn()
    render(
      <LangProvider>
        <NumberField value="30" onCommit={onCommit} integer />
      </LangProvider>,
    )
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: '' } })
    fireEvent.blur(input)
    expect(onCommit).toHaveBeenCalledWith('')
  })

  it('方向键步进：整数 ±1、小数 ±0.1（S3 步进）', () => {
    const onInt = vi.fn()
    render(
      <LangProvider>
        <NumberField value="5" onCommit={onInt} integer min={1} max={10} />
      </LangProvider>,
    )
    const input = screen.getByRole('textbox')
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    expect(onInt).toHaveBeenCalledWith('6')
    // 步进会同时推进本地草稿：第二次从 6 起算
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(onInt).toHaveBeenLastCalledWith('5')

    const onFloat = vi.fn()
    render(
      <LangProvider>
        <NumberField value="0.7" onCommit={onFloat} />
      </LangProvider>,
    )
    fireEvent.keyDown(screen.getAllByRole('textbox')[1], { key: 'ArrowUp' })
    expect(onFloat).toHaveBeenCalledWith('0.8')
  })

  it('步进钳制在 [min,max]，空值从 min 起步', () => {
    const onCommit = vi.fn()
    render(
      <LangProvider>
        <NumberField value="10" onCommit={onCommit} integer min={1} max={10} />
      </LangProvider>,
    )
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'ArrowUp' })
    expect(onCommit).toHaveBeenCalledWith('10')

    const onEmpty = vi.fn()
    render(
      <LangProvider>
        <NumberField value="" onCommit={onEmpty} integer min={2} max={10} />
      </LangProvider>,
    )
    fireEvent.keyDown(screen.getAllByRole('textbox')[1], { key: 'ArrowUp' })
    expect(onEmpty).toHaveBeenCalledWith('3')
  })

  it('invalid 给出 error 描边类与 aria-invalid', () => {
    const { container } = render(
      <LangProvider>
        <NumberField value="abc" onCommit={vi.fn()} invalid />
      </LangProvider>,
    )
    expect(container.querySelector('.wfp-num')).toHaveClass('wfp-num--invalid')
    expect(screen.getByRole('textbox')).toHaveAttribute('aria-invalid', 'true')
  })

  it('label 落为 aria-label（FieldRow 的 label 是 div，input 需显式命名）', () => {
    render(
      <LangProvider>
        <NumberField value="0.7" onCommit={vi.fn()} label="随机程度" />
      </LangProvider>,
    )
    expect(screen.getByRole('textbox', { name: '随机程度' })).toBeInTheDocument()
  })

  it('onDraftChange 每次输入都通报（画布草稿存储据此置脏）', () => {
    const onDraftChange = vi.fn()
    render(
      <LangProvider>
        <NumberField value="5" onCommit={vi.fn()} onDraftChange={onDraftChange} />
      </LangProvider>,
    )
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '55' } })
    expect(onDraftChange).toHaveBeenCalledWith('55')
  })
})

/* ── PathField ── */

describe('PathField', () => {
  it('等宽输入 + 「插变量」按钮回调', () => {
    const onInsertVariable = vi.fn()
    const { container } = render(
      <LangProvider>
        <PathField value="out/first.png" onCommit={vi.fn()} onInsertVariable={onInsertVariable} />
      </LangProvider>,
    )
    expect(container.querySelector('.wfp-path__input')).not.toBeNull()
    const btn = screen.getByRole('button', { name: '插入变量引用' })
    expect(btn.querySelector('svg')).not.toBeNull()
    fireEvent.click(btn)
    expect(onInsertVariable).toHaveBeenCalledTimes(1)
  })

  it('references=false 或无回调时不渲染死按钮', () => {
    const { container } = render(
      <LangProvider>
        <PathField value="a.png" onCommit={vi.fn()} references={false} onInsertVariable={vi.fn()} />
      </LangProvider>,
    )
    expect(screen.queryByRole('button')).toBeNull()
    expect(container.querySelector('.wfp-path__input')).not.toBeNull()
  })

  it('blur 提交路径草稿', () => {
    const onCommit = vi.fn()
    render(
      <LangProvider>
        <PathField value="a.png" onCommit={onCommit} />
      </LangProvider>,
    )
    const input = screen.getByRole('textbox')
    fireEvent.change(input, { target: { value: 'b.png' } })
    fireEvent.blur(input)
    expect(onCommit).toHaveBeenCalledWith('b.png')
  })
})

/* ── JsonField ── */

describe('JsonField', () => {
  it('合法 JSON：success 徽标「格式正确」+ 等宽预览', () => {
    const { container } = render(
      <LangProvider>
        <JsonField value={'{\n  "top_p": 0.95\n}'} onCommit={vi.fn()} />
      </LangProvider>,
    )
    const badge = container.querySelector('.wfp-json__badge')
    expect(badge).toHaveClass('wfp-json__badge--ok')
    expect(badge).toHaveTextContent('格式正确')
    expect(container.querySelector('.wfp-json__pre')).toHaveTextContent('"top_p": 0.95')
  })

  it('非法 JSON：error 徽标「格式错误」', () => {
    const { container } = render(
      <LangProvider>
        <JsonField value={'{ "top_p": '} onCommit={vi.fn()} />
      </LangProvider>,
    )
    const badge = container.querySelector('.wfp-json__badge')
    expect(badge).toHaveClass('wfp-json__badge--err')
    expect(badge).toHaveTextContent('格式错误')
  })

  it('空值视为合法（默认语义，不报错）', () => {
    const { container } = render(
      <LangProvider>
        <JsonField value="" onCommit={vi.fn()} />
      </LangProvider>,
    )
    expect(container.querySelector('.wfp-json__badge')).toHaveClass('wfp-json__badge--ok')
  })

  it('editable 逃生口：textarea 编辑 + blur 提交', () => {
    const onCommit = vi.fn()
    render(
      <LangProvider>
        <JsonField value={'{"a":1}'} onCommit={onCommit} editable />
      </LangProvider>,
    )
    const textarea = screen.getByRole('textbox')
    fireEvent.change(textarea, { target: { value: '{"a":2}' } })
    fireEvent.blur(textarea)
    expect(onCommit).toHaveBeenCalledWith('{"a":2}')
  })
})

/* ── KeyValueRows ── */

describe('KeyValueRows', () => {
  const rows = [
    { key: 'Content-Type', value: 'application/json' },
    { key: 'X-Trace', value: '1' },
  ]

  it('渲染键值输入与删除按钮', () => {
    render(
      <LangProvider>
        <KeyValueRows rows={rows} onChange={vi.fn()} />
      </LangProvider>,
    )
    expect(screen.getByDisplayValue('Content-Type')).toBeInTheDocument()
    expect(screen.getByDisplayValue('application/json')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: '删除该项' })).toHaveLength(2)
    expect(screen.getByRole('button', { name: '添加一项' })).toBeInTheDocument()
  })

  it('编辑键/值回调增量行', () => {
    const onChange = vi.fn()
    render(
      <LangProvider>
        <KeyValueRows rows={rows} onChange={onChange} />
      </LangProvider>,
    )
    fireEvent.change(screen.getByDisplayValue('X-Trace'), { target: { value: 'X-Span' } })
    expect(onChange).toHaveBeenCalledWith([
      { key: 'Content-Type', value: 'application/json' },
      { key: 'X-Span', value: '1' },
    ])
  })

  it('删除行回调过滤后的数组', () => {
    const onChange = vi.fn()
    render(
      <LangProvider>
        <KeyValueRows rows={rows} onChange={onChange} />
      </LangProvider>,
    )
    fireEvent.click(screen.getAllByRole('button', { name: '删除该项' })[0])
    expect(onChange).toHaveBeenCalledWith([{ key: 'X-Trace', value: '1' }])
  })

  it('添加一项追加空行', () => {
    const onChange = vi.fn()
    render(
      <LangProvider>
        <KeyValueRows rows={rows} onChange={onChange} />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: '添加一项' }))
    expect(onChange).toHaveBeenCalledWith([...rows, { key: '', value: '' }])
  })
})

/* ── ConditionRow ── */

describe('ConditionRow', () => {
  it('三列结构：left / op 等宽徽标 / right', () => {
    const { container } = render(
      <LangProvider>
        <ConditionRow
          left={<input aria-label="left" />}
          op="=="
          right={<input aria-label="right" />}
        />
      </LangProvider>,
    )
    expect(container.querySelector('.wfp-cond')).not.toBeNull()
    const op = container.querySelector('.wfp-cond__op')
    expect(op).toHaveTextContent('==')
    expect(screen.getByLabelText('left')).toBeInTheDocument()
    expect(screen.getByLabelText('right')).toBeInTheDocument()
  })
})
