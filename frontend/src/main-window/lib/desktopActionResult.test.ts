import { describe, expect, it } from 'vitest'
import { desktopActionResult } from './desktopActionResult'

describe('desktopActionResult', () => {
  it('shows reported delivery, never interprets requested auto as background', () => {
    const parse = (mode: string) =>
      desktopActionResult(
        'desktop_semantic_action',
        JSON.stringify({
          dispatch_state: 'sent',
          effect: 'confirmed',
          delivery_mode: 'auto',
          receipt: { delivery_mode: mode },
        }),
      )
    expect(parse('foreground')?.deliveryMode).toBe('foreground')
    expect(parse('background')?.deliveryMode).toBe('background')
    expect(parse('auto')?.deliveryMode).toBeUndefined()
  })
  it.each([
    ['sent', undefined, 'sent'],
    ['sent', 'confirmed', 'confirmed'],
    ['sent', 'unverifiable', 'unverifiable'],
    ['sent', 'suspected_noop', 'suspected_noop'],
    ['partial', 'unverifiable', 'partial'],
    ['not_sent', 'refused', 'not_sent'],
    ['unknown', 'unverifiable', 'unknown'],
    // An idempotent action may already be satisfied without sending input.
    ['not_sent', 'confirmed', 'confirmed'],
  ])('keeps delivery %s separate from effect %s', (dispatchState, effect, state) => {
    expect(
      desktopActionResult(
        'desktop_semantic_execute',
        JSON.stringify({ dispatch_state: dispatchState, effect, business_goal_confirmed: false }),
      ),
    ).toEqual({ state, dispatchState })
  })

  it('decodes native client wrappers and legacy text receipts', () => {
    const result = { status: 'dispatched', verified: false }
    for (const output of [
      JSON.stringify({ success: true, result }),
      `result: ${JSON.stringify(result)}\nsuccess: true`,
      `success: true\nresult: ${JSON.stringify(result)}`,
    ]) {
      expect(desktopActionResult('desktop_mouse', output)).toEqual({
        state: 'sent',
        dispatchState: 'sent',
      })
    }
  })

  it('retains partial dispatch information even when the tool failed', () => {
    const encoded = `desktop_action_result:${JSON.stringify({
      dispatch_state: 'partial',
      effect: 'partial',
      message: 'text sent, submit failed',
    })}`
    for (const output of [encoded, `Tool error: ${encoded}`]) {
      expect(desktopActionResult('desktop_input', output)).toEqual({
        state: 'partial',
        dispatchState: 'partial',
      })
    }
  })

  it.each([
    ['achieved', 'confirmed'],
    ['no_change', 'suspected_noop'],
    ['unknown', 'unverifiable'],
    ['unexpected', 'unverifiable'],
  ])('supports older verification receipts: %s', (verification, state) => {
    expect(
      desktopActionResult('desktop_semantic_execute', JSON.stringify({ verification })),
    ).toEqual({
      state,
      dispatchState: undefined,
    })
  })

  it('does not claim an action was sent when enhanced selection hands off', () => {
    expect(
      desktopActionResult('desktop_agent_step', '{"status":"needs_primary_decision"}'),
    ).toEqual({ state: 'not_sent', dispatchState: 'not_sent' })
  })

  it('marks missing or truncated action receipts unknown instead of completed', () => {
    for (const output of [undefined, '', 'not JSON', '{"dispatch_state":"sent","receipt":']) {
      expect(desktopActionResult('desktop_semantic_action', output)).toEqual({
        state: 'unknown',
        dispatchState: 'unknown',
      })
    }
  })

  it('does not reinterpret observation or unrelated tool results', () => {
    expect(desktopActionResult('desktop_semantic_observe', '{"candidates":[]}')).toBeNull()
    expect(desktopActionResult('desktop_windows_list', '[]')).toBeNull()
    expect(desktopActionResult('Read', '{"effect":"confirmed"}')).toBeNull()
    expect(desktopActionResult(undefined, 'anything')).toBeNull()
  })
})
