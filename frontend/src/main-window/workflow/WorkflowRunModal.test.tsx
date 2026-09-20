import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WorkflowRunModal } from './WorkflowRunModal'
import type { WorkflowInputSpec, WorkflowItem } from '../../core/types'

function workflowWith(inputs?: WorkflowInputSpec[]): WorkflowItem {
  return {
    id: 'wf-1',
    title: '测试工作流',
    steps: [{ id: 's1', name: '读文件', do: { tool: 'Read', with: { path: 'a.txt' } } }],
    tags: [],
    created_at: 0,
    updated_at: 0,
    run_count: 0,
    status: 'draft',
    inputs,
  }
}

function renderModal(workflow: WorkflowItem, onRun = vi.fn()) {
  render(<WorkflowRunModal open workflow={workflow} onRun={onRun} onCancel={() => {}} />)
  return onRun
}

describe('WorkflowRunModal 外部输入表单', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    window.localStorage.clear()
    window.sessionStorage.clear()
  })

  it('未声明 inputs → 不渲染表单区，启动照旧（回归）', () => {
    const onRun = renderModal(workflowWith())

    expect(screen.queryByText('运行前需要填写外部输入')).not.toBeInTheDocument()
    expect(screen.getByText('读文件')).toBeInTheDocument()

    const start = screen.getByRole('button', { name: '启动' })
    expect(start).toBeEnabled()
    fireEvent.click(start)
    // 未声明工作流：保持既有 onRun(id) 语义（inputs 为 undefined）
    expect(onRun).toHaveBeenCalledWith('wf-1', undefined)
  })

  it('声明 inputs → 渲染表单，default 预填且可修改', () => {
    renderModal(
      workflowWith([
        { name: 'topic', type: 'string', default: '默认主题', description: '写什么' },
        { name: 'rounds', type: 'number', default: 3 },
        { name: 'json_blob', type: 'json', default: { a: 1 } },
      ]),
    )

    expect(screen.getByText('运行前需要填写外部输入')).toBeInTheDocument()
    expect(screen.getByText('写什么')).toBeInTheDocument()
    expect(screen.getByLabelText(/topic/)).toHaveValue('默认主题')
    expect(screen.getByLabelText(/rounds/)).toHaveValue(3)
    expect(screen.getByLabelText(/json_blob/)).toHaveValue('{"a":1}')

    fireEvent.change(screen.getByLabelText(/topic/), { target: { value: '改过的主题' } })
    expect(screen.getByLabelText(/topic/)).toHaveValue('改过的主题')
  })

  it('required 未填且无 default → 禁用启动并在字段下方给出原因', () => {
    renderModal(workflowWith([{ name: 'token', required: true }, { name: 'note' }]))

    const start = screen.getByRole('button', { name: '启动' })
    expect(start).toBeDisabled()
    expect(screen.getByText('必填输入，未填写')).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText(/token/), { target: { value: 'abc' } })
    expect(start).toBeEnabled()
    expect(screen.queryByText('必填输入，未填写')).not.toBeInTheDocument()
  })

  it('sensitive → 密码控件、不回显、不落 localStorage/sessionStorage', () => {
    const onRun = renderModal(workflowWith([{ name: 'secret', required: true, sensitive: true }]))

    const field = screen.getByLabelText(/secret/) as HTMLInputElement
    expect(field.type).toBe('password')

    fireEvent.change(field, { target: { value: 'S3CRET-VALUE' } })
    // 不回显（DOM 中不存在明文文本）
    expect(screen.queryByText('S3CRET-VALUE')).not.toBeInTheDocument()
    // 不落任何持久化
    expect(window.localStorage.length).toBe(0)
    expect(window.sessionStorage.length).toBe(0)

    fireEvent.click(screen.getByRole('button', { name: '启动' }))
    // 值只随提交对象存在于内存
    expect(onRun).toHaveBeenCalledWith('wf-1', { secret: 'S3CRET-VALUE' })
  })

  it('提交对象只含已声明字段（未填的可选字段不发送）', () => {
    const onRun = renderModal(workflowWith([{ name: 'a' }, { name: 'b' }]))

    fireEvent.change(screen.getByLabelText(/a/), { target: { value: 'A' } })
    fireEvent.click(screen.getByRole('button', { name: '启动' }))

    expect(onRun).toHaveBeenCalledTimes(1)
    const payload = onRun.mock.calls[0][1] as Record<string, unknown>
    expect(payload).toEqual({ a: 'A' })
    expect(Object.keys(payload)).toEqual(['a'])
  })

  it('全部输入可省略（非必填无 default）→ 提交空对象仍走输入链路', () => {
    const onRun = renderModal(workflowWith([{ name: 'opt' }]))

    expect(screen.getByRole('button', { name: '启动' })).toBeEnabled()
    fireEvent.click(screen.getByRole('button', { name: '启动' }))
    expect(onRun).toHaveBeenCalledWith('wf-1', {})
  })
})
