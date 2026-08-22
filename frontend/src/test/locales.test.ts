import { describe, expect, it } from 'vitest'
import zh from '../locales/zh'
import en from '../locales/en'

/**
 * 中英 locales 键一致性回归测试：
 * 历史上曾出现键漂移（一侧新增/删除未同步），导致 UI 显示原始 key。
 * 此测试保证 zh/en 键集合完全相等——新增键必须双语同步。
 */
describe('locales 中英键一致性', () => {
  it('zh/en 键集合相等（零漂移）', () => {
    const zhKeys = Object.keys(zh).sort()
    const enKeys = Object.keys(en).sort()

    const zhOnly = zhKeys.filter(k => !(k in en))
    const enOnly = enKeys.filter(k => !(k in zh))

    expect(zhOnly).toEqual([])
    expect(enOnly).toEqual([])
    expect(zhKeys.length).toBe(enKeys.length)
  })

  it('键数量记录（改动时此处数字应同步更新）', () => {
    expect(Object.keys(zh).length).toBe(994)
    expect(Object.keys(en).length).toBe(994)
  })
})
