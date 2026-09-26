import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ScheduleConfig } from '../../core/types'
import { wfScheduleGet, wfSchedulePreview, wfScheduleSet } from '../lib/api'
import { WorkflowScheduleDialog } from './WorkflowScheduleDialog'

vi.mock('../lib/api', () => ({
  wfScheduleGet: vi.fn(),
  wfSchedulePreview: vi.fn(),
  wfScheduleSet: vi.fn(),
  wfScheduleRemove: vi.fn(),
}))

beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(wfSchedulePreview).mockResolvedValue([])
  vi.mocked(wfScheduleSet).mockResolvedValue(undefined)
})

describe('WorkflowScheduleDialog 定时配置保留', () => {
  it.each([29, 30, 31])('重新打开每月 %i 日的定时规则，只改标签不会改动执行日期', async day => {
    const config: ScheduleConfig = {
      cron: `0 9 ${day} * *`,
      timezone: 'Asia/Shanghai',
      enabled: true,
      label: '月末报表',
    }
    vi.mocked(wfScheduleGet).mockResolvedValue({
      config,
      inputs: {},
      sensitive_inputs: [],
      eligible: true,
    })
    const onChanged = vi.fn()
    const onClose = vi.fn()

    render(
      <WorkflowScheduleDialog
        open
        workflow={{ id: 'monthly-report', title: '月末报表' }}
        onChanged={onChanged}
        onClose={onClose}
      />,
    )

    fireEvent.change(await screen.findByLabelText('标签'), {
      target: { value: '每月归档' },
    })
    const expectedConfig = { ...config, label: '每月归档' }
    await waitFor(() => expect(wfSchedulePreview).toHaveBeenCalled())

    fireEvent.click(screen.getByRole('button', { name: '保存并应用' }))

    await waitFor(() => {
      expect(wfScheduleSet).toHaveBeenCalledWith('monthly-report', expectedConfig, {}, [])
      expect(onChanged).toHaveBeenCalledWith(expectedConfig)
      expect(onClose).toHaveBeenCalledTimes(1)
    })
    await waitFor(() => expect(wfSchedulePreview).toHaveBeenCalledWith(expectedConfig))
    expect(screen.getByLabelText('频率')).toHaveValue('custom')
    expect(screen.getByLabelText('Cron（分 时 日 月 周）')).toHaveValue(config.cron)
  })
})
