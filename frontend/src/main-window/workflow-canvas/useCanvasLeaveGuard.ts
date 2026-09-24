import { useCallback, useRef } from 'react'

export type CanvasLeaveGuard = () => Promise<boolean>

/** Shell navigation shares the editor's single unsaved-change confirmation. */
export function useCanvasLeaveGuard() {
  const guard = useRef<CanvasLeaveGuard | null>(null)
  const pending = useRef(false)
  const register = useCallback((next: CanvasLeaveGuard | null) => {
    guard.current = next
  }, [])
  const leave = useCallback(async (action: () => void) => {
    if (pending.current) return
    pending.current = true
    try {
      if (!guard.current || (await guard.current())) action()
    } finally {
      pending.current = false
    }
  }, [])
  return { register, leave }
}
