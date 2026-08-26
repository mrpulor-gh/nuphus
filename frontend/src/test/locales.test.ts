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

  it('键集合非空且达到基线规模（防整表误删/加载失败）', () => {
    // 不快照精确键数（历史上 987→994→1070→1087 每次增删键都要手改，纯负担且易漏）；
    // 只设保守下限：跌破它必然是误删或 import 失败，而非正常迭代
    expect(Object.keys(zh).length).toBeGreaterThan(500)
    expect(Object.keys(en).length).toBe(Object.keys(zh).length)
  })
})
