import type { ValidationDiagnostic } from '../lib/api'
import type { Problem } from './validate'

export type ProblemCategory = 'missing' | 'variable' | 'invalid' | 'runtime' | 'structure'
export interface EditorProblem {
  code: string
  category: ProblemCategory
  level: 'warning' | 'error'
  stepId?: string
  fieldPath?: string
  details: string[]
  sources: string[]
  subject?: string
}

const rules: Record<string, [string, ProblemCategory, string?]> = {
  V1: ['step_id', 'structure', '/id'],
  V2: ['children', 'structure', '/do'],
  V3: ['loop_control', 'structure', '/do'],
  V4: ['variable', 'variable', '/do'],
  V5: ['variable', 'variable', '/do/loop/for_each/items'],
  V6: ['condition', 'invalid', '/do'],
  V7: ['condition', 'invalid', '/do'],
  V8: ['loop', 'invalid', '/do/loop/max'],
  V9: ['capture', 'invalid', '/capture'],
  V10: ['shadow', 'variable', '/capture'],
  V12: ['required', 'missing', '/do/call'],
  V13: ['legacy', 'structure', '/do'],
  V14: ['history', 'structure'],
  input_reference: ['input_reference', 'variable'],
  empty_wait: ['wait', 'structure', '/do/wait'],
}

export function problemIdentity(issue: EditorProblem): string {
  return JSON.stringify([
    issue.code,
    issue.stepId,
    issue.fieldPath,
    issue.subject,
    issue.code === 'validation' || issue.code === 'execution' ? issue.details[0] : null,
  ])
}

export function mergeEditorProblems(
  local: Problem[],
  report: { errors: string[]; warnings: string[]; diagnostics?: ValidationDiagnostic[] } | null,
): EditorProblem[] {
  const result: EditorProblem[] = []
  const add = (issue: EditorProblem) => {
    const same = result.find(
      other =>
        other.stepId === issue.stepId &&
        other.fieldPath === issue.fieldPath &&
        other.code === issue.code &&
        other.subject === issue.subject &&
        (issue.code !== 'validation' || other.details[0] === issue.details[0]),
    )
    if (same) {
      same.sources = [...new Set([...same.sources, ...issue.sources])]
      same.details = [...new Set([...same.details, ...issue.details])]
      if (issue.level === 'error') same.level = 'error'
    } else result.push(issue)
  }
  for (const problem of local) {
    const [code, category, fieldPath] = rules[problem.rule] ?? ['validation', 'structure']
    add({
      code,
      category,
      fieldPath: problem.fieldPath ?? fieldPath,
      stepId: problem.stepId,
      level: problem.level,
      details: [problem.message],
      sources: [problem.rule],
      subject: problem.subject,
    })
  }
  for (const diagnostic of report?.diagnostics ?? []) {
    // The compiler also reports undeclared inputs at workflow scope; keep the
    // more useful local occurrence locations without counting the aggregate twice.
    const occurrences =
      ['input_reference', 'variable'].includes(diagnostic.code) && diagnostic.subject
        ? result.filter(
            issue =>
              issue.code === diagnostic.code &&
              issue.subject === diagnostic.subject &&
              (diagnostic.code === 'input_reference' || issue.stepId === diagnostic.step_id),
          )
        : []
    if (occurrences.length) {
      for (const issue of occurrences) {
        issue.sources = [...new Set([...issue.sources, 'compiler'])]
        issue.details = [...new Set([...issue.details, diagnostic.detail])]
      }
      continue
    }
    add({
      code: diagnostic.code,
      category: ['missing', 'variable', 'invalid', 'runtime', 'structure'].includes(
        diagnostic.category,
      )
        ? (diagnostic.category as ProblemCategory)
        : 'structure',
      level: diagnostic.severity,
      stepId: diagnostic.step_id ?? undefined,
      fieldPath: diagnostic.field_path ?? undefined,
      details: [diagnostic.detail],
      sources: ['compiler'],
      subject: diagnostic.subject ?? undefined,
    })
  }
  for (const [level, messages] of [
    ['error', report?.errors],
    ['warning', report?.warnings],
  ] as const) {
    for (const message of messages ?? []) {
      if (report?.diagnostics?.some(issue => issue.detail === message && issue.severity === level))
        continue
      add({
        code: 'validation',
        category: 'structure',
        level,
        details: [message],
        sources: ['compiler'],
      })
    }
  }
  return result
}
