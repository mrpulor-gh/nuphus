import type { Condition, VarRef } from '../../core/types'

/** Validate the text before converting: empty input is not the number zero. */
export function numberError(
  value: string,
  options: { integer?: boolean; min?: number; max?: number; optional?: boolean } = {},
): string | null {
  if (!value.trim()) return options.optional ? null : '请填写数值'
  const n = Number(value)
  if (!Number.isFinite(n) || !/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(value.trim()))
    return '请输入有效数值'
  if (options.integer && !Number.isSafeInteger(n)) return '请输入有效整数'
  if (options.min !== undefined && n < options.min) return `不能小于 ${options.min}`
  if (options.max !== undefined && n > options.max) return `不能大于 ${options.max}`
  return null
}

export function parseExitCodes(value: string): number[] | null {
  const tokens = value.split(',').map(v => v.trim())
  if (!tokens.length || tokens.some(v => !/^[+-]?\d+$/.test(v))) return null
  const values = tokens.map(Number)
  if (values.some(v => !Number.isSafeInteger(v) || v < -2147483648 || v > 2147483647)) return null
  return [...new Set(values)]
}

export function changeConditionOperator(op: string, operands: VarRef[]): Condition {
  if (op === 'always_true' || op === 'always_false') return { always: op === 'always_true' }
  if (op === 'not_empty' || op === 'empty') return { [op]: operands[0] ?? '' } as Condition
  return { [op]: [operands[0] ?? '', operands[1] ?? ''] } as Condition
}

export function switchLoopMode(
  def: Record<string, unknown>,
  mode: string,
  saved: Record<string, unknown>,
): Record<string, unknown> {
  const next = { ...def }
  delete next.for_each
  delete next.repeat
  delete next.until
  next[mode] =
    saved[mode] ??
    (mode === 'for_each' ? { items: '', as: 'item' } : mode === 'repeat' ? 1 : { always: false })
  return next
}
