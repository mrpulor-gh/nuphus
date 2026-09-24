import zh from '../../locales/zh'
import type { StepKind } from './types'
import type { VariableCandidate } from './variableCatalog'

export type CanvasTranslate = (key: string, ...args: string[]) => string

/** Default for pure projection callers; interactive views pass the active translator. */
export const defaultCanvasTranslate: CanvasTranslate = (key, ...args) =>
  (zh[key] ?? key).replace(/\{(\d+)\}/g, (match, index: string) => args[Number(index)] ?? match)

export const NODE_KIND_KEYS = {
  tool: 'workflowCanvas.kind.tool',
  seq: 'workflowCanvas.kind.seq',
  loop: 'workflowCanvas.kind.loop',
  if: 'workflowCanvas.kind.if',
  call: 'workflowCanvas.kind.call',
  wait: 'workflowCanvas.kind.wait',
  chat: 'workflowCanvas.kind.chat',
  script: 'workflowCanvas.kind.script',
  assert: 'workflowCanvas.kind.assert',
  mcp: 'workflowCanvas.kind.mcp',
  sleep: 'workflowCanvas.kind.sleep',
  break: 'workflowCanvas.kind.break',
  continue: 'workflowCanvas.kind.continue',
  custom: 'workflowCanvas.kind.custom',
} satisfies Record<StepKind, string>

export const ADDABLE_KINDS = (Object.keys(NODE_KIND_KEYS) as StepKind[]).filter(
  kind => kind !== 'custom',
)

export function nodeKindLabel(kind: string, t: CanvasTranslate): string {
  return t(NODE_KIND_KEYS[kind as StepKind] ?? NODE_KIND_KEYS.custom)
}

export function nodeKindDescription(kind: string, t: CanvasTranslate): string {
  return t(`${NODE_KIND_KEYS[kind as StepKind] ?? NODE_KIND_KEYS.custom}.description`)
}

/** Translate only system descriptions, never user-authored node names or variable names. */
export function variableSourceLabel(candidate: VariableCandidate, t: CanvasTranslate): string {
  if (!candidate.sources?.length) return candidate.sourceLabel
  return [
    ...new Set(
      candidate.sources.map(source => {
        if (source.kind === 'capture') return source.name
        return t(`workflowCanvas.variable.source.${source.kind}`, source.name)
      }),
    ),
  ].join(' / ')
}
