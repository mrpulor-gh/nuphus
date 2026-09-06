import { describe, expect, it, vi } from 'vitest'
import { isProject, projectFileName, readProject } from './project'
import { KIND_ORDER, VARIANTS, type Doc, type Item } from './tokens'

const item = (): Item => ({
  id: 'item',
  kind: 'button',
  label: 'Save',
  icon: null,
  variant: 'filled',
})
const doc = (): Doc => ({
  title: 'Sketch',
  brief: 'A small app',
  paletteKey: 'purple',
  frame: 'phone',
  groups: [{ id: 'group', x: -12.5, y: 0, axis: 'x', items: [item()] }],
  frames: [{ id: 'frame', name: 'Home', x: 0, y: -10 }],
})
const withItem = (patch: Record<string, unknown>) => {
  const value = doc()
  return { ...value, groups: [{ ...value.groups[0], items: [{ ...item(), ...patch }] }] }
}

describe('isProject', () => {
  it('accepts a current document without changing it', () => {
    const value = doc()
    const before = structuredClone(value)
    expect(isProject(value)).toBe(true)
    expect(value).toEqual(before)
  })

  it('accepts legacy minimal documents without supplying defaults', () => {
    const legacy = { groups: [], frames: [{ id: 'f', name: 'Home', x: 0, y: 0 }] }
    expect(isProject(legacy)).toBe(true)
    expect(legacy).not.toHaveProperty('platform')
    expect(legacy.frames[0]).not.toHaveProperty('w')
  })

  it.each([undefined, 'android', 'web'])('accepts platform %s', platform => {
    expect(isProject({ ...doc(), platform })).toBe(true)
  })

  it.each([null, 'ios', '', 0, {}, true])('rejects platform %j', platform => {
    expect(isProject({ ...doc(), platform })).toBe(false)
  })

  it.each(
    [
      null,
      undefined,
      true,
      42,
      '{}',
      [],
      {},
      { groups: [] },
      { frames: [] },
      { groups: null, frames: [] },
      { groups: {}, frames: [] },
      { groups: [], frames: {} },
    ].map(value => [value]),
  )('rejects invalid document shape %# %o', value => {
    expect(isProject(value)).toBe(false)
  })

  it.each(KIND_ORDER)('accepts registered kind %s', kind => {
    expect(isProject(withItem({ kind }))).toBe(true)
  })

  it.each(VARIANTS)('accepts registered variant $key', ({ key }) => {
    expect(isProject(withItem({ variant: key }))).toBe(true)
  })

  it.each([
    { id: 1 },
    { x: NaN },
    { x: Infinity },
    { x: '0' },
    { y: -Infinity },
    { y: null },
    { axis: 'z' },
    { axis: undefined },
    { items: [] },
    { items: null },
    { items: {} },
    { items: [null] },
  ])('rejects invalid group fields %# %o', patch => {
    const value = doc()
    expect(isProject({ ...value, groups: [{ ...value.groups[0], ...patch }] })).toBe(false)
  })

  it('checks every group, frame and item, not only the first', () => {
    const value = doc()
    expect(isProject({ ...value, groups: [...value.groups, null] })).toBe(false)
    expect(isProject({ ...value, frames: [...value.frames, null] })).toBe(false)
    expect(isProject({ ...value, groups: [{ ...value.groups[0], items: [item(), null] }] })).toBe(
      false,
    )
  })

  it('accepts vertical groups and optional item fields including empty strings', () => {
    const value = withItem({
      icon: 'save',
      supporting: '',
      note: '',
      selected: 0,
      corners: { tl: 0, tr: 4, bl: 8, br: 12 },
      tabs: [{ label: 'One', icon: 'home' }, { label: 'Two', icon: null }, { label: '' }],
    })
    value.groups[0].axis = 'y'
    expect(isProject(value)).toBe(true)
    expect(isProject(withItem({ tabs: [] }))).toBe(true)
  })

  it.each([
    { id: undefined },
    { id: 1 },
    { kind: 'unknown' },
    { kind: null },
    { label: 1 },
    { icon: undefined },
    { icon: 1 },
    { variant: 'unknown' },
    { variant: undefined },
    { supporting: null },
    { note: 1 },
    { selected: NaN },
    { selected: Infinity },
    { selected: '0' },
    { corners: null },
    { corners: {} },
    { corners: { tl: 0, tr: 0, bl: 0 } },
    { corners: { tl: '0', tr: 0, bl: 0, br: 0 } },
    { corners: { tl: 0, tr: 0, bl: 0, br: Infinity } },
    { tabs: null },
    { tabs: {} },
    { tabs: [null] },
    { tabs: [{ icon: 'home' }] },
    { tabs: [{ label: 1 }] },
    { tabs: [{ label: 'Home', icon: 1 }] },
  ])('rejects invalid item fields %# %o', patch => {
    expect(isProject(withItem(patch))).toBe(false)
  })

  it.each([
    { id: null },
    { name: 1 },
    { x: Infinity },
    { x: '0' },
    { y: NaN },
    { w: 0 },
    { w: -1 },
    { w: Infinity },
    { w: '100' },
    { w: null },
    { h: 0 },
    { h: -1 },
    { h: NaN },
    { h: '100' },
    { h: null },
    { note: false },
  ])('rejects invalid frame fields %# %o', patch => {
    const value = doc()
    expect(isProject({ ...value, frames: [{ ...value.frames[0], ...patch }] })).toBe(false)
  })

  it.each(['top', 'center', 'bottom', 'spread'])('accepts the body placement %s', place => {
    const value = doc()
    expect(isProject({ ...value, frames: [{ ...value.frames[0], place }] })).toBe(true)
  })

  it.each(['middle', '', 0, null])('rejects an unknown body placement %j', place => {
    const value = doc()
    expect(isProject({ ...value, frames: [{ ...value.frames[0], place }] })).toBe(false)
  })

  it('accepts positive fractional frame dimensions and an empty note', () => {
    const value = doc()
    expect(
      isProject({ ...value, frames: [{ ...value.frames[0], w: 0.5, h: 800, note: '' }] }),
    ).toBe(true)
  })
})

