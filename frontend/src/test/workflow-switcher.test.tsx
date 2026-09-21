import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WorkflowSwitcher } from '../main-window/workflow-canvas/WorkflowSwitcher'

/**
 * 工作流切换下拉（画布标题右侧）契约。
 *
 * 需求背景：过去换工作流必须退回列表页重新找。现在标题旁直接给下拉，
 * 但要守住三点：懒加载（打开画布不额外付 IPC）、当前项标记（确认「我在哪」）、
 * 禁用态不展开（运行中 / 回放中切换会丢上下文）。
 */
const listWorkflows = vi.fn()
vi.mock('../main-window/lib/api', () => ({
  listWorkflows: () => listWorkflows(),
}))

function wf(id: string, title: string) {
  return {
    id,
    title,
    steps: [],
    tags: [],
    created_at: 0,
    updated_at: 0,
    run_count: 0,
    status: 'draft' as const,
  }
}

/** 触发器（菜单项也是 button，故按容器定位） */
function trigger(): HTMLElement {
  const el = document.querySelector('.wfc-wf-switch > button')
  if (!el) throw new Error('未渲染切换触发器')
  return el as HTMLElement
}

describe('工作流切换下拉', () => {
  beforeEach(() => {
    listWorkflows.mockReset()
  })

  it('懒加载：未展开不查列表，展开后才出现菜单与「当前」标记', async () => {
    listWorkflows.mockResolvedValue([wf('a', '工作流 A'), wf('b', '工作流 B')])
    render(<WorkflowSwitcher currentId="a" onSwitch={vi.fn()} />)

    expect(listWorkflows).not.toHaveBeenCalled()
    expect(document.querySelector('.wfc-wf-menu')).toBeNull()

    fireEvent.click(trigger())

    await waitFor(() => expect(screen.getByText('工作流 B')).toBeInTheDocument())
    expect(listWorkflows).toHaveBeenCalledTimes(1)
    expect(screen.getByText('工作流 A')).toBeInTheDocument()
    // 当前工作流只标记不隐藏
    expect(screen.getByText('当前')).toBeInTheDocument()
  })

  it('点其它工作流 → 回调其 id；点当前项不回调', async () => {
    listWorkflows.mockResolvedValue([wf('a', '工作流 A'), wf('b', '工作流 B')])
    const onSwitch = vi.fn()
    render(<WorkflowSwitcher currentId="a" onSwitch={onSwitch} />)

    fireEvent.click(trigger())
    await waitFor(() => expect(screen.getByText('工作流 B')).toBeInTheDocument())
    fireEvent.click(screen.getByText('工作流 B'))
    expect(onSwitch).toHaveBeenCalledWith('b')

    // 菜单已收起 → 重新展开再点当前项
    fireEvent.click(trigger())
    await waitFor(() => expect(screen.getByText('工作流 A')).toBeInTheDocument())
    fireEvent.click(screen.getByText('工作流 A'))
    expect(onSwitch).toHaveBeenCalledTimes(1)
  })

  it('禁用态（运行中 / 回放中）不展开、不查列表', () => {
    render(
      <WorkflowSwitcher
        currentId="a"
        onSwitch={vi.fn()}
        disabled
        disabledHint="运行中 · 画布只读"
      />,
    )

    expect(trigger()).toBeDisabled()
    expect(trigger()).toHaveAttribute('title', '运行中 · 画布只读')
    fireEvent.click(trigger())
    expect(listWorkflows).not.toHaveBeenCalled()
    expect(document.querySelector('.wfc-wf-menu')).toBeNull()
  })

  it('列表查询失败 → 落「没有其它工作流」，不留永久加载中', async () => {
    listWorkflows.mockRejectedValue(new Error('boom'))
    render(<WorkflowSwitcher currentId="a" onSwitch={vi.fn()} />)

    fireEvent.click(trigger())

    await waitFor(() => expect(screen.getByText('没有其它工作流')).toBeInTheDocument())
  })
})
