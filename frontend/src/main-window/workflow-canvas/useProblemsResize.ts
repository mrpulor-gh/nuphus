import { useEffect, useRef, useState, type PointerEvent } from 'react'

export const PROBLEMS_HEIGHT_KEY = 'nuphus.workflow.problems.height'
const DEFAULT_HEIGHT = 220
export function useProblemsResize() {
  const panelRef = useRef<HTMLDivElement>(null)
  const drag = useRef<{ id: number; y: number; height: number } | null>(null)
  const [preferred, setPreferred] = useState(() => {
    try {
      const saved = Number(localStorage.getItem(PROBLEMS_HEIGHT_KEY))
      if (Number.isFinite(saved) && saved >= 120) return saved
    } catch {
      /* Storage may be unavailable. */
    }
    return DEFAULT_HEIGHT
  })
  const [maximum, setMaximum] = useState(Math.max(120, window.innerHeight - 260))
  const minimum = Math.min(120, maximum)
  const height = Math.max(minimum, Math.min(preferred, maximum))
  useEffect(() => {
    const panel = panelRef.current
    const parent = panel?.parentElement
    const canvas = parent?.querySelector('.wfc-canvas-wrap')
    const measure = () => {
      if (!parent || !canvas) return
      const available = parent.getBoundingClientRect().bottom - canvas.getBoundingClientRect().top
      if (available > 0) setMaximum(Math.max(36, available - 160))
    }
    measure()
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    if (parent) observer?.observe(parent)
    if (canvas) observer?.observe(canvas)
    window.addEventListener('resize', measure)
    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', measure)
    }
  }, [])
  const resize = (value: number) => {
    const next = Math.max(minimum, Math.min(value, maximum))
    setPreferred(next)
    try {
      localStorage.setItem(PROBLEMS_HEIGHT_KEY, String(next))
    } catch {
      /* Optional preference. */
    }
  }
  const finish = (event: PointerEvent<HTMLDivElement>) => {
    event.stopPropagation()
    if (drag.current?.id !== event.pointerId) return
    drag.current = null
    if (event.currentTarget.hasPointerCapture?.(event.pointerId))
      event.currentTarget.releasePointerCapture(event.pointerId)
  }
  return {
    panelRef,
    height,
    separatorProps: {
      role: 'separator',
      tabIndex: 0,
      'aria-orientation': 'horizontal' as const,
      'aria-valuemin': minimum,
      'aria-valuemax': maximum,
      'aria-valuenow': height,
      onDoubleClick: () => resize(DEFAULT_HEIGHT),
      onKeyDown: (event: React.KeyboardEvent<HTMLDivElement>) => {
        if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return
        event.preventDefault()
        event.stopPropagation()
        resize(
          event.key === 'Home'
            ? minimum
            : event.key === 'End'
              ? maximum
              : height + (event.key === 'ArrowUp' ? 20 : -20),
        )
      },
      onPointerDown: (event: PointerEvent<HTMLDivElement>) => {
        event.stopPropagation()
        if (event.button !== 0) return
        event.preventDefault()
        event.currentTarget.setPointerCapture(event.pointerId)
        drag.current = { id: event.pointerId, y: event.clientY, height }
      },
      onPointerMove: (event: PointerEvent<HTMLDivElement>) => {
        event.stopPropagation()
        if (drag.current?.id === event.pointerId)
          resize(drag.current.height + drag.current.y - event.clientY)
      },
      onPointerUp: finish,
      onPointerCancel: finish,
      onLostPointerCapture: () => {
        drag.current = null
      },
    },
  }
}
