import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { ProblemsPanel } from './ProblemsPanel'

describe('workflow bottom panel', () => {
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
