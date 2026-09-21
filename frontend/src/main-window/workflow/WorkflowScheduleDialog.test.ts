import { describe, expect, it } from 'vitest'
import type { WorkflowInputSpec } from '../../core/types'
import {
  cronFromPattern,
  patternFromCron,
  resolveScheduleInputs,
  type SchedulePattern,
} from './WorkflowScheduleDialog'

const DAILY: SchedulePattern = {
  mode: 'daily',
  interval: 5,
  hour: 9,
  minute: 30,
  weekday: 1,
  monthDay: 1,
  custom: '0 9 * * *',
}

describe('定时规则预设', () => {
  it.each([
    [{ ...DAILY, mode: 'minutes', interval: 90 }, '*/60 * * * *'],
    [{ ...DAILY, mode: 'hourly' }, '30 * * * *'],
    [DAILY, '30 9 * * *'],
    [{ ...DAILY, mode: 'weekdays' }, '30 9 * * 1-5'],
    [{ ...DAILY, mode: 'weekly', weekday: 0 }, '30 9 * * 0'],
    [{ ...DAILY, mode: 'monthly', monthDay: 12 }, '30 9 12 * *'],
    [{ ...DAILY, mode: 'custom', custom: '  5 4 * * 2  ' }, '5 4 * * 2'],
  ] as const)('把表单模式转换成五字段 Cron', (pattern, expected) => {
    expect(cronFromPattern(pattern)).toBe(expected)
  })

  it.each([
    ['*/10 * * * *', 'minutes'],
    ['5 * * * *', 'hourly'],
    ['5 8 * * *', 'daily'],
    ['5 8 * * 1-5', 'weekdays'],
    ['5 8 * * 6', 'weekly'],
    ['5 8 20 * *', 'monthly'],
    ['1,15 8 * * *', 'custom'],
  ] as const)('识别已有 Cron 的可编辑模式', (cron, mode) => {
    const pattern = patternFromCron(cron)
    expect(pattern.mode).toBe(mode)
    expect(cronFromPattern(pattern)).toBe(cron)
  })
})

describe('定时输入快照', () => {
  it('未固定的可选输入不固化默认值', () => {
    const specs: WorkflowInputSpec[] = [{ name: 'topic', default: '当前默认值' }]
    expect(resolveScheduleInputs(specs, {}, {}, new Set())).toEqual({
      inputs: {},
      preserveSensitive: [],
      errors: {},
    })
  })

  it('必填且没有默认值的输入必须固定', () => {
    const specs: WorkflowInputSpec[] = [{ name: 'topic', required: true }]
    expect(resolveScheduleInputs(specs, {}, {}, new Set()).errors).toEqual({
      topic: '必填输入必须固定一个值',
    })
  })

  it('固定的 boolean false 会明确写入快照', () => {
    const specs: WorkflowInputSpec[] = [{ name: 'enabled', type: 'boolean' }]
    expect(
      resolveScheduleInputs(specs, { enabled: true }, { enabled: false }, new Set()).inputs,
    ).toEqual({ enabled: false })
  })

  it('解析 number 和 JSON，并报告无效值', () => {
    const specs: WorkflowInputSpec[] = [
      { name: 'count', type: 'number' },
      { name: 'payload', type: 'json' },
    ]
    const valid = resolveScheduleInputs(
      specs,
      { count: true, payload: true },
      { count: '2.5', payload: '{"items":[1,2]}' },
      new Set(),
    )
    expect(valid.inputs).toEqual({ count: 2.5, payload: { items: [1, 2] } })
    expect(valid.errors).toEqual({})

    const invalid = resolveScheduleInputs(
      specs,
      { count: true, payload: true },
      { count: '', payload: '{oops' },
      new Set(),
    )
    expect(invalid.errors).toEqual({
      count: '请输入有效数字',
      payload: 'JSON 格式无效',
    })
  })

  it('敏感值可保持、重填或通过取消固定清除', () => {
    const specs: WorkflowInputSpec[] = [{ name: 'token', sensitive: true }]

    const preserved = resolveScheduleInputs(
      specs,
      { token: true },
      { token: '' },
      new Set(['token']),
    )
    expect(preserved).toEqual({ inputs: {}, preserveSensitive: ['token'], errors: {} })

    const replaced = resolveScheduleInputs(
      specs,
      { token: true },
      { token: 'new-secret' },
      new Set(),
    )
    expect(replaced).toEqual({
      inputs: { token: 'new-secret' },
      preserveSensitive: [],
      errors: {},
    })

    const cleared = resolveScheduleInputs(
      specs,
      { token: false },
      { token: '' },
      new Set(),
    )
    expect(cleared).toEqual({ inputs: {}, preserveSensitive: [], errors: {} })
  })
})
