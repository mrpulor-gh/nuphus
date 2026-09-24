import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { TimelineEntry } from '../../core/types'
import { ExecutionTraceFloating } from './ExecutionTraceFloating'

vi.mock('../chat/MarkdownContent', () => ({
  default: ({ content }: { content: string }) => <div>{content}</div>,
}))
vi.mock('../../ui/NuphusAvatar', () => ({ NuphusAvatar: () => <span /> }))

function renderAction(overrides: Partial<TimelineEntry> = {}) {
  const entry: TimelineEntry = {
    id: 'desktop-action',
    kind: 'tool_call',
    toolName: 'desktop_semantic_execute',
    status: 'success',
    output: JSON.stringify({
      dispatch_state: 'sent',
      effect: 'unverifiable',
      business_goal_confirmed: false,
    }),
    ...overrides,
  }
  return render(
    <ExecutionTraceFloating
      timeline={[entry]}
      stepIndex={1}
      progress={{ iteration: 1, max: 20, calls: 1 }}
      isProcessing={false}
      completed={false}
      expandedCalls={new Set()}
      onToggleExpand={vi.fn()}
      visible
    />,
  )
}

describe('desktop action execution trace', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'requestAnimationFrame',
      vi.fn(() => 1),
    )
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('shows delivery and uncertain effect instead of a completed chip', () => {
    const { container } = renderAction()
    expect(screen.getByText('已发送 · 效果未确认')).toHaveAttribute(
      'data-action-state',
      'unverifiable',
    )
    expect(container.querySelector('.tc-status-chip')).not.toBeInTheDocument()
    expect(screen.queryByText('完成', { exact: true })).not.toBeInTheDocument()
  })

  it('terminal mode does not display exit 0 as proof of action effect', () => {
    renderAction()
    fireEvent.click(screen.getByTitle('终端模式'))
    expect(screen.getByText('已发送 · 效果未确认')).toBeInTheDocument()
    expect(screen.queryByText('exit 0')).not.toBeInTheDocument()
  })

  it('clarifies that confirmed UI state is not whole-task completion', () => {
    renderAction({ output: '{"dispatch_state":"sent","effect":"confirmed"}' })
    expect(screen.getByText('已发送 · 状态已确认')).toHaveAttribute(
      'title',
      '已确认指定界面状态，不代表整个业务任务已完成。',
    )
  })

  it('preserves a partial native failure instead of a generic error chip', () => {
    renderAction({
      status: 'error',
      output: 'desktop_action_result:{"dispatch_state":"partial","effect":"partial"}',
    })
    expect(screen.getByText('部分发送')).toHaveAttribute('data-dispatch-state', 'partial')
    expect(screen.queryByText('失败', { exact: true })).not.toBeInTheDocument()
  })

  it('does not report completion when an action preview is truncated', () => {
    renderAction({ output: '{"dispatch_state":"sent","receipt":', isTruncated: true })
    expect(screen.getByText('发送状态不明')).toBeInTheDocument()
    expect(screen.queryByText('完成', { exact: true })).not.toBeInTheDocument()
  })

  it('leaves running tools and observation tools on their existing status path', () => {
    const running = renderAction({ status: 'running' })
    expect(running.container.querySelector('.desktop-action-status')).not.toBeInTheDocument()
    expect(running.container.querySelector('.tc-status-chip.running')).toBeInTheDocument()
    running.unmount()
    const observation = renderAction({ toolName: 'desktop_windows_list', output: '[]' })
    expect(observation.container.querySelector('.desktop-action-status')).not.toBeInTheDocument()
    expect(observation.container.querySelector('.tc-status-chip.success')).toBeInTheDocument()
  })
})
