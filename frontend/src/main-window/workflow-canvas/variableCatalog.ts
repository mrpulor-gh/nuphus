import type { WorkflowInputSpec, WorkflowStep, LoopDef, IfDef } from '../../core/types'
import { walkSteps } from './dataEdges'

export interface VariableSource {
  kind: 'capture' | 'input' | 'item' | 'index'
  name: string
  stepId?: string
}

export interface VariableCandidate {
  /** Bare reference; declared inputs deliberately keep the inputs. namespace. */
  name: string
  source: 'capture' | 'input' | 'loop'
  sourceLabel: string
  /** Language-neutral provenance; formatted only when displayed. No runtime values. */
  sources?: VariableSource[]
  stepId?: string
  /** Every possible producer after control-flow joins (not merely the last DFS producer). */
  producerStepIds?: string[]
  maybeUnset: boolean
}

export interface VariableCatalog {
  references: VariableCandidate[]
  captures: VariableCandidate[]
}

const CAPTURE_ACTIONS = ['tool', 'script', 'chat', 'model', 'mcp']
export const isVariableName = (name: string): boolean => /^[A-Za-z_][A-Za-z0-9_.]*$/.test(name)

function captureOf(step: WorkflowStep): VariableCandidate | undefined {
  if (
    !step.capture ||
    !step.do ||
    typeof step.do !== 'object' ||
    !CAPTURE_ACTIONS.some(key => key in step.do)
  )
    return undefined
  return {
    name: step.capture,
    source: 'capture',
    sourceLabel: step.name || step.id,
    sources: [{ kind: 'capture', name: step.name || step.id, stepId: step.id }],
    stepId: step.id,
    producerStepIds: [step.id],
    maybeUnset: false,
  }
}

type Scope = Map<string, VariableCandidate>

function mergeSources(candidates: (VariableCandidate | undefined)[]): VariableSource[] {
  const sources = candidates.flatMap(candidate => candidate?.sources ?? [])
  return [...new Map(sources.map(source => [JSON.stringify(source), source])).values()]
}

/** Join execution paths, not DFS order: a producer in one branch cannot leak into its sibling. */
function joinScopes(paths: Scope[]): Scope {
  const result: Scope = new Map()
  for (const path of paths) {
    for (const [name, candidate] of path) {
      const producers = paths.map(p => p.get(name))
      result.set(name, {
        ...candidate,
        maybeUnset: producers.some(p => !p || p.maybeUnset),
        sourceLabel: [...new Set(producers.flatMap(p => (p ? [p.sourceLabel] : [])))].join(' / '),
        sources: mergeSources(producers),
        producerStepIds: [
          ...new Set(producers.flatMap(p => p?.producerStepIds ?? (p?.stepId ? [p.stepId] : []))),
        ],
      })
    }
  }
  return result
}

