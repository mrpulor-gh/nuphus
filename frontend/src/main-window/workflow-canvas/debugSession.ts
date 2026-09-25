import { invoke } from '../../core/bridge'
import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { profileStep, walkSteps } from './dataEdges'
import { mergeEditorProblems, type EditorProblem } from './editorProblems'
import { validateIR } from './validate'

export interface DebugRequest {
  workflow_id: string
  steps: WorkflowStep[]
  inputs: WorkflowInputSpec[]
  selected_step_id: string
  mode: 'node' | 'through'
  variables: Record<string, unknown>
  runtime_inputs: Record<string, unknown>
  use_retry_policy: boolean
  source: unknown
}
export const wfDebugRun = (request: DebugRequest) =>
  invoke<{ run_id: string }>('wf_debug_run', { request })
export const wfDebugControl = (
  workflowId: string,
  runId: string,
  action: 'pause' | 'resume' | 'cancel',
) => invoke<void>('wf_debug_control', { workflowId, runId, action })

export function parseTestValues(text: string): Record<string, unknown> {
  const value: unknown = JSON.parse(text)
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('JSON object required')
  return value as Record<string, unknown>
}

/** Structural preflight only; environment and actual data remain runtime-validated. */
export function debugPreflight(
  steps: WorkflowStep[],
  inputs: WorkflowInputSpec[] = [],
): EditorProblem | undefined {
  const issues = mergeEditorProblems(validateIR(steps, { inputs }), null)
  walkSteps(steps, step => {
    if (!step.name?.trim())
      issues.push({
        code: 'required',
        category: 'missing',
        level: 'error',
        stepId: step.id,
        fieldPath: '/name',
        details: [],
        sources: ['debug_preflight'],
      })
    if (
      'sleep' in step.do &&
      (typeof step.do.sleep !== 'number' || !Number.isFinite(step.do.sleep) || step.do.sleep <= 0)
    )
      issues.push({
        code: 'positive',
        category: 'invalid',
        level: 'error',
        stepId: step.id,
        fieldPath: '/do/sleep',
        details: [],
        sources: ['debug_preflight'],
      })
  })
  return issues.find(issue => issue.level === 'error')
}

/** Hints only: runtime resolves branch/loop values using the real execution path. */
export function debugDependencies(step: WorkflowStep): string[] {
  const produced = new Set<string>()
  const consumed = new Set<string>()
  walkSteps([step], child => {
    const profile = profileStep(child)
    for (const ref of profile.consumes)
      if (!produced.has(ref.varName))
        consumed.add(ref.input ? `inputs.${ref.varName}` : ref.varName)
    if (profile.produces) produced.add(profile.produces)
    const loop = (child.do as { loop?: { for_each?: { as?: string } } }).loop
    if (loop?.for_each) produced.add(loop.for_each.as || 'item')
  })
  return [...consumed]
}