describe('projectFileName', () => {
  it.each([
    ['', 'ui-proto-canvas.json'],
    [' \t\n ', 'ui-proto-canvas.json'],
    ['\\/:*?"<>|', 'ui-proto-canvas.json'],
    ['  My\t app\n name  ', 'ui-proto-canvas My app name.json'],
    ['a\\b/c:d*e?f"g<h>i|j', 'ui-proto-canvas a b c d e f g h i j.json'],
    ['設計 한국어 🎨.v2', 'ui-proto-canvas 設計 한국어 🎨.v2.json'],
  ])('sanitizes %j to %j', (title, expected) => {
    expect(projectFileName({ ...doc(), title })).toBe(expected)
  })
})

describe('readProject', () => {
  it('reads a real File as JSON without relying on its name or MIME type', async () => {
    const value = doc()
    await expect(
      readProject(new File([JSON.stringify(value)], 'sketch.txt', { type: 'text/plain' })),
    ).resolves.toEqual(value)
  })

  it("preserves a legacy file and unknown fields for the editor's migration step", async () => {
    const legacy = { groups: [], frames: [], futureField: { keep: true } }
    await expect(readProject(new File([JSON.stringify(legacy)], 'old.json'))).resolves.toEqual(
      legacy,
    )
  })

  it.each(['', '{', 'null', '[]', '{"groups":[],"frames":{}}'])(
    'returns null for invalid file contents %j',
    async text => {
      await expect(readProject(new File([text], 'bad.json'))).resolves.toBeNull()
    },
  )

  it('returns null for valid JSON with an invalid nested item', async () => {
    await expect(
      readProject(new File([JSON.stringify(withItem({ kind: 'unknown' }))], 'bad.json')),
    ).resolves.toBeNull()
  })

  it('returns null when reading the File fails', async () => {
    const file = new File([], 'unreadable.json')
    const read = vi.spyOn(file, 'text').mockRejectedValue(new Error('Read failed'))
    try {
      await expect(readProject(file)).resolves.toBeNull()
    } finally {
      read.mockRestore()
    }
  })
})
