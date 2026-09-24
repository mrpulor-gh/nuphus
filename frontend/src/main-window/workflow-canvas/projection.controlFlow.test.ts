import { describe, expect, it } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { projectWorkflow } from './projection'
import { buildVariableCatalogIndex } from './variableCatalog'

const output = (id: string, capture = id): WorkflowStep => ({
  id,
  name: id,
  capture,
  do: { tool: 'test' },
})
const consume = (id: string, template: string): WorkflowStep => ({
  id,
  name: id,
  do: { tool: 'read', with: { value: template } },
})
const branch = (then: WorkflowStep[], otherwise: WorkflowStep[] = []): WorkflowStep => ({
  id: 'branch',
  name: '分支',
  do: { if: { condition: { not_empty: { var: 'inputs.topic' } }, then, else: otherwise } },
})

describe('control-flow-aware variable projection', () => {
  it('never connects a sibling branch producer to the other branch reader', () => {
    const result = projectWorkflow({
      steps: [branch([output('left', 'shared')], [consume('right', '{{shared}}')])],
    })
    const layer = result.layers.get('branch')!
    expect(layer.edges.some(e => e.source === 'left' && e.target === 'right')).toBe(false)
    expect(layer.edges.find(e => e.target === 'right' && e.kind === 'external')?.dangling).toBe(
      true,
    )
    expect(layer.nodes.some(n => n.id === 'branch::out::shared')).toBe(false)
  })
  it('retains both branch producers and marks conditional output rather than inventing one last source', () => {
    const steps = [
      branch([output('left', 'shared')], [output('right', 'shared'), output('optional')]),
      consume('after', '{{shared}} {{optional}}'),
    ]
    const source = buildVariableCatalogIndex(steps).beforeStep.get('after')!.get('shared')!
    expect(source.producerStepIds).toEqual(['left', 'right'])
    const result = projectWorkflow({ steps })
    const shared = result.layers
      .get('root')!
      .edges.find(e => e.target === 'after' && e.label === 'shared')!
    expect(shared.sourceSummary).toContain('left')
    expect(shared.sourceSummary).toContain('right')
    expect(shared.maybeUnset).toBe(false)
    expect(result.layers.get('root')!.edges.find(e => e.label === 'optional')?.maybeUnset).toBe(
      true,
    )
    expect(
      result.layers
        .get('branch')!
        .edges.filter(e => e.target === 'branch::out::shared')
        .map(e => e.source),
    ).toEqual(['left', 'right'])
  })
  it('keeps explicit inputs separate from same-named captures and shows valid inputs without missing-source warnings', () => {
    const result = projectWorkflow({
      inputs: [{ name: 'topic', required: true }],
      steps: [output('capture', 'topic'), consume('after', '{{topic}} {{inputs.topic}}')],
    })
    const layer = result.layers.get('root')!
    expect(layer.edges.find(e => e.label === 'topic')).toMatchObject({
      source: 'capture',
      producerStepId: 'capture',
    })
    expect(layer.edges.find(e => e.label === 'inputs.topic')).toMatchObject({
      source: 'root::external::inputs.topic',
      dangling: false,
    })
    expect(result.index.nodeById.get('after')!.danglingVars).toBeUndefined()
  })
  it('does not create fake item locals for repeat or until, nor a fake index for until', () => {
    const result = projectWorkflow({
      steps: [
        {
          id: 'repeat',
          name: '重复',
          do: { loop: { repeat: 2, do: [consume('readRepeat', '{{item}} {{_index}}')] } },
        },
        {
          id: 'until',
          name: '直到',
          do: {
            loop: { until: { always: false }, do: [consume('readUntil', '{{item}} {{_index}}')] },
          },
        },
      ],
    })
    expect(result.layers.get('repeat')!.edges.find(e => e.label === '_index')?.source).toBe(
      'repeat::entry',
    )
    expect(result.layers.get('repeat')!.edges.find(e => e.label === 'item')?.dangling).toBe(true)
    expect(
      result.layers
        .get('until')!
        .edges.filter(e => e.kind === 'external')
        .every(e => e.dangling),
    ).toBe(true)
    expect(result.layers.get('until')!.nodes.find(n => n.synthetic === 'entry')?.name).toBe('入口')
  })
  it('keeps scoped captures ahead of loop locals after the local variable is overwritten', () => {
    const result = projectWorkflow({
      steps: [
        {
          id: 'loop',
          name: '遍历',
          do: {
            loop: {
              for_each: { items: { var: 'inputs.items' }, as: 'row' },
              do: [output('replace', 'row'), consume('read', '{{row}}')],
            },
          },
        },
      ],
    })
    expect(result.layers.get('loop')!.edges.find(e => e.label === 'row')).toMatchObject({
      source: 'replace',
      target: 'read',
    })
  })
  it('does not treat unsupported capture fields on call as producers', () => {
    const result = projectWorkflow({
      steps: [
        { id: 'call', name: '调用', capture: 'result', do: { call: 'sub' } },
        consume('read', '{{result}}'),
      ],
    })
    expect(result.layers.get('root')!.edges.find(e => e.label === 'result')?.dangling).toBe(true)
  })
  it('uses unique edge ids when both an input alias and its explicit namespace are referenced', () => {
    const result = projectWorkflow({
      inputs: [{ name: 'topic', required: true }],
      steps: [consume('read', '{{topic}} {{inputs.topic}} {{missing}} {{inputs.missing}}')],
    })
    const ids = result.layers.get('root')!.edges.map(e => e.id)
    expect(new Set(ids).size).toBe(ids.length)
  })
})
