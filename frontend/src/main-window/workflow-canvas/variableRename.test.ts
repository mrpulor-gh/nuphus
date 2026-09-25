import { describe, expect, it } from 'vitest'
import type { WorkflowStep } from '../../core/types'
import { applyVariableRename, previewVariableRename } from './variableRename'
import { fieldReference, parseFieldReference, templateSpans } from './fieldReferences'

describe('field references and safe rename', () => {
  it('renames loop items through joins and body captures in post-body conditions', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'loop',
        name: 'Loop',
        do: {
          loop: {
            for_each: { items: { var: 'items' }, as: 'row' },
            do: [
              {
                id: 'branch',
                name: 'Branch',
                do: { if: { condition: { always: true }, then: [], else: [] } },
              },
              { id: 'use', name: 'Use', do: { tool: 'echo', with: { value: '{{row["title"]}}' } } },
            ],
          },
        },
      },
    ]
    const preview = previewVariableRename(steps, [], 'loop', 'record', 'item')
    expect(preview.changes).toHaveLength(1)
    expect(preview.changes[0].after).toBe('{{record["title"]}}')
    const until: WorkflowStep[] = [
      {
        id: 'until',
        name: 'Until',
        do: {
          loop: {
            until: { equals: [{ var: 'result["done"]' }, 'true'] },
            do: [
              {
                id: 'producer',
                name: 'Produce',
                capture: 'result',
                do: { tool: 'read', with: {} },
              },
            ],
          },
        },
      },
    ]
    const changed = previewVariableRename(until, [], 'producer', 'check')
    expect(changed.changes).toHaveLength(1)
    expect(changed.changes[0]).toMatchObject({
      fieldPath: '/do/loop/until/equals/0/var',
      after: 'check["done"]',
    })
  })
  it('roundtrips keys, indices and template delimiters inside quoted keys', () => {
    const ref = fieldReference('result', ['a.b', 0, 'x}}y', 'quote"'])
    expect(parseFieldReference(ref)).toEqual({
      root: 'result',
      segments: ['a.b', 0, 'x}}y', 'quote"'],
    })
    expect(templateSpans(`before {{${ref}}} after`)).toHaveLength(1)
    expect(parseFieldReference('x[evil()]')).toBeNull()
    expect(parseFieldReference('x[-1]')).toBeNull()
  })
  it('updates template spans, not arbitrary script text, and rejects a stale preview', () => {
    const steps: WorkflowStep[] = [
      { id: 'source', name: 'Source', capture: 'wn', do: { tool: 'read', with: {} } },
      {
        id: 'use',
        name: 'Use',
        do: {
          script: {
            runtime: 'python',
            code: 'wn = "wn"\nprint({{wn["window_id"]}}) # {{wn | get "title"}}',
          },
        },
      },
    ]
    const preview = previewVariableRename(steps, [], 'source', 'window')
    expect(preview.error).toBeUndefined()
    const next = applyVariableRename(steps, preview, {})
    expect((next[1].do as { script: { code: string } }).script.code).toBe(
      'wn = "wn"\nprint({{window["window_id"]}}) # {{window | get "title"}}',
    )
    expect(steps[0].capture).toBe('wn')
    expect(() => applyVariableRename(next, preview, {})).toThrow('rename_stale')
  })
  it('requires an explicit choice for references with multiple branch producers', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'branch',
        name: 'Branch',
        do: {
          if: {
            condition: { always: true },
            then: [{ id: 'a', name: 'A', capture: 'x', do: { tool: 'read', with: {} } }],
            else: [{ id: 'b', name: 'B', capture: 'x', do: { tool: 'read', with: {} } }],
          },
        },
      },
      { id: 'use', name: 'Use', do: { tool: 'echo', with: { value: '{{x}}' } } },
    ]
    const preview = previewVariableRename(steps, [], 'a', 'new_x')
    expect(preview.changes[0].ambiguous).toBe(true)
    expect(() => applyVariableRename(steps, preview, {})).toThrow('rename_ambiguous')
    expect(applyVariableRename(steps, preview, { [preview.changes[0].id]: false })[1]).toEqual(
      steps[1],
    )
  })
})
