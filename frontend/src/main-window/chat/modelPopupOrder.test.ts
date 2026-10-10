/**
 * 输入框 Models 弹窗 → 提供商子列表排序
 *
 * 铁律回归：本模块只改展示顺序，不改模型解析/切换/生效。测试因此只看顺序，
 * 并逐条钉住「没有历史时结果必须与原顺序逐项一致」这个稳定性推论。
 */
import { describe, expect, it } from 'vitest'
import {
  filterProviderModels,
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

  // ── 厂商默认（pinned）──────────────────────────────────────────────
  // 「设为默认」必须在弹窗里有真实作用点，否则它只是一行小字。

  it('pinned 恒定置顶，优先于最近切换序', () => {
    const input = models('m1', 'm2', 'm3')
    expect(orderProviderModels(input, ['m3', 'm2'], 'm1').map(m => m.id)).toEqual([
      'm1',
      'm3',
      'm2',
    ])
  })

  it('只有 pinned、无历史 → 仅把它提到首位，其余保持原序', () => {
    const input = models('a', 'b', 'c')
    expect(orderProviderModels(input, [], 'c').map(m => m.id)).toEqual(['c', 'a', 'b'])
  })

  it('pinned 不在候选集 → 无害忽略，不退化为乱序', () => {
    const input = models('a', 'b')
    expect(orderProviderModels(input, [], 'ghost').map(m => m.id)).toEqual(['a', 'b'])
    expect(orderProviderModels(input, ['b'], 'ghost').map(m => m.id)).toEqual(['b', 'a'])
  })

  it('pinned 不改入参', () => {
    const input = models('a', 'b')
    const out = orderProviderModels(input, [], 'b')
    expect(out).not.toBe(input)
    expect(input.map(m => m.id)).toEqual(['a', 'b'])
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

/**
 * 子列表可见范围（精选过滤）——与设置页「精选」清单配合的纯函数契约。
 * 关键语义：清单为空 / null ⇒ 全显示（不是全隐藏）；清单非空 ⇒ 只留命中项。
 */
describe('filterProviderModels', () => {
  it('清单为 null（无精选记录）→ 全显示，逐项与原顺序一致', () => {
    const input = models('a', 'b', 'c')
    expect(filterProviderModels(input, null)).toEqual(input)
  })

  it('清单为空数组 → 同样全显示（空集语义 = 全显示，不是全隐藏）', () => {
    const input = models('a', 'b')
    expect(filterProviderModels(input, [])).toEqual(input)
  })

  it('清单为 undefined / 未提供 → 全显示', () => {
    expect(filterProviderModels(models('a'), undefined).map(m => m.id)).toEqual(['a'])
  })

  it('非空清单 → 只保留清单命中的模型', () => {
    const input = models('a', 'b', 'c', 'd')
    expect(filterProviderModels(input, ['b', 'd']).map(m => m.id)).toEqual(['b', 'd'])
  })

  it('清单里有已不存在的模型 id → 无害忽略，不影响其余展示', () => {
    const input = models('a', 'b')
    expect(filterProviderModels(input, ['ghost', 'a']).map(m => m.id)).toEqual(['a'])
  })

  it('不修改入参数组（返回新数组）', () => {
    const input = models('a', 'b')
    const out = filterProviderModels(input, ['a'])
    expect(out).not.toBe(input)
    expect(input.map(m => m.id)).toEqual(['a', 'b'])
  })

  it('过滤 → 排序组合：命中项按最近切换顺序排列', () => {
    const input = models('a', 'b', 'c', 'd')
    const out = filterProviderModels(orderProviderModels(input, ['d', 'b']), ['a', 'b', 'd'])
    expect(out.map(m => m.id)).toEqual(['d', 'b', 'a'])
  })

  it('「移除最后一个精选」的闭环：清单回到 null ⇒ 全部重新可见', () => {
    // 模拟 modelPrefs 的语义入口：清单只剩一个时过滤为 1 项，清空后（null）回到全显示
    const input = models('a', 'b', 'c')
    expect(filterProviderModels(input, ['a']).map(m => m.id)).toEqual(['a'])
    expect(filterProviderModels(input, null)).toEqual(input)
  })
})
