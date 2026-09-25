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
    case 'tool': {
      const params =
        action.with && typeof action.with === 'object'
          ? (action.with as Record<string, unknown>)
          : {}
      const scalar = (key: string) =>
        typeof params[key] === 'string' || typeof params[key] === 'number'
          ? String(params[key])
          : null
      // Known, non-content parameters only. Never summarize message bodies,
      // clipboard contents, credentials or arbitrary argument objects.
      if (action.tool === 'system_sleep' && scalar('seconds') !== null)
        return t('workflowEditor.summary.sleep', scalar('seconds')!)
      if (action.tool === 'desktop_targets_list' && scalar('query'))
        return t('workflowEditor.summary.findWindow', scalar('query')!)
      if (action.tool === 'desktop_window_activate' && scalar('hwnd'))
        return t('workflowEditor.summary.activateWindow', scalar('hwnd')!)
      if (action.tool === 'desktop_window_screenshot' && scalar('title'))
        return t('workflowEditor.summary.captureWindow', scalar('title')!)
      return String(action.tool ?? '')
    }
    case 'mcp': {
      const mcp = action.mcp as { server?: string; tool?: string }
      return `${mcp?.server ?? ''} · ${mcp?.tool ?? ''}`
    }
    default:
      return ''
  }
}
