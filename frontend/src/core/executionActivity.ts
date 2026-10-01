import type { NuphusEvent } from './types'

/** Presentation-only projection of execution events, never an execution authority. */
export interface ExecutionActivity {
  startedAt: number
  phase: 'preparing' | 'tool' | 'workflow' | 'user' | 'model'
  calls: string[]
  waitingFor?: string
}

export function projectExecutionActivity(
  current: ExecutionActivity | null,
  event: NuphusEvent,
  now = Date.now(),
): ExecutionActivity | null {
  // 起点优先取后端权威值（`started_at_ms`，绝对 Unix 毫秒）——刷新 / 重连后
  // 该值不变，`now - startedAt` 仍从真实起点继续走，不停在刷新时刻归零
  // （旧实现恒用前端 `now`，与 ChatInputBar 的 ref 起点是同一类失真）。
  // 旧后端不带该字段时才退化为收到事件的时刻（能力所限，非前端臆造起点）。
  if (event.type === 'execution_started')
    return { startedAt: event.started_at_ms ?? now, phase: 'preparing', calls: [] }
  if (
    [
      'execution_completed',
      'execution_error',
      'direct_response',
      'session_changed',
      'new_chat_broadcast',
      'refine_executing',
    ].includes(event.type) ||
    (event.type === 'error' && !event.from_subtask)
  )
    return null
  if (!current) return null
  switch (event.type) {
    case 'tool_call_start':
      return {
        ...current,
        phase: 'tool',
        waitingFor: undefined,
        calls: [...new Set([...current.calls, event.call_id])],
      }
    case 'tool_call_end': {
      const calls = current.calls.filter(id => id !== event.call_id)
      return { ...current, calls, waitingFor: undefined, phase: calls.length ? 'tool' : 'model' }
    }
    case 'llm_text_delta':
      return current.calls.length ? current : { ...current, waitingFor: undefined, phase: 'model' }
    case 'execution_paused':
    case 'user_input_request':
    case 'security_check':
      return { ...current, phase: 'user', waitingFor: event.action_id }
    case 'prompt_timeout':
      return current.waitingFor === event.action_id
        ? { ...current, waitingFor: undefined, phase: current.calls.length ? 'tool' : 'model' }
        : current
    case 'workflow_event':
      if (event.event === 'step_run_paused') return { ...current, phase: 'user' }
      if (event.event === 'step_run_started' || event.event === 'run_started')
        return { ...current, phase: 'workflow' }
      if (event.event === 'run_completed')
        return { ...current, phase: current.calls.length ? 'tool' : 'model' }
      return current
    default:
      return current
  }
}

export const activityLabels: Record<ExecutionActivity['phase'], string> = {
  preparing: '正在准备执行',
  tool: '正在执行工具',
  workflow: '正在运行工作流步骤',
  user: '正在等待用户回应',
  model: '正在等待模型响应',
}
