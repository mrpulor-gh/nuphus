import { describe, expect, it } from 'vitest'
import { isCustomProviderId, isValidCustomInstanceId } from './customProvider'

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

  it('treats legacy custom and custom-* as custom providers', () => {
    expect(isCustomProviderId('custom')).toBe(true)
    expect(isCustomProviderId('custom-team-a')).toBe(true)
    // 官方 Provider 不得被误判为自定义段
    expect(isCustomProviderId('deepseek')).toBe(false)
    expect(isCustomProviderId('opencode-go')).toBe(false)
    expect(isCustomProviderId('customized')).toBe(false)
  })
})
