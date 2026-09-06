import { describe, expect, it } from 'vitest'

import {
  DURATION_PER_SHAPE_MS,
  LoadingAnimator,
  SHAPE_COUNT,
  Spring,
  getShapes,
  morphedShape,
} from './shapes'

describe('getShapes', () => {
  it('samples every shape in the sequence with the same point count', () => {
    const shapes = getShapes()
    expect(shapes).toHaveLength(SHAPE_COUNT)
    const n = shapes[0].length
    expect(n).toBeGreaterThan(0)
    for (const s of shapes) expect(s).toHaveLength(n)
  })

  it('normalizes every shape into the unit box, up to resampling drift', () => {
    for (const shape of getShapes()) {
      let extreme = 0
      for (const [x, y] of shape) {
        expect(Math.abs(x)).toBeLessThanOrEqual(1 + 1e-9)
        expect(Math.abs(y)).toBeLessThanOrEqual(1 + 1e-9)
        extreme = Math.max(extreme, Math.abs(x), Math.abs(y))
      }
      /* resampling along the arc can step just past the exact extreme vertex */
      expect(extreme).toBeGreaterThan(0.99)
    }
  })

  it('caches the sampled shapes', () => {
    expect(getShapes()).toBe(getShapes())
  })
})

describe('morphedShape', () => {
  it('returns a shape exactly at every whole fraction', () => {
    const shapes = getShapes()
    for (let i = 0; i < SHAPE_COUNT; i++) expect(morphedShape(i)).toEqual(shapes[i])
  })

  it('interpolates halfway between neighbours at a half fraction', () => {
    const [a, b] = getShapes()
    const mid = morphedShape(0.5)
    expect(mid[0]).toEqual([(a[0][0] + b[0][0]) / 2, (a[0][1] + b[0][1]) / 2])
  })

  it('wraps around the sequence, past both ends', () => {
    expect(morphedShape(SHAPE_COUNT)).toEqual(getShapes()[0])
    const shapes = getShapes()
    const [last, first] = [shapes[SHAPE_COUNT - 1], shapes[0]]
    const mid = morphedShape(-0.5)
    expect(mid[0]).toEqual([(last[0][0] + first[0][0]) / 2, (last[0][1] + first[0][1]) / 2])
  })
})

describe('Spring', () => {
  it('settles at its target', () => {
    const s = new Spring(200, 0.6)
    s.target = 1
    for (let i = 0; i < 600; i++) s.step(1 / 60)
    expect(s.pos).toBeCloseTo(1, 2)
    expect(Math.abs(s.vel)).toBeLessThan(0.01)
  })

  it('overshoots when underdamped, not when critically damped', () => {
    const peakAfter = (damping: number) => {
      const s = new Spring(200, damping)
      s.target = 1
      let peak = 0
      for (let i = 0; i < 120; i++) {
        s.step(1 / 60)
        peak = Math.max(peak, s.pos)
      }
      return peak
    }
    expect(peakAfter(0.6)).toBeGreaterThan(1)
    expect(peakAfter(1)).toBeLessThanOrEqual(1.001)
  })
})

describe('LoadingAnimator', () => {
  it('advances the morph with time while keeping rotation inside one turn', () => {
    const a = new LoadingAnimator()
    for (let ts = 0; ts <= 3 * DURATION_PER_SHAPE_MS; ts += 50) {
      a.update(ts)
      expect(a.rotation).toBeGreaterThanOrEqual(0)
      expect(a.rotation).toBeLessThan(360)
    }
    expect(a.morph).toBeGreaterThan(0)
  })

  it('is deterministic for identical frame sequences', () => {
    const a = new LoadingAnimator()
    const b = new LoadingAnimator()
    for (let ts = 0; ts <= 2000; ts += 33) {
      a.update(ts)
      b.update(ts)
    }
    expect(a.morph).toBe(b.morph)
    expect(a.rotation).toBe(b.rotation)
  })
})