/** Editor-only provenance. No runtime values, registry, or IR mutation. */
export function buildVariableCatalogIndex(
  steps: WorkflowStep[],
  inputs: WorkflowInputSpec[] = [],
): {
  beforeStep: Map<string, ReadonlyMap<string, VariableCandidate>>
  loopConditionScope: Map<string, ReadonlyMap<string, VariableCandidate>>
  captures: VariableCandidate[]
  inputs: VariableCandidate[]
  /** stepId → the same-path producer it shadows (mutually exclusive branches excluded) */
  shadowed: Map<string, string>
} {
  const captures = new Map<string, VariableCandidate>()
  walkSteps(steps, step => {
    const candidate = captureOf(step)
    if (candidate) {
      const prior = captures.get(candidate.name)
      captures.set(candidate.name, {
        ...candidate,
        sourceLabel: prior
          ? `${prior.sourceLabel} / ${candidate.sourceLabel}`
          : candidate.sourceLabel,
        sources: mergeSources([prior, candidate]),
        producerStepIds: [...new Set([...(prior?.producerStepIds ?? []), step.id])],
      })
    }
  })
  const initial: Scope = new Map(
    inputs.map(input => [
      `inputs.${input.name}`,
      {
        name: `inputs.${input.name}`,
        source: 'input',
        sourceLabel: `工作流输入 · ${input.name}`,
        sources: [{ kind: 'input', name: input.name }],
        maybeUnset: !input.required && input.default === undefined,
      },
    ]),
  )
  const beforeStep = new Map<string, ReadonlyMap<string, VariableCandidate>>()
  const loopConditionScope = new Map<string, ReadonlyMap<string, VariableCandidate>>()
  /**
   * stepId → 被本步遮蔽的生产者 stepId（同一执行路径上的前一个同名 capture）。
   *
   * 判定依据是控制流 join 后的 scope，而非全树 DFS 序：`if` 的 then/else 是互斥
   * 分支，各自从同一 scope 出发，同名 capture 只会在其中一条路径上生效——那不是
   * 遮蔽（旧实现把它算成遮蔽，给合法的分支局部变量报 V10 警告）。
   */
  const shadowed = new Map<string, string>()
  function visit(list: WorkflowStep[], incoming: Scope): Scope {
    let scope = new Map(incoming)
    let conditionalTail = false
    for (const step of list) {
      beforeStep.set(step.id, new Map(scope))
      const action = (step.do && typeof step.do === 'object' ? step.do : {}) as Record<
        string,
        unknown
      >
      const before = new Map(scope)
      // 遮蔽判定只看进入本步时的 scope：同路径前序 capture 才会被这里覆盖。
      // if 的两个分支在 joinScopes 之后才回到同一 scope，互斥分支不会互相遮蔽。
      if (step.capture) {
        const prior = before.get(step.capture)
        if (prior?.stepId && prior.stepId !== step.id) shadowed.set(step.id, prior.stepId)
      }
      if (Array.isArray(action.seq)) scope = visit(action.seq as WorkflowStep[], scope)
      const branch = action.if as IfDef | undefined
      if (branch) scope = joinScopes([visit(branch.then, scope), visit(branch.else ?? [], scope)])
      const loop = action.loop as LoopDef | undefined
      if (loop) {
        const body = new Map(scope)
        if (loop.for_each) {
          const name = loop.for_each.as || 'item'
          body.set(name, {
            name,
            source: 'loop',
            sourceLabel: `${step.name} · 当前项`,
            sources: [{ kind: 'item', name: step.name || step.id, stepId: step.id }],
            stepId: step.id,
            maybeUnset: false,
          })
        }
        if (loop.for_each || loop.repeat !== undefined) {
          body.set('_index', {
            name: '_index',
            source: 'loop',
            sourceLabel: `${step.name} · 从 0 开始的序号`,
            sources: [{ kind: 'index', name: step.name || step.id, stepId: step.id }],
            stepId: step.id,
            maybeUnset: false,
          })
        }
        const after = visit(loop.do, body)
        loopConditionScope.set(step.id, new Map(after))
        // Loop locals are recommended only inside their lexical body, even if runtime leaks them.
        for (const [name, value] of after) {
          if (value.source === 'loop' && value.stepId === step.id) {
            if (before.has(name)) after.set(name, before.get(name)!)
            else after.delete(name)
          }
        }
        scope = joinScopes([before, after])
      }
      if (Array.isArray(action.auto))
        scope = joinScopes([scope, visit(action.auto as WorkflowStep[], scope)])
      const capture = captureOf(step)
      if (capture) scope.set(capture.name, capture)
      if (step.on_error === 'skip' || conditionalTail) scope = joinScopes([before, scope])
      if ('break' in action || 'continue' in action) conditionalTail = true
    }
    return scope
  }
  visit(steps, initial)
  return {
    beforeStep,
    loopConditionScope,
    captures: [...captures.values()],
    inputs: [...initial.values()],
    shadowed,
  }
}

export function buildVariableCatalog(
  steps: WorkflowStep[],
  inputs: WorkflowInputSpec[] = [],
  currentStepId?: string,
): VariableCatalog {
  const index = buildVariableCatalogIndex(steps, inputs)
  const before = currentStepId ? index.beforeStep.get(currentStepId) : undefined
  return { references: before ? [...before.values()] : index.inputs, captures: index.captures }
}
