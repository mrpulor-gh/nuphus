import { describe, expect, it } from 'vitest'
import en from '../../locales/en'
import zh from '../../locales/zh'
import { actionSummary } from './actionSummary'
import type { WorkflowStep } from '../../core/types'

const step = (action: WorkflowStep['do']): WorkflowStep => ({
  id: 'n',
  name: 'User-authored name',
  do: action,
})
const translate =
  (pack: Record<string, string>) =>
  (key: string, ...values: string[]) =>
    values.reduce((text, value, i) => text.replace(`{${i}}`, value), pack[key] ?? key)

describe('deterministic action summaries', () => {
  it('uses actual delay and window lookup arguments in either language', () => {
    expect(actionSummary(step({ sleep: 2 }), translate(en))).toContain('2')
    expect(
      actionSummary(
        step({ tool: 'desktop_targets_list', with: { query: 'WeChat' } }),
        translate(zh),
      ),
    ).toBe('查找应用 / 窗口：WeChat')
    expect(
      actionSummary(
        step({ tool: 'desktop_targets_list', with: { query: 'WeChat' } }),
        translate(en),
      ),
    ).toBe('Find app / window: WeChat')
  })
  it('does not invent descriptions or display arbitrary secret/message parameters', () => {
    expect(
      actionSummary(
        step({ tool: 'custom_tool', with: { message: 'private', api_key: 'secret' } }),
        translate(en),
      ),
    ).toBe('custom_tool')
    expect(
      actionSummary(
        step({ tool: 'desktop_targets_list', with: { query: { unexpected: true } } }),
        translate(en),
      ),
    ).toBe('desktop_targets_list')
  })
})
