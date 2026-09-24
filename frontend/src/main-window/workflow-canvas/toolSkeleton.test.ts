import { describe, expect, it } from 'vitest'
import { skeletonFromSchema } from './toolSkeleton'

describe('new tool parameters', () => {
  it('does not invent zero, false, empty strings or the first enum as defaults', () => {
    expect(
      skeletonFromSchema({
        type: 'object',
        required: ['count', 'enabled', 'mode', 'text'],
        properties: {
          count: { type: 'number' },
          enabled: { type: 'boolean' },
          mode: { enum: ['one', 'two'] },
          text: { type: 'string' },
        },
      }),
    ).toEqual({})
  })
  it('preserves and clones only explicitly declared defaults', () => {
    const schema = {
      properties: {
        count: { default: 0 },
        enabled: { default: false },
        text: { default: '' },
        nil: { default: null },
        list: { default: [1] },
      },
    }
    const result = skeletonFromSchema(schema)
    expect(result).toEqual({ count: 0, enabled: false, text: '', nil: null, list: [1] })
    expect(result.list).not.toBe(schema.properties.list.default)
  })
})
