/**
 * 输入框 Models 弹窗 → 提供商子列表排序
 *
 * 铁律回归：本模块只改展示顺序，不改模型解析/切换/生效。测试因此只看顺序，
 * 并逐条钉住「没有历史时结果必须与原顺序逐项一致」这个稳定性推论。
 */
import { describe, expect, it } from 'vitest'
import {
  orderProviderModels,
  readRecentModels,
  rememberRecentModel,
  RECENT_MODELS_KEY_PREFIX,
  RECENT_MODELS_MAX,
  type OrderableModel,
} from './modelPopupOrder'

/** 最小 Storage 替身：实现完整接口，避免断言退化成对替身形状的断言。 */
function fakeStorage(seed: Record<string, string> = {}): Storage {
  const map = new Map(Object.entries(seed))
  return {
    get length() {
      return map.size
    },
    clear: () => map.clear(),
    getItem: (k: string) => (map.has(k) ? (map.get(k) as string) : null),
    key: (i: number) => Array.from(map.keys())[i] ?? null,
    removeItem: (k: string) => {
      map.delete(k)
    },
    setItem: (k: string, v: string) => {
      map.set(k, v)
    },
  }
}

const models = (...ids: string[]): OrderableModel[] => ids.map(id => ({ id }))

describe('orderProviderModels', () => {
  it('无历史记录 → 顺序与原列表逐项一致（不是「大致不变」）', () => {
    const input = models('b', 'a', 'c')
    expect(orderProviderModels(input, [])).toEqual(input)
  })

  it('历史里的模型按最近切换顺序前置', () => {
    const input = models('m3', 'm1', 'm2', 'm4')
    expect(orderProviderModels(input, ['m2', 'm4']).map(m => m.id)).toEqual([
      'm2',
      'm4',
      'm3',
      'm1',
    ])
  })

  it('没用过的模型保持原相对顺序，接在历史模型之后', () => {
    const input = models('x', 'c', 'y', 'a', 'z', 'b')
    expect(orderProviderModels(input, ['b', 'a']).map(m => m.id)).toEqual([
      'b',
      'a',
      'x',
      'c',
      'y',
      'z',
    ])
  })

  it('历史里指向已不存在的模型 → 无害忽略，不退化为乱序', () => {
    const input = models('a', 'b')
    expect(orderProviderModels(input, ['ghost', 'a']).map(m => m.id)).toEqual(['a', 'b'])
  })

  it('不修改入参数组（返回新数组）', () => {
    const input = models('a', 'b')
    const out = orderProviderModels(input, ['b'])
    expect(out).not.toBe(input)
    expect(input.map(m => m.id)).toEqual(['a', 'b'])
  })
})

// ── 收藏置顶 ────────────────────────────────────────────────────────────
// 收藏只改顺序、不过滤；与最近切换的关系：收藏恒定在最近之前。
describe('orderProviderModels — 收藏置顶', () => {
  it('收藏恒定置顶，优先于最近切换序', () => {
    const input = models('m1', 'm2', 'm3')
    expect(orderProviderModels(input, ['m3', 'm2'], ['m1']).map(m => m.id)).toEqual([
      'm1',
      'm3',
      'm2',
    ])
  })

  it('多个收藏之间保持入参相对顺序（不按收藏写入时间重排）', () => {
    const input = models('a', 'b', 'c', 'd')
    expect(orderProviderModels(input, [], ['d', 'b']).map(m => m.id)).toEqual(['b', 'd', 'a', 'c'])
  })

  it('收藏 + 最近同时存在：收藏在前，其后按最近序', () => {
    const input = models('m1', 'm2', 'm3', 'm4')
    expect(orderProviderModels(input, ['m4', 'm2'], ['m1']).map(m => m.id)).toEqual([
      'm1',
      'm4',
      'm2',
      'm3',
    ])
  })

  it('收藏指向已不存在的模型 → 无害忽略，不退化为乱序', () => {
    const input = models('a', 'b')
    expect(orderProviderModels(input, [], ['ghost']).map(m => m.id)).toEqual(['a', 'b'])
  })

  it('收藏为空数组 / null / undefined → 视作无收藏，行为与不传一致', () => {
    const input = models('x', 'a', 'y')
    expect(orderProviderModels(input, [], []).map(m => m.id)).toEqual(['x', 'a', 'y'])
    expect(orderProviderModels(input, [], null).map(m => m.id)).toEqual(['x', 'a', 'y'])
    expect(orderProviderModels(input, [], undefined).map(m => m.id)).toEqual(['x', 'a', 'y'])
  })

  it('只有收藏、无历史 → 收藏置顶，其余保持原序', () => {
    const input = models('a', 'b', 'c')
    expect(orderProviderModels(input, [], ['c']).map(m => m.id)).toEqual(['c', 'a', 'b'])
  })

  it('收藏与最近命中同一模型 → 不重复乱序（置顶即可）', () => {
    const input = models('a', 'b', 'c')
    expect(orderProviderModels(input, ['a'], ['a']).map(m => m.id)).toEqual(['a', 'b', 'c'])
  })
})

