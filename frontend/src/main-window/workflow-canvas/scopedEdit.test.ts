import { describe, expect, it } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import {
  applyScopedEdit,
  createScopedEditRequest,
  scopedEditDiff,
  scopedOwnStep,
  scopedRevision,
  type ScopedEditProposal,
} from './scopedEdit'

const leaf = (id: string): WorkflowStep => ({ id, name: id, do: { sleep: 1 } })
const tree = (): WorkflowStep[] => [
  { id: 'group', name: 'Group', do: { seq: [leaf('child'), leaf('sibling')] } },
  { id: 'wait', name: 'Wait', do: { wait: 'confirm', auto: [leaf('auto')] } },
  leaf('outside'),
]
const proposal = (steps: WorkflowStep[], edits: WorkflowStep[]): ScopedEditProposal => ({
  base_revision: scopedRevision(steps),
  summary: 'Edited',
  updates: edits.map(step => ({ step_id: step.id, step })),
})

describe('scoped workflow edit boundaries', () => {
  it('edits a selected container and independently selected child without changing siblings or order', () => {
    const steps = tree()
    const group = { ...scopedOwnStep(steps[0]), name: 'New group' }
    const child = { ...leaf('child'), name: 'New child' }
    const p = proposal(steps, [group, child])
    const next = applyScopedEdit(steps, ['group', 'child'], p)
    expect(next[0].name).toBe('New group')
    expect((next[0].do as { seq: WorkflowStep[] }).seq[0].name).toBe('New child')
    expect((next[0].do as { seq: WorkflowStep[] }).seq[1]).toEqual(leaf('sibling'))
    expect(next.slice(1)).toEqual(steps.slice(1))
    expect(steps).toEqual(tree())
    expect(scopedEditDiff(steps, ['group', 'child'], p).map(c => [c.stepId, c.field])).toEqual([
      ['group', 'name'],
      ['child', 'name'],
    ])
  })

  it('rejects unselected changes, child injection, action retyping, ID changes, and duplicate updates', () => {
    const steps = tree()
    const attempts = [
      proposal(steps, [{ ...leaf('outside'), name: 'Bad' }]),
      proposal(steps, [steps[0]]),
      proposal(steps, [leaf('group')]),
      { ...proposal(steps, [leaf('child')]), updates: [{ step_id: 'group', step: leaf('child') }] },
      proposal(steps, [scopedOwnStep(steps[0]), scopedOwnStep(steps[0])]),
    ]
    for (const attempt of attempts)
      expect(() => applyScopedEdit(steps, ['group'], attempt)).toThrow('outside_scope')
  })

  it('rejects stale proposals after either selected or unselected steps or input declarations change', () => {
    const steps = tree()
    const p = proposal(steps, [{ ...leaf('outside'), name: 'New' }])
    expect(() => applyScopedEdit([...steps, leaf('new')], ['outside'], p)).toThrow('stale_revision')
    expect(() => applyScopedEdit(steps, ['outside'], p, [{ name: 'changed' }])).toThrow(
      'stale_revision',
    )
  })

  it('allows a childless node to switch action type without retaining optional empty lanes', () => {
    const steps: WorkflowStep[] = [
      { id: 'selected', name: 'Wait', do: { wait: 'confirm', auto: [] } },
      leaf('outside'),
    ]
    const changed: WorkflowStep = {
      id: 'selected',
      name: 'Wait for window',
      do: { tool: 'window_wait', with: { title: 'Editor' } },
    }
    const next = applyScopedEdit(steps, ['selected'], proposal(steps, [changed]))
    expect(next).toEqual([changed, steps[1]])
    const pause: WorkflowStep = { id: 'selected', name: 'Pause', do: { sleep: 3 } }
    expect(applyScopedEdit(steps, ['selected'], proposal(steps, [pause]))[0]).toEqual(pause)
  })

  it('preserves optional lane presence and children across every container type', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'if',
        name: 'If',
        do: { if: { condition: { always: true }, then: [leaf('then')], else: [leaf('else')] } },
      },
      { id: 'loop', name: 'Loop', do: { loop: { repeat: 2, do: [leaf('body')] } } },
      ...tree(),
    ]
    const p = proposal(
      steps,
      steps.map(step => ({ ...scopedOwnStep(step), name: 'Renamed' })),
    )
    const next = applyScopedEdit(
      steps,
      steps.map(step => step.id),
      p,
    )
    next.forEach((step, i) => expect(step.do).toEqual(steps[i].do))
    const waitWithoutChildren = [{ id: 'wait', name: 'Wait', do: { wait: 'OK' } }]
    const editedWait = [{ id: 'wait', name: 'Next', do: { wait: 'OK', auto: [] } }]
    expect(
      applyScopedEdit(waitWithoutChildren, ['wait'], proposal(waitWithoutChildren, editedWait))[0]
        .do,
    ).toEqual({ wait: 'OK' })
  })

  it('rejects duplicate tree IDs and ambiguous actions', () => {
    expect(() => createScopedEditRequest([leaf('a'), leaf('a')], ['a'], 'edit')).toThrow()
    expect(() =>
      createScopedEditRequest([{ id: 'a', name: 'a', do: { sleep: 1, seq: [] } }], ['a'], 'edit'),
    ).toThrow()
  })

  it('builds declaration-only input and variable context without runtime defaults', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'producer',
        name: 'Producer',
        capture: 'result',
        do: { script: { runtime: 'python', code: 'pass' } },
      },
      leaf('selected'),
    ]
    const request = createScopedEditRequest(steps, ['selected'], 'edit', [
      { name: 'token', default: 'secret-value', sensitive: true },
    ])
    expect(request.inputs[0]).not.toHaveProperty('default')
    expect(request.variables[0].variables.map(v => v.name)).toEqual(['inputs.token', 'result'])
    expect(JSON.stringify(request.variables)).not.toContain('secret-value')
  })
})
