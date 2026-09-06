import { describe, expect, it } from 'vitest'

import { tidyFrame } from './tidy'
import {
  Frame,
  Group,
  Item,
  Kind,
  NAV_BAR_H,
  PHONE_H,
  PHONE_MARGIN,
  PHONE_W,
  makeItem,
} from './tokens'

const frame: Frame = { id: 'f1', name: 'Home', x: 0, y: 0 }
const frames = [frame]

const grp = (id: string, x: number, y: number, items: Item[]): Group => ({
  id,
  x,
  y,
  axis: 'x',
  items,
})
const part = (kind: Kind, id: string): Item => ({ ...makeItem(kind), id })

describe('tidyFrame', () => {
  it('snaps the app bar to the top edge and the navigation bar to the bottom', () => {
    const groups = [
      grp('g-bar', 40, 300, [part('topAppBar', 'bar')]),
      grp('g-nav', 40, 100, [part('bottomNav', 'nav')]),
    ]
    const out = tidyFrame(groups, frame, frames, {})
    const bar = out!.find(g => g.id === 'g-bar')!
    const nav = out!.find(g => g.id === 'g-nav')!
    expect([bar.x, bar.y]).toEqual([0, 0])
    expect([nav.x, nav.y]).toEqual([0, PHONE_H - (80 + NAV_BAR_H)])
  })

  it('moves a FAB to the bottom-right corner, one margin in', () => {
    const out = tidyFrame([grp('g-fab', 40, 100, [part('fab', 'fab')])], frame, frames, {})
    expect([out![0].x, out![0].y]).toEqual([
      PHONE_W - PHONE_MARGIN - 56,
      PHONE_H - PHONE_MARGIN - 56,
    ])
  })

  it('joins neighbouring buttons into one connected run in reading order', () => {
    const out = tidyFrame(
      [grp('g1', 16, 300, [part('button', 'b1')]), grp('g2', 150, 300, [part('button', 'b2')])],
      frame,
      frames,
      {},
    )
    expect(out).toHaveLength(1)
    expect(out![0].items.map(it => it.id)).toEqual(['b1', 'b2'])
  })

  it('joins buttons dropped onto each other, in reading order', () => {
    const out = tidyFrame(
      [grp('g1', 150, 300, [part('button', 'b1')]), grp('g2', 100, 304, [part('button', 'b2')])],
      frame,
      frames,
      {},
    )
    expect(out).toHaveLength(1)
    expect(out![0].items.map(it => it.id)).toEqual(['b2', 'b1'])
  })

  it('joins neighbouring list items into one connected column', () => {
    const out = tidyFrame(
      [grp('g1', 16, 100, [part('listItem', 'l1')]), grp('g2', 16, 180, [part('listItem', 'l2')])],
      frame,
      frames,
      {},
    )
    expect(out).toHaveLength(1)
    expect(out![0].axis).toBe('y')
    expect(out![0].items.map(it => it.id)).toEqual(['l1', 'l2'])
  })

  it('returns null for a frame that is already tidy', () => {
    const messy = [
      grp('g-bar', 40, 300, [part('topAppBar', 'bar')]),
      grp('g-fab', 10, 10, [part('fab', 'fab')]),
    ]
    const once = tidyFrame(messy, frame, frames, {})
    expect(once).not.toBeNull()
    expect(tidyFrame(once!, frame, frames, {})).toBeNull()
  })
})

describe('tidyFrame placement', () => {
  /* two rows of buttons under a top app bar, above a navigation bar */
  const stage = () => [
    grp('g-bar', 40, 300, [part('topAppBar', 'bar')]),
    grp('g-nav', 40, 100, [part('bottomNav', 'nav')]),
    grp('g-a', 60, 500, [part('button', 'a')]),
    grp('g-b', 60, 620, [part('button', 'b')]),
  ]
  const rowsOf = (out: Group[]) =>
    out
      .filter(g => g.id === 'g-a' || g.id === 'g-b')
      .map(g => g.y)
      .sort((a, b) => a - b)
  const bodyTop = 64 + 24 + PHONE_MARGIN // app bar with its status inset, then the margin
  const bodyBottom = PHONE_H - (80 + NAV_BAR_H) - PHONE_MARGIN

  it('stacks the body from the top by default', () => {
    const out = tidyFrame(stage(), frame, frames, {})!
    expect(rowsOf(out)[0]).toBe(bodyTop)
  })

  it('pushes the body against the bottom bar when the screen asks for it', () => {
    const f: Frame = { ...frame, place: 'bottom' }
    const out = tidyFrame(stage(), f, [f], {})!
    const ys = rowsOf(out)
    expect(ys[1] + 56).toBe(bodyBottom)
    expect(ys[1] - ys[0]).toBe(56 + 16) // rows keep their gap
  })

  it('centers the body between the bars', () => {
    const f: Frame = { ...frame, place: 'center' }
    const out = tidyFrame(stage(), f, [f], {})!
    const ys = rowsOf(out)
    const above = ys[0] - bodyTop
    const below = bodyBottom - (ys[1] + 56)
    expect(Math.abs(above - below)).toBeLessThanOrEqual(1)
  })

  it('centers a single row when asked to spread', () => {
    const f: Frame = { ...frame, place: 'spread' }
    const one = stage().filter(g => g.id !== 'g-b')
    const out = tidyFrame(one, f, [f], {})!
    const y = out.find(g => g.id === 'g-a')!.y
    expect(Math.abs(y - bodyTop - (bodyBottom - (y + 56)))).toBeLessThanOrEqual(1)
  })

  it('keeps the block at the top when some rows do not fit', () => {
    const f: Frame = { ...frame, place: 'bottom' }
    /* two buttons, then a 600dp box that no longer fits under them: the buttons stack from
     * the top as before and the box stays where it was */
    const tall = [
      grp('g-bar', 40, 300, [part('topAppBar', 'bar')]),
      grp('g-nav', 40, 100, [part('bottomNav', 'nav')]),
      grp('g-a', 60, 400, [part('button', 'a')]),
      grp('g-b', 60, 470, [part('button', 'b')]),
      grp('g-box', 0, 560, [{ ...part('box', 'box'), size2: 600 }]),
    ]
    const out = tidyFrame(tall, f, [f], {})!
    expect(rowsOf(out)).toEqual([bodyTop, bodyTop + 56 + 16])
    expect(out.find(g => g.id === 'g-box')!.y).toBe(560)
  })

  it('spreads the rows with equal gaps above, between and below', () => {
    const f: Frame = { ...frame, place: 'spread' }
    const out = tidyFrame(stage(), f, [f], {})!
    const ys = rowsOf(out)
    const above = ys[0] - bodyTop
    const between = ys[1] - (ys[0] + 56)
    const below = bodyBottom - (ys[1] + 56)
    expect(Math.abs(above - between)).toBeLessThanOrEqual(1)
    expect(Math.abs(between - below)).toBeLessThanOrEqual(1)
  })
})
