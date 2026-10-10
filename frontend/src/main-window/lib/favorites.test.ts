/**
 * favorites — 模型「收藏」本地偏好的读写契约。
 *
 * 重点：坏数据一律安全降级为「无记录」；收藏只影响排序，不过滤可见范围
 * （不过滤的语义在 modelPopupOrder 的单测里钉死，这里只管读写）。
 */
import { describe, expect, it } from 'vitest'
import {
  FAVORITE_MODELS_KEY,
  getFavorites,
  getProviderFavorites,
  isFavorite,
  setFavorite,
} from './favorites'

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

describe('getFavorites — 坏数据安全降级', () => {
  it('无 key → 空表', () => {
    expect(getFavorites(fakeStorage())).toEqual({})
  })

  it('坏 JSON（截断）→ 空表，不抛', () => {
    expect(getFavorites(fakeStorage({ [FAVORITE_MODELS_KEY]: '{"p":["a"' }))).toEqual({})
  })

  it('顶层非对象（数组 / 数字 / 字符串 / null）→ 空表', () => {
    for (const bad of ['["a"]', '42', '"a"', 'null']) {
      expect(getFavorites(fakeStorage({ [FAVORITE_MODELS_KEY]: bad }))).toEqual({})
    }
  })

  it('某 provider 的值非数组 → 该项降级为无记录，不污染成空数组', () => {
    const s = fakeStorage({ [FAVORITE_MODELS_KEY]: '{"p":"model-x","q":["keep"]}' })
    expect(getFavorites(s)).toEqual({ q: ['keep'] })
    expect(getProviderFavorites(s, 'p')).toBeNull()
  })

  it('数组混入非字符串 → 过滤后保留合法项', () => {
    const s = fakeStorage({ [FAVORITE_MODELS_KEY]: '{"p":["a",1,null,["x"],"b"]}' })
    expect(getFavorites(s)).toEqual({ p: ['a', 'b'] })
  })

  it('空数组值 = 无记录（不写进结果）', () => {
    const s = fakeStorage({ [FAVORITE_MODELS_KEY]: '{"p":[]}' })
    expect(getFavorites(s)).toEqual({})
    expect(getProviderFavorites(s, 'p')).toBeNull()
  })

  it('storage 不可用（undefined）→ 空表，不抛', () => {
    expect(getFavorites(undefined)).toEqual({})
  })
})

describe('setFavorite / isFavorite', () => {
  it('收藏 → 写进该 provider 清单，isFavorite 命中', () => {
    const s = fakeStorage()
    setFavorite(s, 'p', 'm1', true)
    expect(getProviderFavorites(s, 'p')).toEqual(['m1'])
    expect(isFavorite(s, 'p', 'm1')).toBe(true)
    expect(isFavorite(s, 'p', 'm2')).toBe(false)
  })

  it('重复收藏同一模型 → 去重，不产生重复项', () => {
    const s = fakeStorage()
    setFavorite(s, 'p', 'm1', true)
    setFavorite(s, 'p', 'm1', true)
    expect(getProviderFavorites(s, 'p')).toEqual(['m1'])
  })

  it('取消其中一个收藏 → 其余保留', () => {
    const s = fakeStorage()
    setFavorite(s, 'p', 'm1', true)
    setFavorite(s, 'p', 'm2', true)
    setFavorite(s, 'p', 'm1', false)
    expect(getProviderFavorites(s, 'p')).toEqual(['m2'])
  })

  it('取消最后一个收藏 → 删除 key（语义回到「无收藏」）', () => {
    const s = fakeStorage()
    setFavorite(s, 'p', 'm1', true)
    setFavorite(s, 'p', 'm1', false)
    expect(s.getItem(FAVORITE_MODELS_KEY)).toBeNull()
    expect(getProviderFavorites(s, 'p')).toBeNull()
  })

  it('provider 之间互相隔离（A 的收藏不影响 B）', () => {
    const s = fakeStorage()
    setFavorite(s, 'a', 'm1', true)
    expect(isFavorite(s, 'a', 'm1')).toBe(true)
    expect(getProviderFavorites(s, 'b')).toBeNull()
    expect(isFavorite(s, 'b', 'm1')).toBe(false)
  })

  it('所有 provider 都清空 → key 删除（不留空对象）', () => {
    const s = fakeStorage()
    setFavorite(s, 'a', 'm1', true)
    setFavorite(s, 'b', 'm2', true)
    setFavorite(s, 'a', 'm1', false)
    setFavorite(s, 'b', 'm2', false)
    expect(s.getItem(FAVORITE_MODELS_KEY)).toBeNull()
  })

  it('storage 不可用（undefined）→ 读写都不抛', () => {
    expect(isFavorite(undefined, 'p', 'm')).toBe(false)
    expect(() => setFavorite(undefined, 'p', 'm', true)).not.toThrow()
  })

  it('storage.setItem 抛异常（配额满）→ 不向外抛', () => {
    const throwing = {
      getItem: () => null,
      setItem: () => {
        throw new Error('QuotaExceededError')
      },
      removeItem: () => {},
    } as unknown as Storage
    expect(() => setFavorite(throwing, 'p', 'm1', true)).not.toThrow()
  })
})
