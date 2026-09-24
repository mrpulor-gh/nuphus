import { describe, expect, it } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { buildVariableCatalog } from './variableCatalog'

const output = (id: string, capture = id): WorkflowStep => ({
  id,
  name: id,
  capture,
  do: { tool: 'test' },
})
const reader: WorkflowStep = { id: 'read', name: 'read', do: { sleep: 1 } }
const refs = (steps: WorkflowStep[], current = 'read') =>
  buildVariableCatalog(steps, [], current).references

describe('control-flow variable catalog', () => {
  it('offers only earlier outputs, namespaces inputs, and never exposes input values', () => {
    const catalog = buildVariableCatalog(
      [output('before'), { ...reader, capture: 'self' }, output('future')],
      [{ name: 'secret', sensitive: true, default: 'never include me' }],
      'read',
    )
    expect(catalog.references.map(v => v.name)).toEqual(['inputs.secret', 'before'])
    expect(JSON.stringify(catalog)).not.toContain('never include me')
    expect(catalog.captures.map(v => v.name)).toEqual(['before', 'future'])
  })
  it('does not offer sibling branch variables inside the other branch', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'if',
        name: '分支',
        do: { if: { condition: { always: true }, then: [output('left')], else: [reader] } },
      },
    ]
    expect(refs(steps)).toEqual([])
    const after = refs([...steps, { ...reader, id: 'after' }], 'after')
    expect(after).toEqual([expect.objectContaining({ name: 'left', maybeUnset: true })])
  })
  it('merges guaranteed outputs from both branches and handles old values retained on one path', () => {
    const steps: WorkflowStep[] = [
      output('old', 'shared'),
      {
        id: 'if',
        name: '分支',
        do: {
          if: {
            condition: { always: false },
            then: [output('left', 'both'), output('new', 'shared')],
            else: [output('right', 'both')],
          },
        },
      },
      reader,
    ]
    expect(refs(steps).find(v => v.name === 'both')?.maybeUnset).toBe(false)
    expect(refs(steps).find(v => v.name === 'shared')?.maybeUnset).toBe(false)
  })
  it('offers for-each locals only in the loop, and marks exported loop output conditional', () => {
    const loop: WorkflowStep = {
      id: 'loop',
      name: '遍历',
      do: {
        loop: { for_each: { items: { var: 'items' }, as: 'row' }, do: [reader, output('result')] },
      },
    }
    expect(refs([loop]).map(v => v.name)).toEqual(['row', '_index'])
    expect(refs([loop, { ...reader, id: 'after' }], 'after')).toEqual([
      expect.objectContaining({ name: 'result', maybeUnset: true }),
    ])
  })
  it('does not invent item or _index locals for until loops', () => {
    const loop: WorkflowStep = {
      id: 'loop',
      name: '直到',
      do: { loop: { until: { always: false }, do: [reader] } },
    }
    expect(refs([loop])).toEqual([])
  })
  it('exports sequential outputs and warns on skipped producers or optional wait automation', () => {
    const seq: WorkflowStep = {
      id: 'seq',
      name: '顺序',
      do: { seq: [output('first'), { ...output('skip'), on_error: 'skip' }] },
    }
    const wait: WorkflowStep = {
      id: 'wait',
      name: '等待',
      do: { wait: 'ok', auto: [output('auto')] },
    }
    expect(refs([seq, wait, reader]).map(v => [v.name, v.maybeUnset])).toEqual([
      ['first', false],
      ['skip', true],
      ['auto', true],
    ])
  })
  it('deduplicates output choices with producer descriptions and ignores inert call capture', () => {
    const catalog = buildVariableCatalog([
      output('a', 'shared'),
      output('b', 'shared'),
      { id: 'call', name: '调用', capture: 'ignored', do: { call: 'other' } },
    ])
    expect(catalog.captures).toHaveLength(1)
    expect(catalog.captures[0].sourceLabel).toBe('a / b')
  })
})
