import { describe, expect, it } from 'vitest'
import { parseFieldReference, referenceRoot, templateSpans } from './fieldReferences'
import { profileStep } from './dataEdges'
import { projectWorkflow } from './projection'
import { validateIR } from './validate'

/**
 * 管道与方括号路径的组合是引用解析的高危区：`{{inputs["x"] | len}}` 既不是纯
 * 方括号引用（管道尾缀让 parseFieldReference 失败），也不是纯点号引用。
 * 曾经的 bug：fallback 正则只认点号，于是返回裸 `inputs` 根名 → 画布画出
 * 「未找到来源 · inputs」的虚假悬空锚点并报 V4，而 x 是已声明输入。
 */
describe('referenceRoot handles pipes with bracketed paths', () => {
  it.each([
    ['inputs.x', { name: 'x', input: true }],
    ['inputs["x"]', { name: 'x', input: true }],
    ['inputs["x"] | len', { name: 'x', input: true }],
    ['inputs.x | len', { name: 'x', input: true }],
    ['inputs.x | get "y"', { name: 'x', input: true }],
    ['inputs["x"]["y"] | get "z"', { name: 'x', input: true }],
    ['inputs.a.b', { name: 'a', input: true }],
    [' x ', { name: 'x' }],
    ['x.y', { name: 'x' }],
    ['x["y"]', { name: 'x' }],
    ['x["y"] | len', { name: 'x' }],
    ['items[0] | json "k"', { name: 'items' }],
    ['x | default "d"', { name: 'x' }],
  ])('resolves %s', (expression, expected) => {
    expect(referenceRoot(expression)).toEqual(expected)
  })

  it.each(['ENV:HOME', 'ENV:HOME | default "~/x"', 'params', 'params.topic', 'params["t"]'])(
    'excludes the external injection namespace %s',
    expression => {
      expect(referenceRoot(expression)).toBeNull()
    },
  )

  it('keeps parseFieldReference strict: a piped body is not a valid field reference', () => {
    expect(parseFieldReference('inputs["x"]')).toEqual({ root: 'inputs', segments: ['x'] })
    expect(parseFieldReference('inputs["x"] | len')).toBeNull()
  })
});

describe('template scanning records pipes for every reference shape', () => {
  it.each([
    ['{{inputs["x"] | len}}', [{ varName: 'x', pipes: ['len'], input: true }]],
    ['{{inputs.x | get "y"}}', [{ varName: 'x', pipes: ['get'], input: true }]],
    ['{{x["y"] | json "k" | len}}', [{ varName: 'x', pipes: ['json', 'len'] }]],
    ['{{x}}', [{ varName: 'x', pipes: [] }]],
    ['{{missing | default "d"}}', [{ varName: 'missing', pipes: ['default'] }]],
  ])('scans %s', (text, expected) => {
    expect(profileStep({ id: 's', name: 'S', do: { tool: 't', with: { v: text } } }).consumes).toEqual(
      expected,
    )
  })
});

describe('a declared input referenced through a bracket path with a pipe', () => {
  const inputs = [{ name: 'x', type: 'string', required: true }] as never
  const steps = [
    { id: 'a', name: 'A', do: { tool: 't', with: { v: '{{inputs["x"] | len}}' } } },
  ] as never

  it('produces no dangling anchor and no V4 warning', () => {
    const layer = projectWorkflow({ steps, inputs }).layers.get('root')!
    const external = layer.nodes.filter(n => n.synthetic === 'external')
    // Only the declared-input contract anchor exists; no "missing source" anchor.
    expect(external).toHaveLength(1)
    expect(external[0]).toMatchObject({ externalVar: 'x', externalInputDeclared: true })
    // The single edge is the resolved input edge, never a dangling one.
    expect(layer.edges).toEqual([
      expect.objectContaining({
        kind: 'external',
        label: 'inputs.x',
        dangling: false,
        source: 'root::external::inputs.x',
      }),
    ])
    expect(validateIR(steps, { inputs })).toEqual([])
  })

  it('still reports an undeclared bracket input reference', () => {
    const undeclared = [
      { id: 'a', name: 'A', do: { tool: 't', with: { v: '{{inputs["nope"] | len}}' } } },
    ] as never
    const layer = projectWorkflow({ steps: undeclared, inputs: [] }).layers.get('root')!
    const dangling = layer.edges.filter(e => e.kind === 'external' && e.dangling)
    expect(dangling).toHaveLength(1)
    expect(dangling[0].label).toBe('inputs.nope')
    expect(validateIR(undeclared, { inputs: [] })).toMatchObject([
      { rule: 'input_reference', subject: 'nope' },
    ])
  })
});

describe('templateSpans keeps quoted brackets intact', () => {
  it('does not terminate a span on a quoted closing brace', () => {
    expect(templateSpans('{{a["}}"]}}').map(s => s.body)).toEqual(['a["}}"]'])
  })
})
