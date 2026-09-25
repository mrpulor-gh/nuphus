import { afterEach, expect, it, vi } from 'vitest'
import { editorEn } from '../../locales/workflowEditor'
import { subscribeRunStatus, type RunStatusSnapshot } from './runStatus'

const bridge = vi.hoisted(() => ({ receive: (_: unknown) => {} }))
vi.mock('../../core/bridge', () => ({
  listen: vi.fn((_topic, callback) => {
    bridge.receive = callback
    return Promise.resolve(() => {})
  }),
}))
afterEach(() => vi.unstubAllGlobals())

it('localizes progress and keeps a completed node completed at an inclusive breakpoint', () => {
  let flush: FrameRequestCallback = () => {}
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    flush = callback
    return 1
  })
  vi.stubGlobal('cancelAnimationFrame', vi.fn())
  const snapshots: RunStatusSnapshot[] = []
  const dispose = subscribeRunStatus(
    'wf',
    snapshot => snapshots.push(snapshot),
    (key, ...args) => {
      let text = editorEn[key] ?? key
      args.forEach((value, index) => {
        text = text.replace(`{${index}}`, () => value)
      })
      return text
    },
  )
  const emit = (payload: Record<string, unknown>) => {
    bridge.receive({ payload })
    flush(0)
  }
  emit({ event: 'run_started', workflow_id: 'wf' })
  emit({ event: 'step_run_started', step_id: 's', step_name: 'Save' })
  emit({ event: 'step_run_completed', step_id: 's', step_name: 'Save', status: 'Success' })
  emit({ event: 'step_run_paused', step_id: 's', reason: 'debug_after_step' })
  const latest = snapshots[snapshots.length - 1]
  expect(latest.steps.get('s')).toEqual({ state: 'success' })
  expect(latest.timeline.map(entry => entry.message)).toEqual([
    'Workflow started',
    'Started · Save',
    'Completed · Save',
    'Paused · Selected node completed',
  ])
  dispose()
})
