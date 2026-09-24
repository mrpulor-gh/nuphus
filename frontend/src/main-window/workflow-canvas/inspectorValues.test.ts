import { describe, expect, it } from 'vitest'
import {
  changeConditionOperator,
  numberError,
  parseExitCodes,
  switchLoopMode,
} from './inspectorValues'

describe('inspector value validation', () => {
  it('rejects partial, empty and nonfinite numbers without silently replacing values', () => {
    for (const value of ['', 'abc', 'Infinity', '1x', '0x20'])
      expect(numberError(value)).not.toBeNull()
    expect(numberError('', { optional: true })).toBeNull()
    expect(numberError('0.5', { integer: true, min: 1 })).not.toBeNull()
    expect(numberError('0', { integer: true, min: 0 })).toBeNull()
    expect(numberError('3e2', { integer: true })).toBeNull()
  })
  it('accepts complete signed exit codes and deduplicates', () => {
    expect(parseExitCodes('0, -1, +3, 0')).toEqual([0, -1, 3])
    for (const value of ['', '0,', ',1', '1.5', '1e2', '2147483648'])
      expect(parseExitCodes(value)).toBeNull()
  })
  it('distinguishes true from false and keeps compatible operands', () => {
    expect(changeConditionOperator('always_false', [])).toEqual({ always: false })
    expect(changeConditionOperator('equals', [{ var: 'result' }, 'ok'])).toEqual({
      equals: [{ var: 'result' }, 'ok'],
    })
    expect(changeConditionOperator('not_empty', [{ var: 'result' }])).toEqual({
      not_empty: { var: 'result' },
    })
  })
  it('switches loop mode without dropping children or extension fields', () => {
    const def = { repeat: 3, do: [{ id: 'child' }], max: 8, extension: true }
    const next = switchLoopMode(def, 'until', {})
    expect(next).toEqual({ do: def.do, max: 8, extension: true, until: { always: false } })
    expect(switchLoopMode(next, 'repeat', { repeat: 3 })).toEqual(def)
  })
})
