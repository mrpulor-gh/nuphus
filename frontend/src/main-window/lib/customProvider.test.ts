import { describe, expect, it } from 'vitest'
import {
  buildCustomInstanceId,
  isCustomProviderId,
  isValidCustomInstanceId,
} from './customProvider'

describe('custom provider instance naming', () => {
  it('accepts stable custom-xxx instance ids', () => {
    expect(isValidCustomInstanceId('custom-a')).toBe(true)
    expect(isValidCustomInstanceId('custom-team-a')).toBe(true)
    expect(isValidCustomInstanceId('custom-gw2')).toBe(true)
  })

  it('rejects malformed instance ids', () => {
    // 与后端 validate_custom_provider_name 同规则：非法名必须在前端就被挡下，
    // 否则用户会拿着一个后端拒绝的段名去配置，直到保存才报错。
    expect(isValidCustomInstanceId('custom')).toBe(false)
    expect(isValidCustomInstanceId('custom-')).toBe(false)
    expect(isValidCustomInstanceId('Custom-A')).toBe(false)
    expect(isValidCustomInstanceId('custom-team_A')).toBe(false)
    expect(isValidCustomInstanceId('custom--a')).toBe(false)
    expect(isValidCustomInstanceId('custom-a-')).toBe(false)
    expect(isValidCustomInstanceId('deepseek')).toBe(false)
    expect(isValidCustomInstanceId(`custom-${'a'.repeat(64)}`)).toBe(false)
  })

  it('builds a stable ASCII segment id from the user-visible name', () => {
    // 用户填的名称可以任意（含中文），段名必须合法：后端校验不通过就落不了盘
    expect(buildCustomInstanceId('My Relay', [])).toBe('custom-my-relay')
    expect(buildCustomInstanceId('  GPT-Gateway 2  ', [])).toBe('custom-gpt-gateway-2')
    expect(buildCustomInstanceId('a/b:c', [])).toBe('custom-a-b-c')
    // 纯中文没有可用的 ASCII slug → 退化为时间戳 id（合法且唯一）
    const chinese = buildCustomInstanceId('我的中转站', [])
    expect(chinese).toMatch(/^custom-\d+$/)
    expect(isValidCustomInstanceId(chinese)).toBe(true)
    // 截断到后端 64 字符上限之内
    const long = buildCustomInstanceId('x'.repeat(200), [])
    expect(isValidCustomInstanceId(long)).toBe(true)
    expect(long.length).toBeLessThanOrEqual(64)
  })

  it('falls back to a unique id when the slug is already taken', () => {
    const taken = ['custom-my-relay']
    const id = buildCustomInstanceId('My Relay', taken)
    expect(taken.includes(id)).toBe(false)
    expect(isValidCustomInstanceId(id)).toBe(true)
    // 同一秒内连续两次同名称：加序号，绝不重名（重名会被后端拒绝）
    const first = buildCustomInstanceId('我的中转站', [])
    const second = buildCustomInstanceId('我的中转站', [first])
    expect(second).not.toBe(first)
    expect(isValidCustomInstanceId(second)).toBe(true)
  })

  it('treats legacy custom and custom-* as custom providers', () => {
    expect(isCustomProviderId('custom')).toBe(true)
    expect(isCustomProviderId('custom-team-a')).toBe(true)
    // 官方 Provider 不得被误判为自定义段
    expect(isCustomProviderId('deepseek')).toBe(false)
    expect(isCustomProviderId('opencode-go')).toBe(false)
    expect(isCustomProviderId('customized')).toBe(false)
  })
})
