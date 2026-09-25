import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { walkSteps } from './dataEdges'
import { buildVariableCatalogIndex, isVariableName } from './variableCatalog'
import { parseFieldReference, referenceRoot, templateSpans } from './fieldReferences'

export interface ReferenceSite {
  stepId: string
  path: (string | number)[]
  fieldPath: string
  value: string
  start: number
  end: number
  name: string
  input: boolean
}

const pointer = (path: (string | number)[]) =>
  '/' + path.map(key => String(key).replace(/~/g, '~0').replace(/\//g, '~1')).join('/')
const children = new Set(['/do/seq', '/do/loop/do', '/do/if/then', '/do/if/else', '/do/auto'])

/** Exact spans in workflow expressions only. Script identifiers and ordinary text are not references. */
export function referenceSites(step: WorkflowStep): ReferenceSite[] {
  const result: ReferenceSite[] = []
  const scan = (value: unknown, path: (string | number)[]) => {
    const fieldPath = pointer(path)
    if (children.has(fieldPath)) return
    if (typeof value === 'string') {
      const isVarRef =
        /^(?:\/do\/(?:if|assert)\/condition|\/do\/loop\/until)\/.*\/var$/.test(fieldPath) ||
        fieldPath === '/do/loop/for_each/items/var'
      const spans = isVarRef ? [{ body: value, bodyStart: 0 }] : templateSpans(value)
      for (const span of spans) {
        const root = referenceRoot(span.body)
        if (!root) continue
        const parsed = parseFieldReference(span.body)
        const leading = span.body.length - span.body.trimStart().length
        const start = span.bodyStart + leading
        const rootLength = parsed?.root.length ?? root.name.length
        // Input references belong to their declaration, not a same-named capture.
        result.push({
          stepId: step.id,
          path,
          fieldPath,
          value,
          start,
          end: start + rootLength,
          name: root.name,
          input: !!root.input,
        })
      }
    } else if (Array.isArray(value)) value.forEach((item, index) => scan(item, [...path, index]))
    else if (value && typeof value === 'object')
      for (const [key, child] of Object.entries(value)) scan(child, [...path, key])
  }
  scan(step.do, ['do'])
  return result
}

export interface RenameChange {
  id: string
  stepId: string
  stepName?: string
  fieldPath: string
  before: string
  after: string
  ambiguous: boolean
  sites: ReferenceSite[]
}
export interface RenamePreview {
  producerId: string
  oldName: string
  newName: string
  kind: 'capture' | 'item'
  base: string
  changes: RenameChange[]
  error?: 'invalid' | 'collision' | 'missing'
}

function target(steps: WorkflowStep[], id: string): WorkflowStep | undefined {
  let found: WorkflowStep | undefined
  walkSteps(steps, step => {
    if (step.id === id) found = step
  })
  return found
}

export function previewVariableRename(
  steps: WorkflowStep[],
  inputs: WorkflowInputSpec[],
  producerId: string,
  newName: string,
  kind: 'capture' | 'item' = 'capture',
): RenamePreview {
  const producer = target(steps, producerId)
  const oldName =
    kind === 'capture'
      ? producer?.capture
      : ((producer?.do as { loop?: { for_each?: { as?: string } } })?.loop?.for_each?.as ?? 'item')
  const preview: RenamePreview = {
    producerId,
    kind,
    oldName: oldName ?? '',
    newName,
    base: JSON.stringify(steps),
    changes: [],
  }
  if (!producer || !oldName) return { ...preview, error: 'missing' }
  if (!isVariableName(newName) || oldName === newName) return { ...preview, error: 'invalid' }
  const index = buildVariableCatalogIndex(steps, inputs)
  // A rename must not silently capture another binding.
  if (
    index.captures.some(
      candidate =>
        candidate.name === newName && candidate.producerStepIds?.some(id => id !== producerId),
    ) ||
    inputs.some(input => input.name === newName)
  )
    return { ...preview, error: 'collision' }
  walkSteps(steps, step => {
    const groups = new Map<string, ReferenceSite[]>()
    const scopeFor = (path: string) =>
      path.startsWith('/do/loop/until/')
        ? index.loopConditionScope.get(step.id)
        : index.beforeStep.get(step.id)
    for (const site of referenceSites(step)) {
      if (site.name !== oldName || site.input) continue
      const source = scopeFor(site.fieldPath)?.get(oldName)
      const producers = source?.producerStepIds ?? (source?.stepId ? [source.stepId] : [])
      if (!producers.includes(producerId)) continue
      const collisions = scopeFor(site.fieldPath)?.get(newName)
      if (collisions && collisions.stepId !== producerId) preview.error = 'collision'
      const group = groups.get(site.fieldPath) ?? []
      group.push(site)
      groups.set(site.fieldPath, group)
    }
    for (const [fieldPath, sites] of groups) {
      let after = sites[0].value
      for (const site of [...sites].sort((a, b) => b.start - a.start))
        after = after.slice(0, site.start) + newName + after.slice(site.end)
      const binding = scopeFor(fieldPath)?.get(oldName)
      preview.changes.push({
        id: `${step.id}:${fieldPath}`,
        stepId: step.id,
        stepName: step.name,
        fieldPath,
        before: sites[0].value,
        after,
        ambiguous: (binding?.producerStepIds?.length ?? 1) > 1,
        sites,
      })
    }
  })
  return preview
}

export function applyVariableRename(
  steps: WorkflowStep[],
  preview: RenamePreview,
  choices: Record<string, boolean>,
): WorkflowStep[] {
  if (preview.error || JSON.stringify(steps) !== preview.base)
    throw new Error('rename_stale_or_invalid')
  if (preview.changes.some(change => change.ambiguous && !(change.id in choices)))
    throw new Error('rename_ambiguous')
  const next = structuredClone(steps)
  const producer = target(next, preview.producerId)!
  if (preview.kind === 'capture') producer.capture = preview.newName
  else (producer.do as { loop: { for_each: { as: string } } }).loop.for_each.as = preview.newName
  for (const change of preview.changes) {
    if (change.ambiguous && !choices[change.id]) continue
    const path = change.sites[0].path
    let owner: unknown = target(next, change.stepId)
    for (const key of path.slice(0, -1)) owner = (owner as Record<string | number, unknown>)[key]
    ;(owner as Record<string | number, unknown>)[path[path.length - 1]] = change.after
  }
  return next
}
