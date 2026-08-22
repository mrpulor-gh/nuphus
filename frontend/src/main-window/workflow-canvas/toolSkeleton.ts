/**
 * toolSkeleton.ts — 按工具 input_schema 生成 with 参数骨架
 *
 * 规则：required 属性按 type 占位（string→"" / number|integer→0 / boolean→false /
 * array→[] / object→{}），有 enum 取首值；schema 缺失/非法时容错为 {}。
 * 与 Inspector.parseRequired 同为 input_schema 的容错消费方，解析失败绝不抛错。
 */

/** 单个属性的占位值：enum 首值优先，其次按 type；未知类型回退空串 */
function placeholderFor(prop: Record<string, unknown> | undefined): unknown {
  if (!prop) return ''
  if (Array.isArray(prop.enum) && prop.enum.length > 0) return prop.enum[0]
  switch (prop.type) {
    case 'number':
    case 'integer':
      return 0
    case 'boolean':
      return false
    case 'array':
      return []
    case 'object':
      return {}
    default:
      return ''
  }
}

/** 从 input_schema（JSON Schema）生成 with 骨架：仅含 required 属性的类型占位 */
export function skeletonFromSchema(
  schema: Record<string, unknown> | undefined,
): Record<string, unknown> {
  if (!schema || typeof schema !== 'object') return {}
  const required = Array.isArray(schema.required)
    ? schema.required.filter((r): r is string => typeof r === 'string')
    : []
  const props =
    schema.properties && typeof schema.properties === 'object'
      ? (schema.properties as Record<string, unknown>)
      : {}
  const out: Record<string, unknown> = {}
  for (const name of required) {
    const p = props[name]
    out[name] = placeholderFor(
      p && typeof p === 'object' ? (p as Record<string, unknown>) : undefined,
    )
  }
  return out
}
