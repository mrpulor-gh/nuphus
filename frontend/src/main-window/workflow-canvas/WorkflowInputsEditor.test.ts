import { describe, expect, it } from 'vitest'
import { changeInputKind, inputValueMatchesKind } from './WorkflowInputsEditor'

describe('external input declaration defaults', () => {
  it('checks every contract kind without coercion', () => {
    expect(inputValueMatchesKind('string', '1')).toBe(true)
    expect(inputValueMatchesKind('path', 'C:/tmp')).toBe(true)
    expect(inputValueMatchesKind('number', 1)).toBe(true)
    expect(inputValueMatchesKind('number', '1')).toBe(false)
    expect(inputValueMatchesKind('boolean', false)).toBe(true)
    expect(inputValueMatchesKind('boolean', 0)).toBe(false)
    expect(inputValueMatchesKind('json', null)).toBe(true)
  })

  it('clears an incompatible default when the kind changes', () => {
    expect(changeInputKind({ name: 'count', type: 'string', default: '1' }, 'number')).toEqual({
      name: 'count',
      type: 'number',
    })
    expect(changeInputKind({ name: 'count', type: 'number', default: 1 }, 'json')).toEqual({
      name: 'count',
      type: 'json',
      default: 1,
    })
  })
})
