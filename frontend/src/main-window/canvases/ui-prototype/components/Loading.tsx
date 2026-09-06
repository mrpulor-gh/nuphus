import { useEffect, useRef } from 'react'
import { LoadingAnimator, morphedShape } from '../lib/shapes'

/* ---------- shared helpers ---------- */

/** Cubic bezier easing (same parametrisation as CSS / Compose CubicBezierEasing). */
function bezier(x1: number, y1: number, x2: number, y2: number) {
  const ax = 3 * x1 - 3 * x2 + 1
  const bx = 3 * x2 - 6 * x1
  const cx = 3 * x1
  const ay = 3 * y1 - 3 * y2 + 1
  const by = 3 * y2 - 6 * y1
  const cy = 3 * y1
  const sx = (t: number) => ((ax * t + bx) * t + cx) * t
  const sy = (t: number) => ((ay * t + by) * t + cy) * t
  return (x: number) => {
    if (x <= 0) return 0
    if (x >= 1) return 1
    let lo = 0
    let hi = 1
    let t = x
    for (let i = 0; i < 24; i++) {
      const v = sx(t)
      if (Math.abs(v - x) < 1e-4) break
      if (v < x) lo = t
      else hi = t
      t = (lo + hi) / 2
    }
    return sy(t)
  }
}

const seg = (t: number, from: number, to: number, ease: (x: number) => number) =>
  ease(Math.min(1, Math.max(0, (t - from) / (to - from))))

function useFrame(cb: (ts: number) => void) {
  const ref = useRef(cb)
  ref.current = cb
  useEffect(() => {
    let raf = 0
    const loop = (ts: number) => {
      ref.current(ts)
      raf = requestAnimationFrame(loop)
    }
    raf = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(raf)
  }, [])
}

/* ---------- loading indicator (shape morph) ---------- */

export function LoadingIndicator({
  size,
  color,
  contained,
  containerColor,
}: {
  size: number
  color: string
  contained?: boolean
  containerColor?: string
}) {
  const pathRef = useRef<SVGPathElement>(null)
  const anim = useRef<LoadingAnimator | null>(null)
  if (!anim.current) anim.current = new LoadingAnimator()

  const scale = (size * 0.79) / 2
  const c = size / 2

  useFrame(ts => {
    const a = anim.current!
    a.update(ts)
    const pts = morphedShape(a.morph)
    let d = ''
    for (let i = 0; i < pts.length; i++) {
      d +=
        (i === 0 ? 'M' : 'L') +
        (pts[i][0] * scale).toFixed(2) +
        ' ' +
        (pts[i][1] * scale).toFixed(2)
    }
    const el = pathRef.current
    if (el) {
      el.setAttribute('d', d + 'Z')
      el.setAttribute('transform', `translate(${c} ${c}) rotate(${a.rotation.toFixed(2)})`)
    }
  })

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} style={{ display: 'block' }}>
      {contained && <circle cx={c} cy={c} r={c} fill={containerColor ?? 'rgba(0,0,0,0.08)'} />}
      <path ref={pathRef} fill={color} />
    </svg>
  )
}

/* ---------- linear progress (flat / wavy) ---------- */

const LINEAR_CYCLE = 1800
const l1Head = bezier(0.2, 0, 0.8, 1)
const l1Tail = bezier(0.4, 0, 1, 1)
const l2Head = bezier(0, 0, 0.65, 1)
const l2Tail = bezier(0.1, 0, 0.45, 1)

const STROKE = 4
const TRACK_GAP = 4
const STOP_SIZE = 4
const LINEAR_WAVELENGTH = 40
const LINEAR_AMPLITUDE = 3
const WAVE_SPEED = 40 // px per second

function wavePath(x0: number, x1: number, mid: number, amp: number, phase: number, wl: number) {
  if (x1 - x0 < 0.5) return ''
  const step = amp > 0 ? 2 : Math.max(2, x1 - x0)
  let d = ''
  for (let x = x0; ; x += step) {
    const xx = Math.min(x, x1)
    const y = mid + amp * Math.sin((2 * Math.PI * xx) / wl + phase)
    d += (d ? 'L' : 'M') + xx.toFixed(2) + ' ' + y.toFixed(2)
    if (xx >= x1) break
  }
  return d
}

