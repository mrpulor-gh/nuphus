import { afterEach, describe, expect, it, vi } from 'vitest'
import { DOC_PARAM, DOCZ_PARAM, hasShareHash, readShareHash, shareable, shareLink } from './share'
import type { Doc, Item } from './tokens'

const doc = (): Doc => ({
  title: '設計 中文 한국어 🎨 + & # %',
  brief: 'First line\nSecond line',
  paletteKey: 'purple',
  frame: 'phone',
  platform: 'web',
  frames: [
    {
      id: 'f',
      name: 'Home',
      x: 0,
      y: 0,
      w: 1280,
      h: 800,
      note: 'Keep frame note',
      noteHistory: ['Private frame draft'],
    },
  ],
  groups: [
    {
      id: 'g',
      x: -10,
      y: 20,
      axis: 'x',
      items: [
        {
          id: 'i',
          kind: 'image',
          label: '画像',
          icon: null,
          variant: 'filled',
          src: 'data:image/png;base64,AAAA',
          note: 'Keep item note',
          noteHistory: ['Private item draft'],
        },
        {
          id: 'remote',
          kind: 'image',
          label: 'Public image',
          icon: null,
          variant: 'filled',
          src: 'https://example.test/image.png',
        },
      ],
    },
  ],
})
const plainHash = (value: unknown) => `#doc=${encodeURIComponent(JSON.stringify(value))}`
/* Independent raw-deflate helpers over the platform's Web Streams (not node:zlib),
 * keeping the frontend type environment free of Node globals. */
const toBase64Url = (bytes: Uint8Array) => {
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}
const fromBase64Url = (s: string) => {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (s.length % 4)) % 4)
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}
const deflateRaw = async (text: string) => {
  const stream = new CompressionStream('deflate-raw')
  const writer = stream.writable.getWriter()
  writer.write(new TextEncoder().encode(text))
  writer.close()
  return new Uint8Array(await new Response(stream.readable).arrayBuffer())
}
const inflateRaw = async (bytes: Uint8Array) => {
  const stream = new DecompressionStream('deflate-raw')
  const writer = stream.writable.getWriter()
  /* Copy onto a fresh ArrayBuffer: stream writers require BufferSource, and the
   * typed-array generic on the caller's side may be ArrayBufferLike. */
  writer.write(new Uint8Array(bytes))
  writer.close()
  return new TextDecoder().decode(await new Response(stream.readable).arrayBuffer())
}
const packedHash = async (text: string) => `#docz=${toBase64Url(await deflateRaw(text))}`

afterEach(() => vi.unstubAllGlobals())

describe('shareable', () => {
  it('removes every frame/item history and local image while keeping authored content', () => {
    const value = doc()
    value.frames.push({ ...value.frames[0], id: 'f2' })
    value.groups.push({ ...structuredClone(value.groups[0]), id: 'g2' })
    const result = shareable(value)
    expect(result).toEqual({
      ...value,
      frames: value.frames.map(({ noteHistory: _history, ...frame }) => frame),
      groups: value.groups.map(group => ({
        ...group,
        items: [
          {
            id: 'i',
            kind: 'image',
            label: '画像',
            icon: null,
            variant: 'filled',
            note: 'Keep item note',
          },
          {
            id: 'remote',
            kind: 'image',
            label: 'Public image',
            icon: null,
            variant: 'filled',
            src: 'https://example.test/image.png',
          },
        ],
      })),
    })
  })

  it.each(['http://example.test/a.png', 'https://example.test/a.png?x=1&y=2'])(
    'preserves public source %s',
    src => {
      const value = doc()
      value.groups[0].items[0].src = src
      expect(shareable(value).groups[0].items[0].src).toBe(src)
    },
  )

  it.each([
    undefined,
    '',
    'data:image/png;base64,AAAA',
    'blob:https://example.test/id',
    '/image.png',
    '//example.test/image.png',
    'file:///image.png',
  ])('omits non-public source %s', src => {
    const value = doc()
    value.groups[0].items[0].src = src
    expect(shareable(value).groups[0].items[0]).not.toHaveProperty('src')
  })

  it("does not mutate or reuse the edited document's frame/group/item objects", () => {
    const value = doc()
    const before = structuredClone(value)
    const result = shareable(value)
    expect(value).toEqual(before)
    expect(result).not.toBe(value)
    expect(result.frames).not.toBe(value.frames)
    expect(result.groups).not.toBe(value.groups)
    expect(result.frames[0]).not.toBe(value.frames[0])
    expect(result.groups[0]).not.toBe(value.groups[0])
    expect(result.groups[0].items).not.toBe(value.groups[0].items)
    result.groups[0].items.forEach((item: Item, index) =>
      expect(item).not.toBe(value.groups[0].items[index]),
    )
    expect(shareable(result)).toEqual(result)
  })

  it('supports an empty canvas', () => {
    const value = { ...doc(), groups: [], frames: [] }
    expect(shareable(value)).toEqual(value)
  })
})

