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
