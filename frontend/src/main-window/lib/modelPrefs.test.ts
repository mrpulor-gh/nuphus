/**
 * modelPrefs — 模型界面纯前端偏好（精选 / 厂商默认）的读写契约。
 *
 * 重点是「坏数据如何降级」与「空集语义」：
 * - 坏 JSON / 非对象 / 值非数组 一律降级为「无记录」，不抛错、不用空数组污染；
 * - 移出最后一个精选 ⇒ 删除 key，语义回到「全显示」（不是留下空数组 → 弹窗空白）；
 * - provider 之间互相隔离（A 的精选不影响 B）。
 * 另：所有写入只碰 storage，不产生任何副作用（本模块不引入 IPC）。
 */
import { describe, expect, it } from 'vitest'
import {
  DEFAULT_MODEL_KEY_PREFIX,
  VISIBLE_MODELS_KEY,
  getDefaultModel,
  getPickedModels,
  getVisibleModels,
  isModelVisible,
  setDefaultModel,
  setModelVisible,
} from './modelPrefs'

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

describe('getVisibleModels — 坏数据安全降级为「无记录」', () => {
  it('无 key → 空表', () => {
    expect(getVisibleModels(fakeStorage())).toEqual({})
  })

  it('坏 JSON（截断）→ 空表，不抛', () => {
    expect(getVisibleModels(fakeStorage({ [VISIBLE_MODELS_KEY]: '{"p":["a"' }))).toEqual({})
  })

  it('顶层非对象（数组 / 数字 / 字符串 / null）→ 空表', () => {
    for (const bad of ['["a"]', '42', '"a"', 'null']) {
      expect(getVisibleModels(fakeStorage({ [VISIBLE_MODELS_KEY]: bad }))).toEqual({})
    }
  })

  it('某 provider 的值非数组 → 该项降级为无记录，不污染成空数组', () => {
    const s = fakeStorage({ [VISIBLE_MODELS_KEY]: '{"p":"model-x","q":["keep"]}' })
    expect(getVisibleModels(s)).toEqual({ q: ['keep'] })
    expect(getPickedModels(s, 'p')).toBeNull()
  })

  it('数组混入非字符串 → 过滤后保留合法项', () => {
    const s = fakeStorage({ [VISIBLE_MODELS_KEY]: '{"p":["a",1,null,["x"],"b"]}' })
    expect(getVisibleModels(s)).toEqual({ p: ['a', 'b'] })
  })

  it('空数组值 = 无记录（不写进结果）', () => {
    const s = fakeStorage({ [VISIBLE_MODELS_KEY]: '{"p":[]}' })
    expect(getVisibleModels(s)).toEqual({})
    expect(getPickedModels(s, 'p')).toBeNull()
  })

  it('storage 不可用（undefined）→ 空表，不抛', () => {
    expect(getVisibleModels(undefined)).toEqual({})
  })
})

describe('getPickedModels / isModelVisible', () => {
  it('无记录 → null / false（弹窗此时显示全部）', () => {
    const s = fakeStorage()
    expect(getPickedModels(s, 'p')).toBeNull()
    expect(isModelVisible(s, 'p', 'm')).toBe(false)
  })

  it('有精选 → 命中 true，未命中 false', () => {
    const s = fakeStorage({ [VISIBLE_MODELS_KEY]: '{"p":["m1","m2"]}' })
    expect(getPickedModels(s, 'p')).toEqual(['m1', 'm2'])
    expect(isModelVisible(s, 'p', 'm1')).toBe(true)
    expect(isModelVisible(s, 'p', 'm3')).toBe(false)
  })

  it('provider 之间互相隔离', () => {
    const s = fakeStorage()
    setModelVisible(s, 'a', 'm1', true)
    expect(isModelVisible(s, 'a', 'm1')).toBe(true)
    expect(getPickedModels(s, 'b')).toBeNull()
    expect(isModelVisible(s, 'b', 'm1')).toBe(false)
  })
})

describe('setModelVisible', () => {
  it('加入 → 写进该 provider 的精选清单', () => {
    const s = fakeStorage()
    setModelVisible(s, 'p', 'm1', true)
    expect(getPickedModels(s, 'p')).toEqual(['m1'])
  })

  it('重复加入同一模型 → 去重，不产生重复项', () => {
    const s = fakeStorage()
    setModelVisible(s, 'p', 'm1', true)
    setModelVisible(s, 'p', 'm1', true)
    expect(getPickedModels(s, 'p')).toEqual(['m1'])
  })

  it('移除其中一个 → 其余保留', () => {
    const s = fakeStorage()
    setModelVisible(s, 'p', 'm1', true)
    setModelVisible(s, 'p', 'm2', true)
    setModelVisible(s, 'p', 'm1', false)
    expect(getPickedModels(s, 'p')).toEqual(['m2'])
  })

  it('移除最后一个精选 → 删除 key，语义回到「全显示」', () => {
    const s = fakeStorage()
    setModelVisible(s, 'p', 'm1', true)
    setModelVisible(s, 'p', 'm1', false)
    expect(s.getItem(VISIBLE_MODELS_KEY)).toBeNull()
    expect(getVisibleModels(s)).toEqual({})
    expect(getPickedModels(s, 'p')).toBeNull()
  })

  it('只移除某 provider 时，其它 provider 的精选不受影响', () => {
    const s = fakeStorage()
    setModelVisible(s, 'a', 'm1', true)
    setModelVisible(s, 'b', 'm2', true)
    setModelVisible(s, 'a', 'm1', false)
    expect(getPickedModels(s, 'a')).toBeNull()
    expect(getPickedModels(s, 'b')).toEqual(['m2'])
  })

  it('storage 不可用（undefined）→ 不抛', () => {
    expect(() => setModelVisible(undefined, 'p', 'm', true)).not.toThrow()
  })

  it('storage.setItem 抛异常（配额满）→ 不向外抛', () => {
    const throwing = {
      getItem: () => null,
      setItem: () => {
        throw new Error('QuotaExceededError')
      },
      removeItem: () => {},
    } as unknown as Storage
    expect(() => setModelVisible(throwing, 'p', 'm', true)).not.toThrow()
  })
})

describe('getDefaultModel / setDefaultModel', () => {
  it('未设置 → 空串', () => {
    expect(getDefaultModel(fakeStorage(), 'p')).toBe('')
  })

  it('写入后读出，且不同 provider 互相隔离', () => {
    const s = fakeStorage()
    setDefaultModel(s, 'a', 'ma')
    expect(getDefaultModel(s, 'a')).toBe('ma')
    expect(getDefaultModel(s, 'b')).toBe('')
    expect(s.getItem(DEFAULT_MODEL_KEY_PREFIX + 'a')).toBe('ma')
  })

  it('覆盖已有默认', () => {
    const s = fakeStorage()
    setDefaultModel(s, 'p', 'old')
    setDefaultModel(s, 'p', 'new')
    expect(getDefaultModel(s, 'p')).toBe('new')
  })

  it('传空串 = 取消默认（删除 key）', () => {
    const s = fakeStorage()
    setDefaultModel(s, 'p', 'm')
    setDefaultModel(s, 'p', '')
    expect(getDefaultModel(s, 'p')).toBe('')
    expect(s.getItem(DEFAULT_MODEL_KEY_PREFIX + 'p')).toBeNull()
  })

  it('storage 不可用（undefined）→ 读写都不抛', () => {
    expect(getDefaultModel(undefined, 'p')).toBe('')
    expect(() => setDefaultModel(undefined, 'p', 'm')).not.toThrow()
  })
})