export function LinearProgress({
  width,
  color,
  trackColor,
  wavy,
  value,
}: {
  width: number
  color: string
  trackColor: string
  wavy?: boolean
  /** 0..1, undefined = indeterminate */
  value?: number
}) {
  const height = wavy ? STROKE + LINEAR_AMPLITUDE * 2 + 2 : STROKE + 2
  const mid = height / 2
  const inset = STROKE / 2
  const w = width - STROKE
  const activeRef = useRef<SVGPathElement>(null)
  const active2Ref = useRef<SVGPathElement>(null)
  const trackRef = useRef<SVGPathElement>(null)
  const start = useRef<number>(0)

  useFrame(ts => {
    if (!start.current) start.current = ts
    const ms = ts - start.current
    const phase = wavy ? (-(ms / 1000) * WAVE_SPEED * 2 * Math.PI) / LINEAR_WAVELENGTH : 0
    const ampBase = wavy ? LINEAR_AMPLITUDE : 0
    const a = activeRef.current
    const b = active2Ref.current
    const t = trackRef.current
    if (!a || !b || !t) return

    if (value !== undefined) {
      const v = Math.min(1, Math.max(0, value))
      const amp = v < 0.1 || v > 0.95 ? 0 : ampBase
      const end = inset + w * v
      a.setAttribute('d', wavePath(inset, end, mid, amp, phase, LINEAR_WAVELENGTH))
      b.setAttribute('d', '')
      const trackStart = Math.min(inset + w - STOP_SIZE, end + TRACK_GAP + STROKE / 2)
      t.setAttribute('d', wavePath(v <= 0 ? inset : trackStart, inset + w, mid, 0, 0, 1))
      return
    }

    const c = ms % LINEAR_CYCLE
    const h1 = seg(c, 0, 750, l1Head)
    const t1 = seg(c, 333, 1183, l1Tail)
    const h2 = seg(c, 1000, 1567, l2Head)
    const t2 = seg(c, 1267, 1800, l2Tail)
    const s1 = [inset + w * t1, inset + w * h1]
    const s2 = [inset + w * t2, inset + w * h2]
    a.setAttribute('d', wavePath(s1[0], s1[1], mid, ampBase, phase, LINEAR_WAVELENGTH))
    b.setAttribute('d', wavePath(s2[0], s2[1], mid, ampBase, phase, LINEAR_WAVELENGTH))

    const segs = [s1, s2].filter(s => s[1] - s[0] > 0.5).sort((p, q) => p[0] - q[0])
    let cursor = inset
    let d = ''
    for (const s of segs) {
      const to = s[0] - TRACK_GAP - STROKE / 2
      if (to - cursor > 0.5) d += wavePath(cursor, to, mid, 0, 0, 1)
      cursor = Math.max(cursor, s[1] + TRACK_GAP + STROKE / 2)
    }
    if (inset + w - cursor > 0.5) d += wavePath(cursor, inset + w, mid, 0, 0, 1)
    t.setAttribute('d', d)
  })

  const stroke = { fill: 'none', strokeWidth: STROKE, strokeLinecap: 'round' as const }
  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      style={{ display: 'block' }}
    >
      <path ref={trackRef} stroke={trackColor} {...stroke} />
      <path ref={activeRef} stroke={color} {...stroke} />
      <path ref={active2Ref} stroke={color} {...stroke} />
      {value !== undefined && (
        <circle cx={width - inset - STOP_SIZE / 2} cy={mid} r={STOP_SIZE / 2} fill={color} />
      )}
    </svg>
  )
}

/* ---------- circular progress (flat / wavy) ---------- */

