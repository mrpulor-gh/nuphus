import { describe, it, expect } from 'vitest'
import { formatTokens, formatExactTokens } from './tokenFormat'

describe('formatTokens（紧凑读数口径）', () => {
  it('M 档保留两位小数：1,048,576 不再被压成 1.0M', () => {
    // 回归背景：ctx 分母显示 1.0M，用户手算 319.1k/1.0M=31.9%，系统却报 30%
    expect(formatTokens(1_048_576)).toBe('1.05M')
    expect(formatTokens(2_097_152)).toBe('2.1M')
    expect(formatTokens(1_234_567)).toBe('1.23M')
  })

  it('整百万 / 整千去掉小数尾零', () => {
    expect(formatTokens(1_000_000)).toBe('1M')
    expect(formatTokens(2_000_000)).toBe('2M')
    expect(formatTokens(100_000)).toBe('100k')
    expect(formatTokens(128_000)).toBe('128k')
  })

  it('k 档保留两位小数并去尾零', () => {
    expect(formatTokens(319_100)).toBe('319.1k')
    expect(formatTokens(32_768)).toBe('32.77k')
    expect(formatTokens(1_500)).toBe('1.5k')
    expect(formatTokens(12_400)).toBe('12.4k')
  })

  it('小于 1k 原样取整', () => {
    expect(formatTokens(860)).toBe('860')
    expect(formatTokens(0)).toBe('0')
  })

  it('99.9999 万级提升到 M 档，不出现 1000k', () => {
    expect(formatTokens(999_999)).toBe('1M')
    expect(formatTokens(999_994)).toBe('999.99k')
  })

  it('非法值不产出 NaN 文案', () => {
    expect(formatTokens(Number.NaN)).toBe('0')
  })
})

describe('formatExactTokens（设置界面真实值口径）', () => {
  it('千分位输出真实 token 数，不做任何缩写', () => {
    const grouped = (1_048_576).toLocaleString()
    expect(formatExactTokens(1_048_576)).toBe(grouped)
    expect(formatExactTokens(1_048_576)).not.toContain('M')
    expect(formatExactTokens(1_048_576)).not.toContain('k')
  })

  it('小数入参取整后显示', () => {
    expect(formatExactTokens(1048.576)).toBe((1049).toLocaleString())
  })
})
