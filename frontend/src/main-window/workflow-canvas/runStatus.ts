/**
 * runStatus.ts — workflow-event → 画布执行状态叠加（设计文档 2.3）
 *
 * - 画布内独立 listener，不复用 useExecutionUI（避免与 HUD 面板状态耦合）
 * - 会话边界：step 级事件无 run_id（events.rs），以最近 RunStarted{workflow_id}
 *   为界清空上一运行状态；非当前 workflow 的事件忽略
 * - 性能：loop 迭代会对同一 step_id 反复发事件 → rAF 合帧 + 状态相等跳过；
 *   StepRunOutput 不进画布节点，仅保留最近 3 行供 Inspector 展示
 * - 终态对账：通道洪峰只影响显示实时性，RunCompleted 后由 CanvasPage
 *   重新拉取 run_history 作为权威回放源
 */

import { listen } from '../../core/bridge'
import type { ProjectionIndex, StepVisualStatus } from './types'

export interface RunStatusSnapshot {
  /** stepId → 视觉状态 */
  steps: ReadonlyMap<string, StepVisualStatus>
  /** stepId → 最近输出（最多 3 行，Inspector 用） */
  outputs: ReadonlyMap<string, string[]>
  /** 当前 workflow 正在运行（RunStarted 后、RunCompleted 前） */
  running: boolean
  /** 最近一次终态（RunCompleted.status 的文本描述） */
  lastTerminal?: string
}

type Listener = (snap: RunStatusSnapshot) => void

interface PendingMutation {
  status?: StepVisualStatus
  outputLine?: string
}

/** 状态相等（跳过无意义重渲染） */
function sameStatus(a: StepVisualStatus | undefined, b: StepVisualStatus | undefined): boolean {
  if (a === b) return true
  if (!a || !b) return false
  if (a.state !== b.state) return false
  if (a.state === 'retrying' && b.state === 'retrying') return a.attempt === b.attempt
  if (a.state === 'error' && b.state === 'error') return a.message === b.message
  if (a.state === 'paused' && b.state === 'paused') return a.reason === b.reason
  return true
}

/** StepRunStatus serde 形状：'Success' | 'Skipped' | 'Running' | { Error: string } */
function parseStepStatus(raw: unknown): StepVisualStatus | null {
  if (raw === 'Success') return { state: 'success' }
  if (raw === 'Skipped') return { state: 'skipped' }
  if (typeof raw === 'object' && raw !== null && 'Error' in (raw as object)) {
    return { state: 'error', message: String((raw as { Error: unknown }).Error) }
  }
  return null
}

function terminalText(raw: unknown): string {
  if (typeof raw === 'string') return raw
  if (typeof raw === 'object' && raw !== null && 'Error' in (raw as object)) {
    return `Error: ${String((raw as { Error: unknown }).Error)}`
  }
  return String(raw)
}

/**
 * 订阅指定 workflow 的执行事件。
 * 返回 dispose；onChange 在 rAF 合帧后以新快照回调（不可变语义）。
 */
export function subscribeRunStatus(workflowId: string, onChange: Listener): () => void {
  let steps = new Map<string, StepVisualStatus>()
  let outputs = new Map<string, string[]>()
  let running = false
  let lastTerminal: string | undefined

  const pending = new Map<string, PendingMutation>()
  let rafId: number | null = null
  let dirtyMeta = false

  const flush = () => {
    rafId = null
    let changed = dirtyMeta
    dirtyMeta = false
    for (const [stepId, mut] of pending) {
      if (mut.status && !sameStatus(steps.get(stepId), mut.status)) {
        steps.set(stepId, mut.status)
        changed = true
      }
      if (mut.outputLine !== undefined) {
        const prev = outputs.get(stepId) ?? []
        const next = [...prev, mut.outputLine].slice(-3)
        outputs.set(stepId, next)
        changed = true
      }
    }
    pending.clear()
    if (changed) {
      // 快照不可变语义：回调前复制，React 可靠引用比较跳过
      onChange({
        steps: new Map(steps),
        outputs: new Map(outputs),
        running,
        lastTerminal,
      })
    }
  }

  const schedule = () => {
    if (rafId === null) rafId = requestAnimationFrame(flush)
  }

  const unlisten = listen<Record<string, unknown>>('workflow-event', event => {
    const payload = (event as { payload?: Record<string, unknown> }).payload ?? (event as Record<string, unknown>)
    const type = String(payload.event ?? '')

    switch (type) {
      case 'run_started': {
        if (payload.workflow_id !== workflowId) return
        // 会话边界：新一轮运行清空上一运行状态
        steps = new Map()
        outputs = new Map()
        running = true
        lastTerminal = undefined
        dirtyMeta = true
        schedule()
        break
      }
      case 'step_run_started': {
        if (!running) return
        pending.set(String(payload.step_id), {
          ...pending.get(String(payload.step_id)),
          status: { state: 'running' },
        })
        schedule()
        break
      }
      case 'step_run_retry': {
        if (!running) return
        const id = String(payload.step_id)
        pending.set(id, {
          ...pending.get(id),
          status: { state: 'retrying', attempt: Number(payload.attempt ?? 0) },
        })
        schedule()
        break
      }
      case 'step_run_output': {
        if (!running) return
        const id = String(payload.step_id)
        pending.set(id, {
          ...pending.get(id),
          outputLine: String(payload.text ?? ''),
        })
        schedule()
        break
      }
      case 'step_run_completed': {
        if (!running) return
        const st = parseStepStatus(payload.status)
        if (!st) return
        const id = String(payload.step_id)
        pending.set(id, { ...pending.get(id), status: st })
        schedule()
        break
      }
      case 'step_run_paused': {
        if (!running) return
        const id = String(payload.step_id)
        pending.set(id, {
          ...pending.get(id),
          status: { state: 'paused', reason: String(payload.reason ?? '') },
        })
        schedule()
        break
      }
      case 'run_completed': {
        // run_completed 携带 run_id 无 workflow_id——以 running 标志为会话门
        if (!running) return
        running = false
        lastTerminal = terminalText(payload.status)
        dirtyMeta = true
        schedule()
        break
      }
    }
  })

  return () => {
    if (rafId !== null) cancelAnimationFrame(rafId)
    pending.clear()
    // core/bridge listen 返回 unlisten Promise（对齐 useExecutionUI 用法）
    void Promise.resolve(unlisten as unknown as Promise<() => void>).then(fn => fn?.())
  }
}

/** 容器聚合徽标（2.3）：子树内 running/error 计数上浮到各层容器，红点优先于蓝点 */
export function aggregateContainerBadges(
  index: ProjectionIndex,
  statuses: ReadonlyMap<string, StepVisualStatus>,
): Map<string, { running: number; error: number }> {
  const badges = new Map<string, { running: number; error: number }>()
  for (const [stepId, st] of statuses) {
    const isRunning = st.state === 'running' || st.state === 'retrying'
    const isError = st.state === 'error'
    if (!isRunning && !isError) continue
    let cur = index.parentOf.get(stepId) ?? null
    while (cur) {
      const b = badges.get(cur) ?? { running: 0, error: 0 }
      if (isRunning) b.running++
      if (isError) b.error++
      badges.set(cur, b)
      cur = index.parentOf.get(cur) ?? null
    }
  }
  return badges
}