const ROTATION_MS = 1332
const ROTATIONS_PER_CYCLE = 5
const BASE_ROTATION = 286
const JUMP_ROTATION = 290
const ROTATION_OFFSET = (BASE_ROTATION + JUMP_ROTATION) % 360
const circEase = bezier(0.4, 0, 0.2, 1)
const CIRC_WAVELENGTH = 20
const CIRC_AMPLITUDE = 1.5

function arcPath(
  cx: number,
  cy: number,
  r: number,
  a0: number,
  a1: number,
  amp: number,
  k: number,
  phase: number,
) {
  if (a1 - a0 < 0.05) return ''
  const step = amp > 0 ? 2 : 6
  let d = ''
  for (let a = a0; ; a += step) {
    const aa = Math.min(a, a1)
    const rad = (aa * Math.PI) / 180
    const rr = r + amp * Math.sin(k * rad + phase)
    d +=
      (d ? 'L' : 'M') +
      (cx + rr * Math.cos(rad)).toFixed(2) +
      ' ' +
      (cy + rr * Math.sin(rad)).toFixed(2)
    if (aa >= a1) break
  }
  return d
}

export function CircularProgress({
  size,
  color,
  trackColor,
  wavy,
  value,
}: {
  size: number
  color: string
  trackColor: string
  wavy?: boolean
  value?: number
}) {
  const amp = wavy ? CIRC_AMPLITUDE : 0
  const r = size / 2 - STROKE / 2 - amp
  const c = size / 2
  const k = Math.max(3, Math.round((2 * Math.PI * r) / CIRC_WAVELENGTH))
  const gapDeg = ((TRACK_GAP + STROKE) / (2 * Math.PI * r)) * 360
  const activeRef = useRef<SVGPathElement>(null)
  const trackRef = useRef<SVGPathElement>(null)
  const start = useRef(0)

  useFrame(ts => {
    if (!start.current) start.current = ts
    const ms = ts - start.current
    const phase = wavy ? -(ms / 1000) * 2 * Math.PI : 0
    const a = activeRef.current
    const t = trackRef.current
    if (!a || !t) return

    if (value !== undefined) {
      const v = Math.min(1, Math.max(0, value))
      const sweep = 360 * v
      const useAmp = v < 0.05 || v > 0.95 ? 0 : amp
      a.setAttribute('d', arcPath(c, c, r, -90, -90 + sweep, useAmp, k, phase))
      const trackFrom = -90 + sweep + (v > 0 ? gapDeg : 0)
      const trackTo = 270 - (v < 1 ? gapDeg : 0)
      t.setAttribute('d', v >= 1 ? '' : arcPath(c, c, r, trackFrom, trackTo, 0, k, 0))
      return
    }

    const cycle = ROTATION_MS * ROTATIONS_PER_CYCLE
    const inCycle = ms % cycle
    const rotIndex = Math.floor(inCycle / ROTATION_MS)
    const inRot = inCycle % ROTATION_MS
    const baseRotation = ((BASE_ROTATION * inCycle) / cycle) * ROTATIONS_PER_CYCLE
    const head = seg(inRot, 0, 666, circEase) * JUMP_ROTATION
    const tail = seg(inRot, 666, 1332, circEase) * JUMP_ROTATION
    const offset = (rotIndex * ROTATION_OFFSET) % 360
    const startAngle = tail + offset + baseRotation - 90
    const sweep = Math.max(0.1, head - tail)
    a.setAttribute('d', arcPath(c, c, r, startAngle, startAngle + sweep, amp, k, phase))
    const trackFrom = startAngle + sweep + gapDeg
    const trackTo = startAngle + 360 - gapDeg
    t.setAttribute(
      'd',
      trackTo - trackFrom > 1 ? arcPath(c, c, r, trackFrom, trackTo, 0, k, 0) : '',
    )
  })

  const stroke = { fill: 'none', strokeWidth: STROKE, strokeLinecap: 'round' as const }
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} style={{ display: 'block' }}>
      <path ref={trackRef} stroke={trackColor} {...stroke} />
      <path ref={activeRef} stroke={color} {...stroke} />
    </svg>
  )
}
