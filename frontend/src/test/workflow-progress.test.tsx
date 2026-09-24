import { act, render, renderHook, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { composeAssistantReplies, foldAssistantHistory } from '../core/progressMessages'
import { chatReducer, initialChatState } from '../mobile/store'
import { useEvents, type EventHandlers } from '../hooks/useEvents'
import type { ChatMessage, NuphusEvent } from '../core/types'
import { projectExecutionActivity } from '../core/executionActivity'
import { ExecutionActivityLine } from '../ui/ExecutionActivityLine'

const { callbacks } = vi.hoisted(() => ({
  callbacks: new Map<string, (payload: unknown) => void>(),
}))
vi.mock('../core/bridge', () => ({
  invoke: vi.fn(async () => undefined),
  listen: vi.fn(async (name: string, callback: (payload: unknown) => void) => {
    callbacks.set(name, callback)
    return () => callbacks.delete(name)
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }))
vi.mock('../ui/sound', () => ({ playUiSound: vi.fn() }))
vi.mock('../ui/islandChannel', () => ({ showAppFeedbackByHudPhase: vi.fn() }))

const started: NuphusEvent = {
  type: 'execution_started',
  session_id: 'session',
  turn_id: 'turn',
  step_index: 0,
  goal: 'Temporary file task',
  tools: [],
  source: 'workflow',
  mode: 'workflow',
}
const progress = (overrides = {}): Extract<NuphusEvent, { type: 'assistant_progress' }> => ({
  type: 'assistant_progress',
  session_id: 'session',
  turn_id: 'turn',
  message_id: 'p1',
  text: '先检查已有流程。',
  timestamp: 1000,
  replaces_draft: false,
  ...overrides,
})
const completed: NuphusEvent = {
  type: 'execution_completed',
  step_index: 0,
  output: { result_message: '完成了。', step_index: 0, artifacts: [], tool_calls_count: 0 },
  total_duration_ms: 0,
  total_calls: 0,
}
const text: NuphusEvent = {
  type: 'llm_text_delta',
  text: '先检查已有流程。',
  is_thinking: false,
  from_task: false,
}

function desktop() {
  let messages: ChatMessage[] = []
  const refs = {
    streamingMsgId: { current: null },
    lastStreamingMsgId: { current: null },
    executionActiveRef: { current: false },
    processingRef: { current: false },
    toolCallCountRef: { current: 0 },
    interruptedRef: { current: false },
  }
  const handlers = new Proxy(
    {
      refs,
      messages,
      setMessages: (value: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => {
        messages = typeof value === 'function' ? value(messages) : value
      },
      addMessage: (message: ChatMessage) => {
        messages.push(message)
      },
    },
    { get: (target, name) => (name in target ? target[name as keyof typeof target] : vi.fn()) },
  )
  renderHook(() => useEvents(handlers as unknown as EventHandlers))
  let seq = 0
  const send = (event: NuphusEvent) =>
    act(() => callbacks.get('nuphus-event')?.({ seq: ++seq, event }))
  send(started)
  return { send, messages: () => composeAssistantReplies(messages), records: () => messages, refs }
}

describe('workflow progress narration', () => {
  afterEach(() => {
    callbacks.clear()
    vi.useRealTimers()
  })
  it('desktop keeps narration and final in the same reply', () => {
    const h = desktop()
    h.send(progress())
    h.send(completed)
    expect(h.messages().map(m => m.content)).toEqual(['先检查已有流程。\n\n完成了。'])
    expect(h.messages()[0].runtime).toBe('done')
  })
  it('desktop converts streamed text once and ignores duplicate/old turn events', () => {
    const h = desktop()
    h.send(text)
    h.send(progress({ replaces_draft: true }))
    h.send(progress({ replaces_draft: true }))
    h.send(progress({ turn_id: 'old', message_id: 'old' }))
    expect(h.messages().map(m => m.content)).toEqual(['先检查已有流程。'])
    h.send(completed)
    h.send(progress({ message_id: 'late' }))
    expect(h.messages()).toHaveLength(1)
  })
  it('desktop preserves progress on errors and rejects switched-session events', () => {
    const h = desktop()
    h.send(progress())
    h.send({ type: 'execution_error', step_index: 0, error: 'Stopped' })
    expect(h.messages()[0].content).toBe('先检查已有流程。')
    h.send({ type: 'new_chat_broadcast' })
    h.send(progress({ message_id: 'late' }))
    expect(h.messages()).toHaveLength(1)
  })
  it('desktop appends final streaming text and keeps one stable presentation container', () => {
    const h = desktop()
    h.send(text)
    const id = h.messages()[0].id
    h.send(progress({ replaces_draft: true }))
    h.send(progress({ message_id: 'p2', text: '正在验证。' }))
    h.send({ ...text, text: '完成' })
    expect(h.messages()).toHaveLength(1)
    expect(h.messages()[0].id).toBe(id)
    expect(h.messages()[0].content).toBe('先检查已有流程。\n\n正在验证。\n\n完成')
    h.send(completed)
    expect(h.messages()[0].content).toBe('先检查已有流程。\n\n正在验证。\n\n完成了。')
  })
  it('desktop output after an appended user message stays after that message', () => {
    const h = desktop()
    h.send(progress())
    h.send({ type: 'user_message_received', source: 'mobile', content: '改用新目录' })
    h.send(progress({ message_id: 'p2', text: '按新目录继续。' }))
    h.send(completed)
    expect(h.messages().map(m => m.content)).toEqual([
      '先检查已有流程。',
      '改用新目录',
      '按新目录继续。\n\n完成了。',
    ])
    expect(new Set(h.messages().map(m => m.id)).size).toBe(3)
  })
  it('desktop does not leave an empty bubble when stopped before any public text', () => {
    const h = desktop()
    h.send({ type: 'execution_error', step_index: 0, error: 'Stopped' })
    expect(h.messages()).toHaveLength(0)
  })
  it('mobile preserves a progress-only reply on pause and interruption without placeholders', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: progress() })
    state = chatReducer(state, {
      type: 'event',
      event: { type: 'execution_paused', action_id: 'a' },
    })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。',
    ])
    state = chatReducer(state, { type: 'sync_running', running: false })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。',
    ])
  })
  it('mobile keeps subsequent output after the user append', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: progress() })
    state = chatReducer(state, {
      type: 'optimistic',
      message: { id: 'u2', role: 'user', content: '追加' },
    })
    state = chatReducer(state, {
      type: 'event',
      event: progress({ message_id: 'p2', text: '收到。' }),
    })
    state = chatReducer(state, { type: 'event', event: completed })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。',
      '追加',
      '收到。\n\n完成了。',
    ])
  })
  it('history composition respects user and refine boundaries and preserves media', () => {
    const messages: ChatMessage[] = [
      {
        id: 'p1',
        message_id: 'p1',
        kind: 'progress',
        role: 'assistant',
        content: '检查。',
        timestamp: 1,
      },
      {
        id: 'p2',
        message_id: 'p2',
        kind: 'progress',
        role: 'assistant',
        content: '检查。',
        timestamp: 2,
      },
      {
        id: 'f1',
        role: 'assistant',
        content: '完成。',
        images: ['image'],
        audio: ['audio'],
        timestamp: 3,
      },
      { id: 'r', role: 'refine', content: '提炼', timestamp: 4 },
      { id: 'p3', kind: 'progress', role: 'assistant', content: '新阶段', timestamp: 5 },
      { id: 'u', role: 'user', content: '新问题', timestamp: 6 },
      { id: 'f2', role: 'assistant', content: '新回复', timestamp: 7 },
    ]
    const visible = composeAssistantReplies(foldAssistantHistory(messages))
    expect(visible.map(m => m.content)).toEqual([
      '检查。\n\n检查。\n\n完成。',
      '提炼',
      '新阶段',
      '新问题',
      '新回复',
    ])
    expect(visible[0]).toMatchObject({ timestamp: 1, images: ['image'], audio: ['audio'] })
    expect(composeAssistantReplies(visible)).toEqual(visible)
  })
  it('mobile preserves converted text, standalone reports and final response', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    for (const event of [
      text,
      progress({ replaces_draft: true }),
      progress({ replaces_draft: true }),
      progress({ message_id: 'p2', text: '正在验证结果。' }),
      completed,
    ]) {
      state = chatReducer(state, { type: 'event', event })
    }
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。\n\n正在验证结果。\n\n完成了。',
    ])
  })
  it('mobile ignores stale progress and does not expose subtask text as narration', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: progress({ session_id: 'other' }) })
    state = chatReducer(state, { type: 'event', event: { ...text, from_task: true } })
    expect(state.messages.every(m => !m.content)).toBe(true)
  })
  it('mobile history reconciliation deduplicates ids, not repeated progress text', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: progress() })
    state = chatReducer(state, { type: 'event', event: progress({ message_id: 'p2' }) })
    state = chatReducer(state, {
      type: 'history_merge',
      messages: state.messages.filter(m => m.kind === 'progress'),
    })
    expect(state.messages.filter(m => m.kind === 'progress')).toHaveLength(2)
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。\n\n先检查已有流程。',
    ])
  })
  it('mobile reconnect binds missing scope only after authoritative running recovery', () => {
    let state = chatReducer(initialChatState, {
      type: 'session_snapshot',
      messages: [],
      welcome: false,
      running: true,
    })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages).toHaveLength(0)
    state = chatReducer(state, { type: 'sync_running', running: true })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.activity.session_id).toBe('session')
    expect(state.activity.turn_id).toBe('turn')
    expect(state.messages.filter(m => m.kind === 'progress')).toHaveLength(1)
    state = chatReducer(state, {
      type: 'event',
      event: progress({ message_id: 'old', turn_id: 'old' }),
    })
    expect(state.messages.filter(m => m.kind === 'progress')).toHaveLength(1)
  })
  it('mobile reload recovers native text conversion and finishes without overwriting progress', () => {
    let state = chatReducer(initialChatState, { type: 'sync_running', running: true })
    state = chatReducer(state, { type: 'event', event: text })
    state = chatReducer(state, { type: 'event', event: progress({ replaces_draft: true }) })
    state = chatReducer(state, { type: 'event', event: completed })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。\n\n完成了。',
    ])
  })
  it('mobile reconnect binds a history duplicate without duplicating or losing later progress', () => {
    let state = chatReducer(initialChatState, {
      type: 'history',
      messages: [
        {
          id: 'p1',
          message_id: 'p1',
          kind: 'progress',
          role: 'assistant',
          content: '先检查已有流程。',
        },
      ],
    })
    state = chatReducer(state, { type: 'sync_running', running: true })
    state = chatReducer(state, { type: 'event', event: progress() })
    state = chatReducer(state, { type: 'event', event: progress({ message_id: 'p2' }) })
    expect(state.messages.filter(m => m.kind === 'progress')).toHaveLength(2)
    expect(state.activity.progressScopeRecovery).toBeUndefined()
    expect(composeAssistantReplies(state.messages)).toHaveLength(1)
  })
  it('explicit new chat blocks stale recovery even if a late running response arrives', () => {
    let state = chatReducer(initialChatState, { type: 'sync_running', running: true })
    state = chatReducer(state, { type: 'new_chat' })
    state = chatReducer(state, { type: 'sync_running', running: true })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages).toHaveLength(0)
    state = chatReducer(state, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages.filter(m => m.kind === 'progress')).toHaveLength(1)
  })
  it('broadcast session changes close the recovery gate and idle recovery ignores events', () => {
    let state = chatReducer(initialChatState, { type: 'sync_running', running: true })
    state = chatReducer(state, {
      type: 'event',
      event: { type: 'session_changed', session_id: 'other' },
    })
    state = chatReducer(state, { type: 'sync_running', running: true })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages).toHaveLength(0)
    state = chatReducer(initialChatState, { type: 'sync_running', running: false })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages).toHaveLength(0)
  })
  it('history preserves receipt IDs internally but renders one reply without mutating traces', () => {
    const history = [
      { role: 'user', content: 'go' },
      { role: 'assistant', content: '', traceItems: ['tool'] },
      { role: 'assistant', content: 'checking', kind: 'progress' as const, message_id: 'p1' },
      { role: 'assistant', content: 'checking', kind: 'progress' as const, message_id: 'p1' },
      { role: 'assistant', content: '', traceItems: ['another tool'] },
      { role: 'assistant', content: 'done' },
    ]
    const folded = foldAssistantHistory(history)
    expect(folded.map(m => m.content)).toEqual(['go', 'checking', 'done'])
    expect(folded[1].traceItems).toEqual(['tool'])
    expect(history[2].traceItems).toBeUndefined()
    const visible = composeAssistantReplies(folded.map((m, i) => ({ ...m, id: String(i) })))
    expect(visible.map(m => m.content)).toEqual(['go', 'checking\n\ndone'])
    expect(visible[1].traceItems).toEqual(['tool', 'another tool'])
  })
  it('old history still folds consecutive assistant rounds', () => {
    expect(
      foldAssistantHistory([
        { role: 'assistant', content: 'old' },
        { role: 'assistant', content: 'final' },
      ]),
    ).toEqual([{ role: 'assistant', content: 'final', traceItems: [] }])
  })
  it('history retains audio-only answers and tool traces after the last progress', () => {
    const folded = foldAssistantHistory([
      { role: 'assistant', content: 'checking', kind: 'progress' as const },
      { role: 'assistant', content: '', traceItems: ['last tool'] },
      { role: 'user', content: 'next' },
      { role: 'assistant', content: '', audio: ['data:audio/test'] },
    ])
    expect(folded).toHaveLength(3)
    expect(folded[0].traceItems).toEqual(['last tool'])
    expect(folded[2].audio).toEqual(['data:audio/test'])
  })
  it('mobile rejects progress while refining or after session switching', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: { type: 'refine_executing' } })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages.some(m => m.kind === 'progress')).toBe(false)
    state = chatReducer(state, { type: 'event', event: { type: 'new_chat_broadcast' } })
    state = chatReducer(state, { type: 'event', event: progress() })
    expect(state.messages.some(m => m.kind === 'progress')).toBe(false)
    expect(state.activity.detail).toBeNull()
  })
  it('status reports only event facts, tracks concurrent tools and resets on refine', () => {
    let state = projectExecutionActivity(null, started, 1000)
    for (const id of ['1', '2'])
      state = projectExecutionActivity(state, {
        type: 'tool_call_start',
        call_id: id,
        tool_name: 'secret command',
        params: 'private',
        iteration: 1,
        from_task: false,
      })
    state = projectExecutionActivity(state, {
      type: 'tool_call_end',
      call_id: '1',
      tool_name: 'secret command',
      success: true,
      duration_ms: 1,
      output_preview: '',
      from_task: false,
      output_full_size: 0,
      is_truncated: false,
      error: null,
    })
    expect(state?.phase).toBe('tool')
    expect(JSON.stringify(state)).not.toContain('private')
    expect(projectExecutionActivity(state, { type: 'refine_executing' })).toBeNull()
  })
  it('status elapsed time stops when inactive and timer cleans up', () => {
    vi.useFakeTimers()
    vi.setSystemTime(1000)
    const view = render(
      <ExecutionActivityLine activity={{ startedAt: 1000, phase: 'model', calls: [] }} />,
    )
    act(() => {
      vi.advanceTimersByTime(3000)
    })
    expect(screen.getByRole('status')).toHaveTextContent('正在等待模型响应 · 已用时 3 秒')
    view.rerender(<ExecutionActivityLine activity={null} />)
    expect(screen.queryByRole('status')).toBeNull()
    expect(vi.getTimerCount()).toBe(0)
  })
})
