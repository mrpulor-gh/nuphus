import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useEvents, type EventHandlers } from '../hooks/useEvents'
import type { NuphusEvent } from '../core/types'

const { callbacks } = vi.hoisted(() => ({
  callbacks: new Map<string, (payload: unknown) => void>(),
}))

vi.mock('../core/bridge', () => ({
  invoke: vi.fn(async () => undefined),
  listen: vi.fn(async (name: string, cb: (payload: unknown) => void) => {
    callbacks.set(name, cb)
    return () => callbacks.delete(name)
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({}) }))
vi.mock('../ui/sound', () => ({ playUiSound: vi.fn() }))
vi.mock('../ui/islandChannel', () => ({ showAppFeedbackByHudPhase: vi.fn() }))

function setup() {
  const setApprovalState = vi.fn()
  const setSecurity = vi.fn()
  const h = {
    refs: {
      executionActiveRef: { current: false },
      processingRef: { current: false },
      interruptedRef: { current: false },
    },
    setApprovalState,
    setSecurity,
  } as unknown as EventHandlers
  renderHook(() => useEvents(h))
  return { setApprovalState, setSecurity }
}

function dispatch(event: NuphusEvent, seq = 1) {
  act(() => callbacks.get('nuphus-event')?.({ seq, event }))
}

describe('desktop approval event routing', () => {
  beforeEach(() => callbacks.clear())

  it('opens a one-shot approval instead of the generic session-security dialog', () => {
    const h = setup()
    dispatch({
      type: 'security_check',
      action_id: 'one',
      tool: 'desktop_action_approval',
      params: JSON.stringify({ title: 'Delete record', content: 'Temporary record' }),
      risk: 'critical',
      reason: 'Confirm once',
    })
    expect(h.setApprovalState).toHaveBeenCalledExactlyOnceWith({
      open: true,
      kind: 'desktop_action',
      title: 'Delete record',
      content: 'Temporary record',
      actionId: 'one',
      tenetCount: 0,
    })
    expect(h.setSecurity).not.toHaveBeenCalled()
  })

  it('keeps ordinary security prompts unchanged', () => {
    const h = setup()
    dispatch({
      type: 'security_check',
      action_id: 'normal',
      tool: 'system_shell',
      params: '{}',
      risk: 'high',
      reason: 'Existing security prompt',
    })
    expect(h.setSecurity).toHaveBeenCalledOnce()
    expect(h.setApprovalState).not.toHaveBeenCalled()
  })

  it('timeout only closes the matching pending approval', () => {
    const h = setup()
    dispatch({ type: 'prompt_timeout', action_id: 'one' })
    const update = h.setApprovalState.mock.calls[0][0] as (state: object) => object
    const pending = { open: true, actionId: 'one', kind: 'desktop_action' }
    expect(update(pending)).toEqual({ ...pending, open: false })
    const other = { ...pending, actionId: 'two' }
    expect(update(other)).toBe(other)
  })

  it('uses the host reason when display details are absent', () => {
    const h = setup()
    dispatch({
      type: 'security_check',
      action_id: 'one',
      tool: 'desktop_action_approval',
      params: 'null',
      risk: 'critical',
      reason: 'Confirm current action',
    })
    expect(h.setApprovalState.mock.calls[0][0]).toMatchObject({
      title: 'Confirm current action',
      content: 'Confirm current action',
    })
  })
})
