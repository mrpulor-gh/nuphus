import { describe, expect, it } from 'vitest'
import zh from '../../locales/zh'
import en from '../../locales/en'
import {
  ADDABLE_KINDS,
  NODE_KIND_KEYS,
  defaultCanvasTranslate,
  nodeKindLabel,
  nodeKindDescription,
  variableSourceLabel,
  type CanvasTranslate,
} from './presentation'
import { buildVariableCatalog } from './variableCatalog'
import { projectWorkflow } from './projection'
import type { WorkflowStep } from '../../core/types'

const english: CanvasTranslate = (key, ...args) =>
  (en[key] ?? key).replace(/\{(\d+)\}/g, (match, index: string) => args[Number(index)] ?? match)

describe('canvas presentation translations', () => {
  it('covers every node kind in both languages and distinguishes delay from confirmation', () => {
    expect(Object.keys(NODE_KIND_KEYS)).toHaveLength(14)
    expect(ADDABLE_KINDS).toHaveLength(13)
    expect(ADDABLE_KINDS).not.toContain('custom')
    for (const [kind, key] of Object.entries(NODE_KIND_KEYS)) {
      expect(zh[key]).toBeTruthy()
      expect(en[key]).toBeTruthy()
      expect(nodeKindLabel(kind, english)).toBe(en[key])
      expect(nodeKindDescription(kind, english)).toBe(en[`${key}.description`])
      expect(zh[`${key}.description`]).toBeTruthy()
    }
    expect(nodeKindLabel('sleep', english)).toBe('Delay')
    expect(nodeKindLabel('wait', english)).toBe('Confirm')
    expect(nodeKindLabel('future_kind', english)).toBe('Legacy node')
  })

  it('preserves placeholders and has no untranslated system copy in the English keys', () => {
    for (const key of Object.keys(zh).filter(key => key.startsWith('workflowCanvas.'))) {
      expect(en[key], key).toBeTruthy()
      expect(en[key], key).not.toMatch(/[\u3400-\u9fff]/u)
      expect(en[key].match(/\{\d+\}/g)?.sort() ?? [], key).toEqual(
        zh[key].match(/\{\d+\}/g)?.sort() ?? [],
      )
    }
  })

  it('formats structured input and loop sources without translating user names or exposing values', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'loop',
        name: '遍历 $&',
        do: {
          loop: {
            for_each: { items: { var: 'inputs.items' }, as: 'row' },
            do: [{ id: 'read', name: '读取', do: { sleep: 1 } }],
          },
        },
      },
    ]
    const catalog = buildVariableCatalog(steps, [{ name: 'items', default: 'private' }], 'read')
    expect(catalog.references.map(v => variableSourceLabel(v, english))).toEqual([
      'Workflow input · items',
      '遍历 $& · Current item',
      '遍历 $& · Zero-based index',
    ])
    expect(catalog.references.map(v => variableSourceLabel(v, defaultCanvasTranslate))).toEqual([
      '工作流输入 · items',
      '遍历 $& · 当前项',
      '遍历 $& · 从 0 开始的序号',
    ])
    expect(JSON.stringify(catalog)).not.toContain('private')
    expect(JSON.stringify(catalog)).not.toContain('dataType')
  })

  it('retains distinct producers after a branch overwrites a loop-local variable', () => {
    const reader: WorkflowStep = { id: 'read', name: 'read', do: { sleep: 1 } }
    const catalog = buildVariableCatalog(
      [
        {
          id: 'loop',
          name: '遍历',
          do: {
            loop: {
              for_each: { items: { var: 'items' }, as: 'row' },
              do: [
                {
                  id: 'branch',
                  name: '分支',
                  do: {
                    if: {
                      condition: { always: true },
                      then: [
                        { id: 'write', name: '修改当前项', capture: 'row', do: { tool: 'test' } },
                      ],
                    },
                  },
                },
                reader,
              ],
            },
          },
        },
      ],
      [],
      'read',
    )
    const row = catalog.references.find(v => v.name === 'row')!
    expect(variableSourceLabel(row, english)).toBe('修改当前项 / 遍历 · Current item')
    expect(row.maybeUnset).toBe(false)
  })

  it('keeps every capture producer and only translates system projection text', () => {
    const steps: WorkflowStep[] = [
      {
        id: 'one',
        name: '[唤起微信] 后台/托盘态唤起主窗口',
        capture: 'wn',
        on_error: 'skip',
        do: { tool: 'desktop_launch' },
      },
      {
        id: 'two',
        name: '另一步',
        capture: 'wn',
        do: { script: { runtime: 'python', code: 'print(1)' } },
      },
      { id: 'repeat', name: '重复', do: { loop: { repeat: 3, do: [] } } },
    ]
    const saved = JSON.stringify(steps)
    const captures = buildVariableCatalog(steps).captures
    expect(variableSourceLabel(captures[0], english)).toBe(`${steps[0].name} / 另一步`)
    expect(captures[0].producerStepIds).toEqual(['one', 'two'])
    const projection = projectWorkflow({ steps, inputs: [{ name: 'topic' }] }, english)
    expect(projection.index.nodeById.get('one')).toMatchObject({
      name: steps[0].name,
      capture: 'wn',
      onErrorLabel: 'Skip on error',
      kind: 'tool',
    })
    expect(projection.index.nodeById.get('repeat')?.containerSummary).toBe(
      'Repeat 3 times · max 100',
    )
    expect(
      projection.layers.get('root')?.nodes.some(n => n.name === 'Workflow input · topic'),
    ).toBe(true)
    expect(JSON.stringify(steps)).toBe(saved)
  })
})
