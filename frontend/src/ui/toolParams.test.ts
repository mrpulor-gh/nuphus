import { describe, it, expect } from 'vitest'
import { formatToolParams } from './toolParams'

describe('formatToolParams（执行过程工具参数摘要）', () => {
  it('无参数 / 非对象按原样降级', () => {
    expect(formatToolParams(undefined)).toBeNull()
    expect(formatToolParams(null)).toBeNull()
    expect(formatToolParams('ls -la')).toBe('ls -la')
    expect(formatToolParams(42)).toBe('42')
  })

  it('按定位字段优先级拼接：command / cwd 先于 path / query', () => {
    expect(formatToolParams({ query: 'x', path: '/a.ts', command: 'rg x', cwd: '/repo' })).toBe(
      'command: rg x  cwd: /repo  path: /a.ts  query: x',
    )
  })

  it('字段值为空串 / null 时跳过', () => {
    expect(formatToolParams({ command: '', cwd: null, path: '/a.ts' })).toBe('path: /a.ts')
  })

  it('无常用字段：退回精简 JSON，超长截断', () => {
    expect(formatToolParams({ patterns: ['*.rs'] })).toBe('{"patterns":["*.rs"]}')
    const long = formatToolParams({ blob: 'x'.repeat(300) })
    expect(long).not.toBeNull()
    expect(long?.length).toBe(158)
    expect(long?.endsWith('…')).toBe(true)
  })

  it('循环引用无法序列化时返回 null', () => {
    const circular: Record<string, unknown> = {}
    circular.self = circular
    expect(formatToolParams(circular)).toBeNull()
  })
})
