// useToolCapture.ts — 截图 / 选区 / 鼠标 全屏捕获 hook
// 从 DesktopToolbar.tsx 提取，Promise-based 异步返回给调用方
//
// flow: start_overlay_mask → 轮询 take_capture_result → Promise resolve
// 支持取消 / 超时 (60s) / 并发保护
//
// CaptureOverlay modes:
//   'screenshot' → 拖拽框选 → 预览 → 确认 → {path, region}
//   'picker'     → 拖拽框选 → 自动确认 → {region}
//   'mouse_pos'  → 点击单点 → {region: {x,y,1,1}}（调用方提取 x,y）

import { useRef, useCallback, useEffect } from 'react'

// ══════════════════════════════════════════════
// Types
// ══════════════════════════════════════════════

export type CaptureMode = 'screenshot' | 'picker' | 'mouse_pos'

/// 将 request_user_input 的 input_type 映射到 CaptureMode
export function inputTypeToCaptureMode(inputType: string): CaptureMode {
  switch (inputType) {
    case 'screenshot':
      return 'screenshot'
    case 'region':
      return 'picker'
    case 'mouse_pos':
      return 'mouse_pos'
    case 'color':
      return 'picker' // 取色也走框选流程，用户框选区域后后端提取颜色
    default:
      return 'screenshot'
  }
}

export interface CaptureRegion {
  x: number
  y: number
  width: number
  height: number
}

export interface CaptureResult {
  mode: CaptureMode
  path?: string
  region?: CaptureRegion
  cancelled: boolean
}

// ══════════════════════════════════════════════
// Tauri invoke wrapper (same as DesktopToolbar)
// ══════════════════════════════════════════════

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
    } catch {
      /* ignore */
    }
    const msg = parts.filter(Boolean).join(' | ') || '未知错误'
    throw new Error(`Tauri invoke ${cmd} failed: ${msg}`)
  }
}

// ══════════════════════════════════════════════
// Constants
// ══════════════════════════════════════════════

const POLL_INTERVAL_MS = 500
const CAPTURE_TIMEOUT_MS = 60_000

// ══════════════════════════════════════════════
// Hook
// ══════════════════════════════════════════════

export function useToolCapture() {
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const pendingRef = useRef<{
    resolve: (value: CaptureResult) => void
    reject: (reason: Error) => void
  } | null>(null)

  // ── Stop polling ──
  const stopPolling = useCallback(() => {
    if (pollRef.current !== null) {
      clearInterval(pollRef.current)
      pollRef.current = null
    }
  }, [])

  // ── Cleanup on unmount ──
  useEffect(() => {
    return () => {
      stopPolling()
      if (pendingRef.current) {
        pendingRef.current.reject(new Error('组件已卸载'))
        pendingRef.current = null
      }
    }
  }, [stopPolling])

  // ── capture(mode): Promise<CaptureResult> ──
  const capture = useCallback(
    (mode: CaptureMode): Promise<CaptureResult> => {
      // ── Cancel any in-progress capture ──
      if (pendingRef.current) {
        const prev = pendingRef.current
        pendingRef.current = null
        // Best-effort cancel the overlay on the backend side
        tauriInvoke('overlay_capture_cancel').catch(() => {})
        prev.reject(new Error('已被新的捕获操作取代'))
      }
      stopPolling()

      return new Promise<CaptureResult>(async (resolve, reject) => {
        // ── Completion helpers ──
        const done = (result: CaptureResult) => {
          stopPolling()
          pendingRef.current = null
          resolve(result)
        }

        const fail = (err: Error) => {
          stopPolling()
          pendingRef.current = null
          reject(err)
        }

        pendingRef.current = { resolve, reject }

        // ── Phase 1: start overlay ──
        try {
          await tauriInvoke('start_overlay_mask', { mode })
        } catch (e: any) {
          fail(e instanceof Error ? e : new Error(String(e)))
          return
        }

        const startedAt = Date.now()

        // ── Phase 2: poll take_capture_result ──
        pollRef.current = setInterval(async () => {
          try {
            const raw = await tauriInvoke<any>('take_capture_result')

            if (raw === null || raw === undefined) {
              // No result yet — check timeout
              if (Date.now() - startedAt > CAPTURE_TIMEOUT_MS) {
                fail(new Error('截图超时（60秒）'))
              }
              return
            }

            // Got a result — stop polling
            stopPolling()

            // User cancelled (Esc / right-click in overlay)
            if (raw.cancelled) {
              done({ mode, cancelled: true })
              return
            }

            // Parse region from raw result
            const region: CaptureRegion | undefined = raw.region
              ? {
                  x: raw.region.x as number,
                  y: raw.region.y as number,
                  width: raw.region.width as number,
                  height: raw.region.height as number,
                }
              : undefined

            done({
              mode,
              path: (raw.path as string) || undefined,
              region,
              cancelled: false,
            })
          } catch (e: any) {
            // Polling error — reject
            fail(e instanceof Error ? e : new Error(String(e)))
          }
        }, POLL_INTERVAL_MS)
      })
    },
    [stopPolling],
  )

  return { capture }
}
