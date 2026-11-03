import { describe, expect, it } from 'vitest'
import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { validateIR } from './validate'
import { debugPreflight } from './debugSession'

const inputs: WorkflowInputSpec[] = ['contact', 'message'].map(name => ({
  name,
  type: 'string',
  required: true,
}))
const tool = (id: string, text: string): WorkflowStep => ({
  id,
  name: id,
  do: { tool: 'test', with: { text } },
})
describe('input-aware canvas validation', () => {
  it('accepts the sanitized declared-input and empty-THEN guard workflow without executing it', () => {
    const steps: WorkflowStep[] = [
      { ...tool('wake', ''), capture: 'wn' },
      { ...tool('open_chat', '{{inputs.contact}}'), capture: 'open' },
      {
        id: 'guard_open',
        name: 'Guard',
        do: {
          if: {
            condition: { not_empty: { var: 'open' } },
            then: [],
            else: [tool('fail_open', '{{inputs.contact}} {{open}}')],
          },
        },
      },
      tool('type_message', '{{wn | json hwnd}} {{inputs.message}}'),
      { id: 'wait_input', name: 'Wait', do: { sleep: 1 } },
      { ...tool('send_verify', '{{inputs.message}}'), capture: 'send_state' },
      {
        id: 'guard_send',
        name: 'Guard',
        do: {
          if: {
            condition: { not_empty: { var: 'send_state' } },
            then: [],
            else: [tool('fail_send', '{{send_state}}')],
          },
        },
      },
    ]
    expect(validateIR(steps, { inputs })).toEqual([])
    expect(debugPreflight(steps, inputs)).toBeUndefined()
  })
  it('recognizes aliases, bracket fields, pipes, conditions and collections', () => {
    const steps: WorkflowStep[] = [
      tool('a', '{{contact}} {{inputs["message"]}} {{inputs.message["text"] | json value}}'),
      {
        id: 'b',
        name: 'b',
        do: { if: { condition: { not_empty: { var: 'inputs["contact"]' } }, then: [] } },
      },
      {
        id: 'c',
        name: 'c',
        do: {
          loop: {
            for_each: { items: { var: 'inputs.message' }, as: 'item' },
            do: [tool('d', '{{item}}')],
          },
        },
      },
    ]
    expect(validateIR(steps, { inputs })).toEqual([])
    const errors = validateIR(steps, { inputs: [] }).filter(issue => issue.level === 'error')
    expect(errors.map(issue => issue.fieldPath)).toContain('/do/loop/for_each/items/var')
    expect(errors.map(issue => issue.fieldPath)).toContain('/do/if/condition/not_empty/var')
    expect(debugPreflight(steps, [])?.code).toBe('input_reference')
  })
  it('never substitutes a capture for an explicit input declaration', () => {
    const steps = [
      { ...tool('capture', ''), capture: 'contact' },
      tool('ref', '{{inputs.contact}}'),
    ]
    expect(validateIR(steps)).toContainEqual(
      expect.objectContaining({
        rule: 'input_reference',
        subject: 'contact',
        stepId: 'ref',
        fieldPath: '/do/with/text',
      }),
    )
    expect(validateIR(steps, { inputs })).toEqual([])
  })
  it('keeps missing variables and permits empty branches and loops', () => {
    expect(validateIR([tool('x', '{{missing}}')])).toContainEqual(
      expect.objectContaining({ rule: 'V4', subject: 'missing' }),
    )
    const steps: WorkflowStep[] = [
      { id: 'if', name: 'if', do: { if: { condition: { always: true }, then: [] } } },
      { id: 'loop', name: 'loop', do: { loop: { repeat: 1, do: [] } } },
      { id: 'wait', name: 'wait', do: { wait: 'Confirm', auto: [] } },
      { id: 'seq', name: 'seq', do: { seq: [] } },
      { id: 'empty_wait', name: 'empty_wait', do: { wait: '', auto: [] } },
    ]
    expect(validateIR(steps).map(issue => [issue.stepId, issue.level])).toEqual([
      ['seq', 'warning'],
      ['empty_wait', 'warning'],
    ])
  })
})

/**
 * V10 遮蔽必须按**执行路径**判定，不能按全树 DFS 序。
 * `if` 的 then/else 互斥执行，同名 capture 各只在一条路径生效——旧实现把兄弟分支
 * 判为互相遮蔽，给合法的分支局部变量报了 warning。
 */
describe('V10 shadowing follows execution paths', () => {
  const capture = (id: string, name: string): WorkflowStep => ({
    id,
    name,
    do: { tool: 'test', with: {} },
    capture: name,
  })

  it('reports genuine same-path shadowing', () => {
    const steps = [capture('p1', 'x'), capture('p2', 'x')]
    expect(validateIR(steps)).toContainEqual(
      expect.objectContaining({ rule: 'V10', stepId: 'p2' }),
    )
  })

  it('reports shadowing across a seq container boundary', () => {
    const steps = [capture('outer', 'x'), { id: 'g', name: 'g', do: { seq: [capture('inner', 'x')] } }]
    expect(validateIR(steps)).toContainEqual(
      expect.objectContaining({ rule: 'V10', stepId: 'inner' }),
    )
  })

  it('does not report mutually exclusive if branches', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'if',
        name: 'if',
        do: {
          if: {
            condition: { always: true },
            then: [capture('then_producer', 'x')],
            else: [capture('else_producer', 'x')],
          },
        },
      },
    ]
    expect(validateIR(steps)).toEqual([])
  })

  it('does not report nested containers in separate branches', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'if',
        name: 'if',
        do: {
          if: {
            condition: { always: true },
            then: [{ id: 'tg', name: 'tg', do: { seq: [capture('tp', 'x')] } }],
            else: [{ id: 'eg', name: 'eg', do: { seq: [capture('ep', 'x')] } }],
          },
        },
      },
    ]
    expect(validateIR(steps)).toEqual([])
  })

  it('still reports shadowing inside a single branch', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'if',
        name: 'if',
        do: {
          if: {
            condition: { always: true },
            then: [capture('first', 'x'), capture('second', 'x')],
          },
        },
      },
    ]
    expect(validateIR(steps)).toContainEqual(
      expect.objectContaining({ rule: 'V10', stepId: 'second' }),
    )
  })

  it('does not report a loop body producer against its own iteration', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'loop',
        name: 'loop',
        do: { loop: { repeat: 2, do: [capture('body', 'acc')] } },
      },
    ]
    // Re-capturing the same name every iteration is the loop's job, not shadowing.
    expect(validateIR(steps)).toEqual([])
  })

  it('reports a post-loop capture that shadows a pre-loop one', () => {
    const steps: WorkflowStep[] = [
      capture('before_loop', 'x'),
      { id: 'loop', name: 'loop', do: { loop: { repeat: 2, do: [capture('body', 'other')] } } },
      capture('after_loop', 'x'),
    ]
    expect(validateIR(steps)).toContainEqual(
      expect.objectContaining({ rule: 'V10', stepId: 'after_loop' }),
    )
  })
})
