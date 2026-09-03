/**
 * recorderApi.ts — rec_* 命令 + overlay 复用的薄封装（与 DesktopToolbar tauriInvoke 同模式）
 *
 * 契约（main.rs 已注册，全部直接 invoke）：
 * - rec_set_workflow(workflowId) → RecSessionInfo
 * - rec_session_status() → RecSessionInfo
 * - rec_start(actionKind, timeout_secs?) → CaptureEvent（阻塞至捕获，默认 60s）
 * - rec_cancel() / rec_abort()
 * - rec_complete(steps, user_notes?) → { path, workflow_id, created_at, step_count }
 * - rec_save_pending(steps, user_notes?) → { path, workflow_id, step_count }（回 idle）
 * - rec_load_pending() → { exists:true, ... } | { exists:false }
 * - rec_discard_pending() → void（幂等）
 * - start_overlay_mask(mode) / take_capture_result()（既有 capture overlay 通道）
 */

import type { CaptureEvent, RecDraft, RecRect, RecSessionInfo } from './recorderTypes'

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    return (await invoke(cmd, args)) as T
  } catch (e: any) {
    const parts: string[] = []
    if (e?.message) parts.push(e.message)
    if (typeof e === 'string') parts.push(e)
    try {
      parts.push(JSON.stringify(e))
    } catch {}
    const msg = parts.filter(Boolean).join(' | ') || '未知错误'
    throw new Error(`Tauri invoke ${cmd} failed: ${msg}`)
  }
}

export function recSetWorkflow(workflowId: string): Promise<RecSessionInfo> {
  return tauriInvoke<RecSessionInfo>('rec_set_workflow', { workflowId })
}

export function recSessionStatus(): Promise<RecSessionInfo> {
  return tauriInvoke<RecSessionInfo>('rec_session_status')
}

export function recStart(
  actionKind: 'click' | 'scroll' | 'hotkey',
  timeoutSecs?: number,
): Promise<CaptureEvent> {
  return tauriInvoke<CaptureEvent>('rec_start', { actionKind, timeoutSecs })
}

export function recCancel(): Promise<void> {
  return tauriInvoke<void>('rec_cancel').then(() => undefined)
}

export function recAbort(): Promise<void> {
  return tauriInvoke<void>('rec_abort').then(() => undefined)
}

export interface RecCompleteResult {
  path: string
  workflow_id: string
  created_at: string
  step_count: number
}

export function recComplete(
  steps: unknown[],
  userNotes?: string | null,
): Promise<RecCompleteResult> {
  return tauriInvoke<RecCompleteResult>('rec_complete', {
    steps,
    userNotes: userNotes || null,
  })
}

// ── 进度持久化（pending checkpoint）──

export interface RecSavePendingResult {
  path: string
  workflow_id: string
  step_count: number
}

export type RecLoadPendingResult =
  | {
      exists: true
      workflow_id: string
      saved_at: string
      steps: RecDraft[]
      user_notes?: string | null
    }
  | { exists: false }

/** 保存进度：drafts → record-draft.pending.json，会话回 idle（下次 begin 自动恢复） */
export function recSavePending(
  steps: RecDraft[],
  userNotes?: string | null,
): Promise<RecSavePendingResult> {
  return tauriInvoke<RecSavePendingResult>('rec_save_pending', {
    steps,
    userNotes: userNotes || null,
  })
}

/** 读取待恢复进度（begin 自动恢复；不存在 → { exists:false }） */
export function recLoadPending(): Promise<RecLoadPendingResult> {
  return tauriInvoke<RecLoadPendingResult>('rec_load_pending')
}

/** 幂等删除当前会话 workflow 的 pending 文件（清空草稿后调用；不存在也 Ok） */
export function recDiscardPending(): Promise<void> {
  return tauriInvoke<void>('rec_discard_pending').then(() => undefined)
}

export function startOverlayMask(mode: 'rec_region' | 'rec_template'): Promise<void> {
  return tauriInvoke<void>('start_overlay_mask', { mode }).then(() => undefined)
}

export interface OverlayRaw {
  cancelled?: boolean
  path?: string
  region?: RecRect
  base64?: string | null
}

export function takeCaptureResult(): Promise<OverlayRaw | null> {
  return tauriInvoke<OverlayRaw | null>('take_capture_result')
}

const delay = (ms: number) => new Promise<void>(r => setTimeout(r, ms))

/** overlay 结果轮询：overlay_capture_done/cancel 后主窗自动恢复，前端轮询 take_capture_result */
export async function pollOverlayResult(timeoutMs = 120000): Promise<OverlayRaw | null> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    const raw = await takeCaptureResult()
    if (raw) return raw
    await delay(400)
  }
  return null
}
// ── 浏览器网页点击录制（rec_browser_*：CDP 注入捕获真实点击 → 稳定 CSS selector）──

/** rec_browser_capture_click_start 返回：注入捕获监听（幂等） */
export interface RecBrowserStartResult {
  ok: boolean
  /** 当前页地址（前端展示 / draft 上下文） */
  url?: string
  /** true=本次完成注入；false=同 document 已注入（复用已有监听） */
  injected?: boolean
  already?: boolean
  note?: string
}

/** rec_browser_capture_click_poll 四态返回（每态一次消费清空） */
export interface RecBrowserPollResult {
  captured: boolean
  selector?: string
  tag?: string
  text?: string
  href?: string | null
  ts?: number
  /** disabled/无法生成 selector：提示重试 */
  error?: string
  /** 整页导航注入丢失 → 前端应自动再次 start 重注入后继续等 */
  need_reinject?: boolean
}

/** 开始捕获网页点击：注入页面监听（幂等）。Err = 会话未初始化/浏览器未就绪/无页面/about:blank/不可注入页 */
export function recBrowserCaptureClickStart(): Promise<RecBrowserStartResult> {
  return tauriInvoke<RecBrowserStartResult>('rec_browser_capture_click_start')
}

/** 轮询读取最近一次点击捕获（读后清空） */
export function recBrowserCaptureClickPoll(): Promise<RecBrowserPollResult> {
  return tauriInvoke<RecBrowserPollResult>('rec_browser_capture_click_poll')
}

/** 取消/清理点击捕获（幂等：清结果保留注入监听） */
export function recBrowserCaptureClickCancel(): Promise<void> {
  return tauriInvoke<void>('rec_browser_capture_cancel').then(() => undefined)
}
