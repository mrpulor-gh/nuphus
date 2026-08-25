import { describe, expect, it } from 'vitest'
import { chatReducer, initialChatState, type ChatState } from './store'

describe('mobile session_changed sync', () => {
  it('clears the previous chat state while preserving mode and identity', () => {
    const state: ChatState = {
      ...initialChatState,
      messages: [{ id: 'old', role: 'user', content: '旧会话' }],
      activity: {
        running: true,
        goal: '旧任务',
        mode: 'custom',
        tools: [{ callId: 'call-1', name: 'Read' }],
        paused: true,
        pauseActionId: 'pause-1',
        startedAt: 1,
        pausedAt: 2,
      },
      pendingConfirm: {
        actionId: 'security-1',
        tool: 'Write',
        params: '{}',
        risk: 'high',
        reason: 'test',
      },
      pendingUserInput: {
        actionId: 'input-1',
        title: '输入',
        prompt: '请输入',
        sensitive: false,
        inputType: 'text',
      },
      refining: true,
      identity: { assistantName: 'Nuphus', userLabel: '用户' },
      tokenUsage: { inputTokens: 123 },
      workflowRun: { steps: [], isPaused: true, done: false },
    }

    const next = chatReducer(state, {
      type: 'event',
      event: { type: 'session_changed', session_id: 'new-session', source: 'desktop' },
    })

    expect(next.messages).toEqual([])
    expect(next.activity).toEqual({
      running: false,
      goal: '',
      mode: 'custom',
      tools: [],
      paused: false,
      pauseActionId: undefined,
      startedAt: undefined,
      pausedAt: undefined,
    })
    expect(next.pendingConfirm).toBeNull()
    expect(next.pendingUserInput).toBeNull()
    expect(next.refining).toBe(false)
    expect(next.tokenUsage).toBeUndefined()
    expect(next.workflowRun).toBeUndefined()
    expect(next.identity).toEqual(state.identity)
  })
})
