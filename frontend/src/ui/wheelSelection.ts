import { useCallback, useRef, type WheelEvent } from 'react'

const TRACKPAD_STEP_PX = 40
const STEP_THROTTLE_MS = 45
const DISCRETE_WHEEL_PX = 80

export interface WheelSelectionAccumulator {
  push(deltaY: number, deltaMode?: number, now?: number): number
  reset(): void
}

/** Converts noisy trackpad deltas into bounded option moves without changing native scroll elsewhere. */
export function createWheelSelectionAccumulator(): WheelSelectionAccumulator {
  let accumulated = 0
  let direction = 0
  let lastStepAt = Number.NEGATIVE_INFINITY

  return {
    push(deltaY, deltaMode = 0, now = performance.now()) {
      const pixels = deltaY * (deltaMode === 1 ? 16 : deltaMode === 2 ? 400 : 1)
      if (!pixels) return 0
      const nextDirection = Math.sign(pixels)
      if (nextDirection !== direction) {
        accumulated = 0
        direction = nextDirection
      }

      if (Math.abs(pixels) >= DISCRETE_WHEEL_PX) {
        accumulated = 0
        if (now - lastStepAt < STEP_THROTTLE_MS) return 0
        lastStepAt = now
        return nextDirection
      }

      accumulated += pixels
      if (Math.abs(accumulated) < TRACKPAD_STEP_PX || now - lastStepAt < STEP_THROTTLE_MS) {
        return 0
      }
      const steps = Math.min(2, Math.floor(Math.abs(accumulated) / TRACKPAD_STEP_PX))
      accumulated -= nextDirection * steps * TRACKPAD_STEP_PX
      lastStepAt = now
      return nextDirection * steps
    },
    reset() {
      accumulated = 0
      direction = 0
      lastStepAt = Number.NEGATIVE_INFINITY
    },
  }
}

export function useWheelSelection(onMove: (steps: number) => void) {
  const onMoveRef = useRef(onMove)
  onMoveRef.current = onMove
  const accumulatorRef = useRef<WheelSelectionAccumulator | null>(null)
  if (!accumulatorRef.current) accumulatorRef.current = createWheelSelectionAccumulator()

  return useCallback((event: WheelEvent<HTMLElement>) => {
    event.preventDefault()
    const steps = accumulatorRef.current?.push(event.deltaY, event.deltaMode) ?? 0
    if (steps) onMoveRef.current(steps)
  }, [])
}
