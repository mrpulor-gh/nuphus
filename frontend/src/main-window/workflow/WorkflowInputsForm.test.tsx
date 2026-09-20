import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  WorkflowInputsDialog,
  WorkflowInputsForm,
  useWorkflowInputs,
  type WorkflowInputsState,
} from './WorkflowInputsForm'
import type { WorkflowInputSpec } from '../../core/types'

/** 覆盖全部 5 种类型 + sensitive 的声明 */
const ALL_KINDS: WorkflowInputSpec[] = [
  { name: 'text_one', type: 'string', description: '文本说明' },
  { name: 'count', type: 'number' },
  { name: 'flag', type: 'boolean' },
  { name: 'file_path', type: 'path' },
  { name: 'blob', type: 'json' },
  { name: 'secret', type: 'string', required: true, sensitive: true },
]

function stubState(overrides: Partial<WorkflowInputsState> = {}): WorkflowInputsState {
  return {
    values: {},
    canSubmit: true,
    errors: {},
    setValue: () => {},
    payload: {},
    ...overrides,
  }
}

/** 用真实 hook 驱动表单：断言「编辑态 → 提交对象」的转换与校验 */
function HookHarness({
  specs,
  token = 't1',
  onPayload,
}: {
  specs: WorkflowInputSpec[]
  token?: string
  onPayload: (payload: Record<string, unknown>) => void
}) {
  const state = useWorkflowInputs(specs, token)
  return (
    <div>
      <WorkflowInputsForm specs={specs} state={state} />
      <span data-testid="can-submit">{state.canSubmit ? 'yes' : 'no'}</span>
      <button type="button" onClick={() => onPayload(state.payload)}>
        提交
      </button>
    </div>
  )
}

describe('WorkflowInputsForm 控件渲染', () => {
  it('按声明类型渲染对应控件（含 sensitive 密码控件）', () => {
    render(<WorkflowInputsForm specs={ALL_KINDS} state={stubState()} />)

    expect(screen.getByLabelText(/text_one/)).toHaveAttribute('type', 'text')
    expect(screen.getByLabelText(/count/)).toHaveAttribute('type', 'number')
    expect(screen.getByLabelText(/flag/)).toHaveAttribute('type', 'checkbox')
    expect(screen.getByLabelText(/file_path/)).toHaveAttribute('type', 'text')
    expect(screen.getByLabelText(/blob/).tagName).toBe('TEXTAREA')
    expect(screen.getByLabelText(/secret/)).toHaveAttribute('type', 'password')

    // 说明文案与 required 标记
    expect(screen.getByText('文本说明')).toBeInTheDocument()
    expect(screen.getByText('必填')).toBeInTheDocument()
  })

  it('字段错误原因渲染在字段下方', () => {
    render(
      <WorkflowInputsForm
        specs={[{ name: 'token', required: true }]}
        state={stubState({ canSubmit: false, errors: { token: '必填输入，未填写' } })}
      />,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('必填输入，未填写')
  })
})

describe('useWorkflowInputs 求值与提交对象', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('default 预填 + 各类型转换（number/json/boolean/path）', () => {
    const onPayload = vi.fn()
    render(
      <HookHarness
        specs={[
          { name: 'topic', type: 'string', default: '默认主题' },
          { name: 'count', type: 'number', default: 2 },
          { name: 'flag', type: 'boolean', default: true },
          { name: 'file_path', type: 'path' },
          { name: 'blob', type: 'json', default: { a: 1 } },
        ]}
        onPayload={onPayload}
      />,
    )

    expect(screen.getByLabelText(/topic/)).toHaveValue('默认主题')
    expect(screen.getByLabelText(/flag/)).toBeChecked()

    fireEvent.change(screen.getByLabelText(/file_path/), { target: { value: 'D:/tmp/a.txt' } })
    fireEvent.change(screen.getByLabelText(/blob/), { target: { value: '{"b":[1,2]}' } })
    fireEvent.click(screen.getByRole('button', { name: '提交' }))

    expect(onPayload).toHaveBeenCalledWith({
      topic: '默认主题',
      count: 2,
      flag: true,
      file_path: 'D:/tmp/a.txt',
      blob: { b: [1, 2] },
    })
  })

  it('非必填未填 → 不发送该字段，且不阻断提交', () => {
    const onPayload = vi.fn()
    render(
      <HookHarness
        specs={[{ name: 'filled' }, { name: 'empty_optional', type: 'number' }]}
        onPayload={onPayload}
      />,
    )

    fireEvent.change(screen.getByLabelText(/filled/), { target: { value: 'v' } })
    expect(screen.getByTestId('can-submit')).toHaveTextContent('yes')
    fireEvent.click(screen.getByRole('button', { name: '提交' }))

    expect(onPayload).toHaveBeenCalledWith({ filled: 'v' })
  })

  it('必填未填 / JSON 非法 → 阻断并给出原因；空的可选字段不发送', () => {
    const onPayload = vi.fn()
    render(
      <HookHarness
        specs={[
          { name: 'need_it', required: true },
          { name: 'count', type: 'number' },
          { name: 'blob', type: 'json' },
        ]}
        onPayload={onPayload}
      />,
    )

    expect(screen.getByTestId('can-submit')).toHaveTextContent('no')
    expect(screen.getByText('必填输入，未填写')).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText(/need_it/), { target: { value: 'x' } })
    expect(screen.getByTestId('can-submit')).toHaveTextContent('yes')

    // JSON 非法 → 阻断并给出原因
    fireEvent.change(screen.getByLabelText(/blob/), { target: { value: '{oops' } })
    expect(screen.getByTestId('can-submit')).toHaveTextContent('no')
    expect(screen.getByText('JSON 格式无效')).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText(/blob/), { target: { value: '[1]' } })
    expect(screen.getByTestId('can-submit')).toHaveTextContent('yes')

    fireEvent.click(screen.getByRole('button', { name: '提交' }))
    // count 未填（number 控件拒收非数字 → 空值）→ 不发送，交由后端「未提供 → 不注入」
    expect(onPayload).toHaveBeenCalledWith({ need_it: 'x', blob: [1] })
  })

  it('重置令牌变化 → 回到 default 预填态（切换工作流不残留旧输入）', () => {
    const { rerender } = render(
      <HookHarness
        specs={[{ name: 'topic', default: '默认' }]}
        token="wf-a|open"
        onPayload={vi.fn()}
      />,
    )
    fireEvent.change(screen.getByLabelText(/topic/), { target: { value: '改过' } })
    expect(screen.getByLabelText(/topic/)).toHaveValue('改过')

    rerender(
      <HookHarness
        specs={[{ name: 'topic', default: '默认' }]}
        token="wf-b|open"
        onPayload={vi.fn()}
      />,
    )
    expect(screen.getByLabelText(/topic/)).toHaveValue('默认')
  })
})

describe('WorkflowInputsDialog（画布运行路径共用的同一实现）', () => {
  it('必填未填阻断启动，填写后回调收到提交对象', () => {
    const onConfirm = vi.fn()
    render(
      <WorkflowInputsDialog
        open
        specs={[{ name: 'token', required: true, sensitive: true }]}
        resetToken="wf-1"
        onConfirm={onConfirm}
        onCancel={() => {}}
      />,
    )

    const start = screen.getByRole('button', { name: '启动' })
    expect(start).toBeDisabled()
    expect(screen.getByText('必填输入，未填写')).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText(/token/), { target: { value: 'v' } })
    expect(start).toBeEnabled()
    fireEvent.click(start)
    expect(onConfirm).toHaveBeenCalledWith({ token: 'v' })
  })
})
