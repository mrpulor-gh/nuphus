import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ProblemsPanel } from './ProblemsPanel'
import { PROBLEMS_HEIGHT_KEY } from './useProblemsResize'
import { LangProvider } from '../../locales'

const emptyProps = {
  problems: [],
  backendReport: null,
  timeline: [],
  running: false,
  runHistory: [],
  onLocate: vi.fn(),
  nameOf: (id: string) => id,
}
beforeEach(() => {
  localStorage.removeItem(PROBLEMS_HEIGHT_KEY)
  localStorage.removeItem('nuphus_language')
})

describe('workflow bottom panel', () => {
  it('shows a single summary and keeps expanded details when issues reorder', () => {
    const problems = ['contact', 'message'].map(subject => ({
      rule: 'input_reference',
      subject,
      level: 'error' as const,
      message: `raw ${subject}`,
      stepId: 'node',
      fieldPath: '/do/with/text',
    }))
    const { rerender } = render(<ProblemsPanel {...emptyProps} problems={problems} />)
    expect(screen.getByText('外部输入 contact 尚未声明')).toBeInTheDocument()
    expect(screen.queryByText('raw contact')).not.toBeInTheDocument()
    fireEvent.click(screen.getAllByRole('button', { name: '展开问题详情' })[0])
    expect(screen.getByText('raw contact')).toBeInTheDocument()
    rerender(<ProblemsPanel {...emptyProps} problems={[...problems].reverse()} />)
    expect(screen.getByText('raw contact')).toBeInTheDocument()
    expect(screen.queryByText('raw message')).not.toBeInTheDocument()
    fireEvent.click(screen.getAllByTitle('定位并修改')[0])
    expect(emptyProps.onLocate).toHaveBeenCalledWith('node', '/do/with/text')
  })
  it('resizes by keyboard, persists expansion height and resets on double click', () => {
    render(<ProblemsPanel {...emptyProps} />)
    const handle = screen.getByRole('separator')
    expect(handle).toHaveAttribute('aria-valuenow', '220')
    fireEvent.keyDown(handle, { key: 'ArrowUp' })
    expect(handle).toHaveAttribute('aria-valuenow', '240')
    expect(localStorage.getItem(PROBLEMS_HEIGHT_KEY)).toBe('240')
    fireEvent.click(screen.getByRole('button', { name: '收起底部面板' }))
    expect(screen.queryByRole('separator')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '展开底部面板' }))
    expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '240')
    fireEvent.doubleClick(screen.getByRole('separator'))
    expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '220')
  })
  it('captures pointer drags without propagating to the canvas, including cancellation', () => {
    const parent = vi.fn()
    render(
      <div onPointerDown={parent} onPointerMove={parent}>
        <ProblemsPanel {...emptyProps} />
      </div>,
    )
    const handle = screen.getByRole('separator')
    handle.setPointerCapture = vi.fn()
    handle.hasPointerCapture = () => true
    handle.releasePointerCapture = vi.fn()
    const pointer = (type: string, y: number) => {
      const event = new MouseEvent(type, { bubbles: true, button: 0, clientY: y })
      Object.defineProperty(event, 'pointerId', { value: 7 })
      fireEvent(handle, event)
    }
    pointer('pointerdown', 400)
    pointer('pointermove', 300)
    expect(handle).toHaveAttribute('aria-valuenow', '320')
    expect(handle.setPointerCapture).toHaveBeenCalledWith(7)
    expect(parent).not.toHaveBeenCalled()
    pointer('pointercancel', 300)
    pointer('pointermove', 250)
    expect(handle).toHaveAttribute('aria-valuenow', '320')
    expect(handle.releasePointerCapture).toHaveBeenCalledWith(7)
  })
  it('provides English summaries and resize controls', () => {
    localStorage.setItem('nuphus_language', 'en')
    render(
      <LangProvider>
        <ProblemsPanel
          {...emptyProps}
          problems={[
            { rule: 'input_reference', subject: 'contact', message: 'raw', level: 'error' },
          ]}
        />
      </LangProvider>,
    )
    expect(screen.getByText('External input contact is not declared')).toBeInTheDocument()
    expect(screen.getByRole('separator', { name: 'Resize bottom panel' })).toBeInTheDocument()
  })
  it('switches to the live log when a run starts', () => {
    const props = {
      problems: [],
      backendReport: null,
      timeline: [
        {
          id: 1,
          at: Date.now(),
          event: 'step_run_started',
          level: 'info' as const,
          message: '开始 · 读取文件',
          stepId: 'read',
        },
      ],
      runHistory: [],
      onLocate: () => undefined,
      nameOf: (id: string) => id,
    }
    const { rerender } = render(<ProblemsPanel {...props} running={false} />)
    rerender(<ProblemsPanel {...props} running />)
    expect(screen.getByText('开始 · 读取文件')).toBeInTheDocument()
    expect(screen.getByText('运行中')).toBeInTheDocument()
  })

  it('shows the latest persisted output summary when no live run exists', () => {
    render(
      <ProblemsPanel
        problems={[]}
        backendReport={null}
        timeline={[]}
        running={false}
        runHistory={[
          {
            run_id: 'run-1',
            started_at: '2026-09-21T03:00:00Z',
            finished_at: '2026-09-21T03:00:01Z',
            status: 'Success',
            steps: [
              {
                step_id: 'read',
                started_at: '2026-09-21T03:00:00Z',
                finished_at: '2026-09-21T03:00:01Z',
                status: 'Success',
                output_summary: '读取完成',
              },
            ],
          },
        ]}
        onLocate={() => undefined}
        nameOf={() => '读取文件'}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /运行日志/ }))
    expect(screen.getByText('读取文件 · 读取完成')).toBeInTheDocument()
  })
})