describe('shareLink and readShareHash', () => {
  it('keeps the public wire parameter names stable', () => {
    expect(DOC_PARAM).toBe('doc')
    expect(DOCZ_PARAM).toBe('docz')
  })

  it('round-trips Unicode with native raw-deflate streams and unpadded base64url', async () => {
    const value = doc()
    const before = structuredClone(value)
    const base = 'https://example.test/canvas/?mode=edit'
    // No mock or conditional skip: supported Node must exercise real Web Streams.
    const link = await shareLink(value, base)
    expect(link).toMatch(/^https:\/\/example\.test\/canvas\/\?mode=edit#docz=[A-Za-z0-9_-]+$/)
    const hash = new URL(link).hash
    const text = await inflateRaw(fromBase64Url(hash.slice('#docz='.length)))
    expect(JSON.parse(text)).toEqual(shareable(value))
    await expect(readShareHash(hash)).resolves.toEqual(shareable(value))
    expect(value).toEqual(before)
  })

  it('round-trips percent-encoded JSON when CompressionStream is unavailable', async () => {
    vi.stubGlobal('CompressionStream', undefined)
    vi.stubGlobal('DecompressionStream', undefined)
    const value = doc()
    const base = 'https://example.test/subdir/?keep=1'
    const link = await shareLink(value, base)
    expect(link).toBe(`${base}#doc=${encodeURIComponent(JSON.stringify(shareable(value)))}`)
    await expect(readShareHash(new URL(link).hash)).resolves.toEqual(shareable(value))
  })

  it('reads independently generated raw-deflate payloads', async () => {
    await expect(readShareHash(await packedHash(JSON.stringify(doc())))).resolves.toEqual(doc())
  })

  it('reads plain JSON with or without a leading hash and alongside unrelated parameters', async () => {
    const hash = plainHash(doc())
    await expect(readShareHash(hash.slice(1))).resolves.toEqual(doc())
    await expect(readShareHash(`#other=1&${hash.slice(1)}&last=2`)).resolves.toEqual(doc())
  })

  it('accepts a legacy document without adding defaults', async () => {
    const legacy = { groups: [], frames: [] }
    await expect(readShareHash(plainHash(legacy))).resolves.toEqual(legacy)
  })

  it('prefers a nonempty compressed document to a plain document', async () => {
    const packed = { ...doc(), title: 'Compressed wins' }
    const packedTail = (await packedHash(JSON.stringify(packed))).slice(1)
    await expect(readShareHash(`${plainHash(doc())}&${packedTail}`)).resolves.toEqual(packed)
  })

  it('uses plain JSON when the compressed parameter is empty', async () => {
    await expect(readShareHash(`${plainHash(doc())}&docz=`)).resolves.toEqual(doc())
  })

  it('does not silently fall back to plain JSON after compressed corruption', async () => {
    await expect(readShareHash(`${plainHash(doc())}&docz=!!!`)).resolves.toBeNull()
  })

  it.each([
    '',
    '#',
    '#unrelated=1',
    '#doc=',
    '#docz=',
    '#doc=%7B',
    '#doc=%FF',
    '#docz=!!!',
    '#docz=A',
    '#docz=AAAA',
  ])('returns null for missing or broken payload %s', async hash => {
    await expect(readShareHash(hash)).resolves.toBeNull()
  })

  it.each(
    [
      null,
      [],
      {},
      { groups: [], frames: null },
      { ...doc(), platform: 'ios' },
      { ...doc(), groups: [{ ...doc().groups[0], items: [] }] },
    ].map(value => [value]),
  )('rejects non-project JSON in either encoding: %# %o', async value => {
    await expect(readShareHash(plainHash(value))).resolves.toBeNull()
    await expect(readShareHash(await packedHash(JSON.stringify(value)))).resolves.toBeNull()
  })

  it('returns null for valid deflate containing invalid JSON', async () => {
    await expect(readShareHash(await packedHash('not JSON'))).resolves.toBeNull()
  })

  it('returns null for a truncated compressed stream', async () => {
    const bytes = await deflateRaw(JSON.stringify(doc()))
    await expect(
      readShareHash(`#docz=${toBase64Url(bytes.subarray(0, bytes.length - 8))}`),
    ).resolves.toBeNull()
  })

  it('returns null for compressed input when decompression is unavailable', async () => {
    vi.stubGlobal('DecompressionStream', undefined)
    await expect(readShareHash(await packedHash(JSON.stringify(doc())))).resolves.toBeNull()
  })
})

describe('hasShareHash', () => {
  it.each(['#doc=', '#docz=', 'doc=x', 'docz=x', '#other=1&doc=bad', '#docz=!!!'])(
    'recognizes parameter presence without validating %s',
    hash => {
      expect(hasShareHash(hash)).toBe(true)
    },
  )

  it.each(['', '#', '#other=doc', '#document=x', '#DOC=x', '#other=docz%3Dx', '#mydoc=x'])(
    'does not mistake unrelated hash %s for a project',
    hash => {
      expect(hasShareHash(hash)).toBe(false)
    },
  )
})
