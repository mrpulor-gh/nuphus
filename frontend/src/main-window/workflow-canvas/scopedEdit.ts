import { invoke, listen } from '../../core/bridge'
import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { buildVariableCatalogIndex } from './variableCatalog'

export interface ScopedUpdate {
  step_id: string
  step: WorkflowStep
}
export interface ScopedEditProposal {
  base_revision: string
  summary: string
  updates: ScopedUpdate[]
}
export interface ScopedEditRequest {
  request_id?: string
  base_revision: string
  steps: WorkflowStep[]
  selected_ids: string[]
  instruction: string
  inputs: WorkflowInputSpec[]
  variables: {
    step_id: string
    variables: { name: string; source: string; maybe_unset: boolean }[]
  }[]
}
export interface ScopedFieldChange {
  stepId: string
  field: string
  before: unknown
  after: unknown
}

const ACTIONS = [
  'tool',
  'seq',
  'loop',
  'if',
  'call',
  'wait',
  'chat',
  'script',
  'assert',
  'mcp',
  'sleep',
  'break',
  'continue',
]
const STEP_FIELDS = ['id', 'name', 'description', 'on_error', 'capture', 'timeout_secs', 'do']
type ObjectValue = Record<string, unknown>
function object(value: unknown): ObjectValue {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('invalid_proposal')
  return value as ObjectValue
}
function kind(step: WorkflowStep): string {
  const action = object(step.do)
  const keys = ACTIONS.filter(key => key in action)
  if (keys.length !== 1 || Object.keys(step).some(key => !STEP_FIELDS.includes(key)))
    throw new Error('invalid_proposal')
  const type = keys[0]
  if (
    Object.keys(action).some(
      key =>
        key !== type &&
        !(key === 'with' && ['tool', 'call', 'chat'].includes(type)) &&
        !(key === 'auto' && type === 'wait'),
    )
  )
    throw new Error('invalid_proposal')
  return type
}
function lanes(step: WorkflowStep): string[][] {
  switch (kind(step)) {
    case 'seq':
      return [['seq']]
    case 'loop':
      return [['loop', 'do']]
    case 'if':
      return [
        ['if', 'then'],
        ['if', 'else'],
      ]
    case 'wait':
      return [['auto']]
    default:
      return []
  }
}
function laneParent(step: WorkflowStep, path: string[]): ObjectValue {
  let value = object(step.do)
  for (const key of path.slice(0, -1)) value = object(value[key])
  return value
}
function children(step: WorkflowStep, path: string[]): WorkflowStep[] | undefined {
  const value = laneParent(step, path)[path[path.length - 1]]
  if (value === undefined) return undefined
  if (!Array.isArray(value)) throw new Error('invalid_proposal')
  return value as WorkflowStep[]
}
export function scopedOwnStep(step: WorkflowStep): WorkflowStep {
  const own = structuredClone(step)
  for (const path of lanes(own)) {
    if (children(own, path)) laneParent(own, path)[path[path.length - 1]] = []
  }
  return own
}
function indexSteps(steps: WorkflowStep[]): Map<string, WorkflowStep> {
  const index = new Map<string, WorkflowStep>()
  function visit(list: WorkflowStep[], depth: number) {
    if (depth > 64) throw new Error('invalid_proposal')
    for (const step of list) {
      if (!step.id || index.has(step.id)) throw new Error('invalid_proposal')
      index.set(step.id, step)
      for (const path of lanes(step)) visit(children(step, path) ?? [], depth + 1)
    }
  }
  visit(steps, 0)
  return index
}
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`
  if (value && typeof value === 'object')
    return `{${Object.entries(value)
      .filter(([, v]) => v !== undefined)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([key, v]) => `${JSON.stringify(key)}:${canonical(v)}`)
      .join(',')}}`
  return JSON.stringify(value) ?? 'undefined'
}
/** Exact editor snapshot, not a collision-prone hash. Never included in the provider prompt. */
export function scopedRevision(steps: WorkflowStep[], inputs: WorkflowInputSpec[] = []): string {
  return canonical({ steps, inputs })
}
export function createScopedEditRequest(
  steps: WorkflowStep[],
  selectedIds: readonly string[],
  instruction: string,
  inputs: WorkflowInputSpec[] = [],
): ScopedEditRequest {
  const index = indexSteps(steps)
  if (
    !selectedIds.length ||
    new Set(selectedIds).size !== selectedIds.length ||
    selectedIds.some(id => !index.has(id))
  )
    throw new Error('invalid_selection')
  const catalog = buildVariableCatalogIndex(steps, inputs)
  return {
    base_revision: scopedRevision(steps, inputs),
    steps: structuredClone(steps),
    selected_ids: [...selectedIds],
    instruction,
    // Defaults and runtime values are not needed to explain a variable's contract.
    inputs: inputs.map(({ name, type, required, sensitive }) => ({
      name,
      type,
      required,
      sensitive,
    })),
    variables: selectedIds.map(step_id => ({
      step_id,
      variables: [...(catalog.beforeStep.get(step_id)?.values() ?? [])].map(variable => ({
        name: variable.name,
        source: variable.source,
        maybe_unset: variable.maybeUnset,
      })),
    })),
  }
}
export async function requestScopedEdit(
  request: ScopedEditRequest,
  onCorrecting?: () => void,
): Promise<ScopedEditProposal> {
  const requestId = crypto.randomUUID()
  const unlisten = onCorrecting
    ? await listen<{ request_id: string; phase: string }>('workflow-edit-progress', event => {
        if (event.request_id === requestId && event.phase === 'correcting') onCorrecting()
      })
    : undefined
  try {
    const proposal = await invoke<ScopedEditProposal>('wf_propose_scoped_edit', {
      request: { ...request, request_id: requestId },
    })
    if (!proposal) throw new Error('invalid_proposal')
    return proposal
  } finally {
    unlisten?.()
  }
}

/** Recheck the current editor revision and scope at apply time, independently of the backend. */
export function applyScopedEdit(
  currentSteps: WorkflowStep[],
  selectedIds: readonly string[],
  proposal: ScopedEditProposal,
  inputs: WorkflowInputSpec[] = [],
): WorkflowStep[] {
  if (proposal.base_revision !== scopedRevision(currentSteps, inputs))
    throw new Error('stale_revision')
  const index = indexSteps(currentSteps)
  const selected = new Set(selectedIds)
  if (
    !selected.size ||
    selected.size !== selectedIds.length ||
    selectedIds.some(id => !index.has(id))
  )
    throw new Error('invalid_selection')
  if (!Array.isArray(proposal.updates)) throw new Error('invalid_proposal')
  const updates = new Map<string, WorkflowStep>()
  for (const update of proposal.updates) {
    const original = index.get(update.step_id)
    const step = update.step
    if (!original || !selected.has(update.step_id) || updates.has(update.step_id))
      throw new Error('outside_scope')
    const hasChildren = lanes(original).some(path => (children(original, path)?.length ?? 0) > 0)
    if (
      step.id !== update.step_id ||
      typeof step.name !== 'string' ||
      (hasChildren && kind(step) !== kind(original))
    )
      throw new Error('outside_scope')
    if (canonical(scopedOwnStep(step)) !== canonical(step)) throw new Error('outside_scope')
    updates.set(update.step_id, step)
  }
  function merge(list: WorkflowStep[]): WorkflowStep[] {
    return list.map(original => {
      const next = structuredClone(updates.get(original.id) ?? original)
      if (kind(next) !== kind(original)) return next
      for (const path of lanes(original)) {
        const body = children(original, path)
        const parent = laneParent(next, path)
        const key = path[path.length - 1]
        if (body) parent[key] = merge(body)
        else delete parent[key]
      }
      return next
    })
  }
  return merge(currentSteps)
}
export function scopedEditDiff(
  steps: WorkflowStep[],
  selectedIds: readonly string[],
  proposal: ScopedEditProposal,
  inputs: WorkflowInputSpec[] = [],
): ScopedFieldChange[] {
  const next = indexSteps(applyScopedEdit(steps, selectedIds, proposal, inputs))
  const before = indexSteps(steps)
  const changes: ScopedFieldChange[] = []
  function compare(stepId: string, field: string, a: unknown, b: unknown) {
    if (canonical(a) === canonical(b)) return
    if (
      a &&
      b &&
      typeof a === 'object' &&
      typeof b === 'object' &&
      !Array.isArray(a) &&
      !Array.isArray(b)
    ) {
      for (const key of new Set([...Object.keys(a), ...Object.keys(b)]))
        compare(
          stepId,
          field ? `${field}.${key}` : key,
          (a as ObjectValue)[key],
          (b as ObjectValue)[key],
        )
    } else changes.push({ stepId, field, before: a, after: b })
  }
  for (const { step_id } of proposal.updates)
    compare(step_id, '', scopedOwnStep(before.get(step_id)!), scopedOwnStep(next.get(step_id)!))
  return changes
}
