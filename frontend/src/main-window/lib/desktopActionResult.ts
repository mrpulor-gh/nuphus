/** Native event delivery and effect verification are not business completion. */
export type DesktopActionState =
  'sent' | 'confirmed' | 'unverifiable' | 'suspected_noop' | 'partial' | 'not_sent' | 'unknown'

export interface DesktopActionResult {
  state: DesktopActionState
  dispatchState?: 'not_sent' | 'sent' | 'partial' | 'unknown'
  deliveryMode?: 'foreground' | 'background'
}

const ACTION_TOOLS = new Set([
  'desktop_semantic_action',
  'desktop_semantic_execute',
  'desktop_agent_step',
  'desktop_mouse',
  'desktop_keyboard',
  'desktop_input',
  'desktop_mouse_click',
  'desktop_mouse_hover',
  'desktop_mouse_move',
  'desktop_mouse_drag',
  'desktop_mouse_scroll',
  'desktop_keyboard_press',
  'desktop_keyboard_hotkey',
])

const NO_DISPATCH_STATUSES = new Set([
  'needs_primary_decision',
  'needs_primary_completion_check',
  'needs_input_value',
  'needs_user_input',
  'cannot_proceed',
])

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function parseOutput(output: string): Record<string, unknown> | null {
  let text = output.trim()
  // Error callbacks preserve this envelope even when the tool itself failed.
  const prefix = 'desktop_action_result:'
  if (text.startsWith('Tool error: ')) text = text.slice('Tool error: '.length)
  if (text.startsWith(prefix)) text = text.slice(prefix.length)
  try {
    return object(JSON.parse(text))
  } catch {
    // Legacy DesktopClient formatting: "result: { ... }\nsuccess: true".
    const legacy = text.match(/^(?:success: true\s+)?result:\s*(\{[\s\S]*\})(?:\s+success: true)?$/)
    if (!legacy) return null
    try {
      return object(JSON.parse(legacy[1]))
    } catch {
      return null
    }
  }
}

export function desktopActionResult(
  toolName: string | undefined,
  output: string | undefined,
): DesktopActionResult | null {
  if (!toolName?.startsWith('desktop_')) return null
  // Event previews can be truncated. Do not turn an unreadable action receipt
  // into a green "completed" marker or infer that input was sent.
  const fallback: DesktopActionResult | null = ACTION_TOOLS.has(toolName)
    ? { state: 'unknown', dispatchState: 'unknown' }
    : null
  const outer = output ? parseOutput(output) : null
  if (!outer) return fallback
  const result = object(outer.result) ?? outer
  const delivery = result.dispatch_state
  const dispatchState =
    delivery === 'not_sent' ||
    delivery === 'sent' ||
    delivery === 'partial' ||
    delivery === 'unknown'
      ? delivery
      : result.status === 'dispatched'
        ? 'sent'
        : undefined
  const effect = result.effect
  let state: DesktopActionState | undefined
  if (dispatchState === 'partial' || effect === 'partial') state = 'partial'
  else if (effect === 'confirmed') state = 'confirmed'
  else if (dispatchState === 'unknown') state = 'unknown'
  else if (effect === 'suspected_noop') state = 'suspected_noop'
  else if (effect === 'unverifiable') state = 'unverifiable'
  else if (effect === 'refused' || dispatchState === 'not_sent') state = 'not_sent'
  else if (dispatchState) state = dispatchState
  // Older semantic receipts did not expose effect separately.
  else if (result.verification === 'achieved') state = 'confirmed'
  else if (result.verification === 'no_change') state = 'suspected_noop'
  else if (result.verification === 'unknown' || result.verification === 'unexpected')
    state = 'unverifiable'
  else if (typeof result.status === 'string' && NO_DISPATCH_STATUSES.has(result.status))
    return { state: 'not_sent', dispatchState: 'not_sent' }
  const reportedMode = object(result.receipt)?.delivery_mode
  const deliveryMode =
    reportedMode === 'foreground' || reportedMode === 'background' ? reportedMode : undefined
  return state ? { state, dispatchState, ...(deliveryMode ? { deliveryMode } : {}) } : fallback
}
