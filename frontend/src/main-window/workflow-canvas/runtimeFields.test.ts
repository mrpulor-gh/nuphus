import { describe, expect, it } from 'vitest'
import { runtimeChildren, runtimeRoots } from './runtimeFields'
import { parseFieldReference } from './fieldReferences'
import type { VariableCatalog } from './variableCatalog'

const catalog: VariableCatalog = {
  references: [
    { name: 'result.v2', source: 'capture', sourceLabel: 'producer', maybeUnset: false },
    { name: 'text', source: 'capture', sourceLabel: 'script', maybeUnset: false },
    { name: 'inputs.config', source: 'input', sourceLabel: 'input', maybeUnset: false },
    { name: 'missing', source: 'capture', sourceLabel: 'branch', maybeUnset: true },
  ],
  captures: [],
}

describe('runtime field references', () => {
  it('uses only present runtime values in the current static scope', () => {
    const roots = runtimeRoots(
      { 'result.v2': null, text: '', inputs: { config: false }, hidden: { secret: 1 } },
      catalog,
    )
    expect(roots.map(field => [field.expression, field.value])).toEqual([
      ['result.v2', null],
      ['text', ''],
      ['inputs["config"]', false],
    ])
  })

  it('preserves literal dots, quotes, bracket characters and numeric object keys', () => {
    const roots = runtimeRoots(
      { 'result.v2': { 'a.b': [{ 'quote" ] }}': { '0': false } }] } },
      catalog,
    )
    const field = runtimeChildren(
      runtimeChildren(runtimeChildren(runtimeChildren(roots[0])[0])[0])[0],
    )[0]
    expect(field.expression).toBe('result.v2["a.b"][0]["quote\\\" ] }}"]["0"]')
    expect(parseFieldReference(field.expression)).toEqual({
      root: 'result.v2',
      segments: ['a.b', 0, 'quote" ] }}', '0'],
    })
    expect(field.value).toBe(false)
  })

  it('does not infer children from JSON-looking strings or empty containers', () => {
    const roots = runtimeRoots(
      { text: '{"guessed":1}', 'result.v2': [], inputs: { config: {} } },
      catalog,
    )
    expect(roots.map(runtimeChildren)).toEqual([[], [], []])
  })

  it('does not read inherited properties as observed runtime fields', () => {
    expect(runtimeRoots(Object.create({ text: 'inherited' }), catalog)).toEqual([])
  })
})
