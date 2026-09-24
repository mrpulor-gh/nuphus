import type { WorkflowStep } from '../../core/types'
import { containerLanes, containerSummary, stepKind } from './projection'
import type { CanvasTranslate } from './presentation'

/** Only explicit action fields are used. Unknown tool arguments may contain secrets. */
export function actionSummary(step: WorkflowStep, t: CanvasTranslate): string {
  const action = step.do as Record<string, unknown>
  const kind = stepKind(step)
  if (containerLanes(step)) return containerSummary(step, kind, t) ?? ''
  switch (kind) {
    case 'sleep':
      return t('workflowEditor.summary.sleep', String(action.sleep))
    case 'call':
      return t('workflowEditor.summary.call', String(action.call ?? ''))
    case 'script':
      return t(
        'workflowEditor.summary.script',
        String((action.script as { runtime?: string })?.runtime ?? ''),
      )
    case 'wait':
    case 'chat':
    case 'assert':
    case 'break':
    case 'continue':
      return t(`workflowEditor.summary.${kind}`)
    case 'tool':
      return String(action.tool ?? '')
    case 'mcp': {
      const mcp = action.mcp as { server?: string; tool?: string }
      return `${mcp?.server ?? ''} · ${mcp?.tool ?? ''}`
    }
    default:
      return ''
  }
}