describe('readRecentModels', () => {
  it('无 key → 空序', () => {
    expect(readRecentModels(fakeStorage(), 'local')).toEqual([])
  })

  it('合法 JSON 数组 → 原样读出', () => {
    const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'local']: '["a","b"]' })
    expect(readRecentModels(s, 'local')).toEqual(['a', 'b'])
  })

  it('坏 JSON（截断的字符串）→ 空序，不抛', () => {
    const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'local']: '["a",' })
    expect(readRecentModels(s, 'local')).toEqual([])
  })

  it('合法 JSON 但不是数组（对象/数字/字符串/null）→ 空序', () => {
    for (const bad of ['{"a":1}', '42', '"a"', 'null']) {
      const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'local']: bad })
      expect(readRecentModels(s, 'local')).toEqual([])
    }
  })

  it('数组里混入非字符串 → 过滤掉，保留合法项', () => {
    const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'local']: '["a",1,null,["x"],"b"]' })
    expect(readRecentModels(s, 'local')).toEqual(['a', 'b'])
  })

  it('storage 不可用（undefined）→ 空序，不抛', () => {
    expect(readRecentModels(undefined, 'local')).toEqual([])
  })
})

describe('rememberRecentModel', () => {
  it('新切换的模型置顶', () => {
    const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'p']: '["a","b"]' })
    rememberRecentModel(s, 'p', 'c')
    expect(readRecentModels(s, 'p')).toEqual(['c', 'a', 'b'])
  })

  it('重复切换同一模型 → 去重后仍置顶（不产生重复项）', () => {
    const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'p']: '["a","b"]' })
    rememberRecentModel(s, 'p', 'a')
    expect(readRecentModels(s, 'p')).toEqual(['a', 'b'])
  })

  it(`超过 ${RECENT_MODELS_MAX} 个 → 截断，队尾（最旧）被挤出`, () => {
    // seed[0] 是最新、seed[last] 是最旧——记住RecentModels 的位次语义
    const seed = Array.from({ length: RECENT_MODELS_MAX }, (_, i) => `m${i}`)
    const oldest = seed[seed.length - 1]
    const s = fakeStorage({ [RECENT_MODELS_KEY_PREFIX + 'p']: JSON.stringify(seed) })
    rememberRecentModel(s, 'p', 'new')
    const after = readRecentModels(s, 'p')
    expect(after).toHaveLength(RECENT_MODELS_MAX)
    expect(after[0]).toBe('new')
    expect(after).not.toContain(oldest) // 只有最旧那个被挤出，较新的仍在
    expect(after).toContain(seed[1]) // 次新的保留
  })

  it('storage.setItem 抛异常（配额满）→ 不向外抛', () => {
    const throwing = {
      getItem: () => null,
      setItem: () => {
        throw new Error('QuotaExceededError')
      },
    } as unknown as Storage
    expect(() => rememberRecentModel(throwing, 'p', 'a')).not.toThrow()
  })

  it('storage 不可用（undefined）→ 不抛', () => {
    expect(() => rememberRecentModel(undefined, 'p', 'a')).not.toThrow()
  })
})

describe('读写闭环（rememberRecentModel → readRecentModels → orderProviderModels）', () => {
  it('切换两次后，子列表把这两项按逆序提到最前', () => {
    const s = fakeStorage()
    rememberRecentModel(s, 'p', 'b')
    rememberRecentModel(s, 'p', 'd')
    const ordered = orderProviderModels(models('a', 'b', 'c', 'd'), readRecentModels(s, 'p'))
    expect(ordered.map(m => m.id)).toEqual(['d', 'b', 'a', 'c'])
  })
})
