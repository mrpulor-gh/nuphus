import { describe, expect, it } from 'vitest'
import type { LoopDef, WorkflowStep } from '../../core/types'
import { applyOp, checkOp } from './irEdit'
import type { IrEditOp } from './types'
import { buildVariableCatalogIndex } from './variableCatalog'

const pause = (id: string): WorkflowStep => ({ id, name: id, do: { sleep: 1 } })
const reader = (code: string): WorkflowStep => ({
  id: 'read',
  name: 'Read current item',
  do: { script: { runtime: 'python', code } },
})
const group = (id: string, children: WorkflowStep[]): WorkflowStep => ({
  id,
  name: id,
  do: { seq: children },
})
const loop = (id: string, def: LoopDef): WorkflowStep => ({ id, name: id, do: { loop: def } })
const forEach = (id: string, children: WorkflowStep[], itemName?: string): WorkflowStep =>
  loop(id, { for_each: { items: '["a", "b"]', as: itemName }, do: children })

function fixture(itemName: string | undefined): WorkflowStep[] {
  return [
    forEach(
      'rows',
      [
        group('group', [reader(`print("{{${itemName ?? 'item'}}}")`), pause('keep-group')]),
        group('other-group', [pause('keep-other-group')]),
      ],
      itemName,
    ),
  ]
}

function moveTo(layerId: string): IrEditOp {
  return {
    op: 'move_step',
    stepId: 'read',
    to: { parent: { layerId }, lane: 'main', index: 1 },
  }
}

describe('canvas moves preserve loop item scope', () => {
  it.each(['row', undefined])(
    'moves a nested step back to its loop body with item %s',
    itemName => {
      const steps = fixture(itemName)
      const op = moveTo('rows')

      expect(checkOp(steps, op)).toEqual({ ok: true })
      const moved = applyOp(steps, op)
      expect(
        buildVariableCatalogIndex(moved)
          .beforeStep.get('read')
          ?.get(itemName ?? 'item'),
      ).toMatchObject({ source: 'loop', stepId: 'rows' })
    },
  )

  it('moves to another for_each loop that declares the same item name', () => {
    const steps = [...fixture('row'), forEach('other-rows', [pause('keep-other-rows')], 'row')]

    expect(checkOp(steps, moveTo('other-rows'))).toEqual({ ok: true })
  })

  it('keeps both target and ancestor loop items available in a nested loop body', () => {
    const steps = [
      forEach(
        'rows',
        [
          forEach(
            'cells',
            [group('group', [reader('print("{{row}} {{cell}}")'), pause('keep-group')])],
            'cell',
          ),
        ],
        'row',
      ),
    ]

    expect(checkOp(steps, moveTo('cells'))).toEqual({ ok: true })
    expect(checkOp(steps, moveTo('rows'))).toMatchObject({
      ok: false,
      reason: expect.stringContaining('{{cell}}'),
    })
  })

  it.each(['repeat', 'until'] as const)(
    'inherits the outer item through a target %s loop',
    mode => {
      const target = loop('target', {
        ...(mode === 'repeat' ? { repeat: 2 } : { until: { always: false } }),
        do: [pause('keep-target')],
      })
      const steps = [
        forEach(
          'rows',
          [group('group', [reader('print("{{row}}")'), pause('keep-group')]), target],
          'row',
        ),
      ]

      expect(checkOp(steps, moveTo('target'))).toEqual({ ok: true })
    },
  )

  it.each(['repeat', 'until'] as const)(
    'does not invent an item binding in a target %s loop',
    mode => {
      const steps = [
        ...fixture(undefined),
        loop('target', {
          ...(mode === 'repeat' ? { repeat: 2 } : { until: { always: false } }),
          do: [group('target-group', [pause('keep-target')])],
        }),
      ]

      for (const layerId of ['target', 'target-group']) {
        expect(checkOp(steps, moveTo(layerId))).toMatchObject({
          ok: false,
          reason: expect.stringContaining('{{item}}'),
        })
      }
    },
  )

  it('allows a move with no loop item reference', () => {
    const steps = [
      forEach('rows', [group('group', [reader('print("literal")'), pause('keep')])], 'row'),
    ]

    expect(checkOp(steps, moveTo('rows'))).toEqual({ ok: true })
  })

  it('allows moving to another nested group inside the same loop', () => {
    expect(checkOp(fixture('row'), moveTo('other-group'))).toEqual({ ok: true })
  })

  it('rejects moving an item reference outside all loops', () => {
    expect(checkOp(fixture('row'), moveTo('root'))).toMatchObject({
      ok: false,
      reason: expect.stringContaining('{{row}}'),
    })
  })

  it('rejects moving to a loop with a different item name', () => {
    const steps = [...fixture('row'), forEach('other-items', [pause('keep-other-items')], 'item')]

    expect(checkOp(steps, moveTo('other-items'))).toMatchObject({
      ok: false,
      reason: expect.stringContaining('{{row}}'),
    })
  })

  it('moves an entire loop without treating its own item as an outer dependency', () => {
    const steps = [
      forEach('rows', [reader('print("{{row}}")')], 'row'),
      group('target', [pause('keep')]),
    ]

    expect(
      checkOp(steps, {
        op: 'move_step',
        stepId: 'rows',
        to: { parent: { layerId: 'target' }, lane: 'main', index: 1 },
      }),
    ).toEqual({ ok: true })
  })
})
