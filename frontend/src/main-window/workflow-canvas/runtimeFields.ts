import { fieldReference, type PathSegment } from './fieldReferences'
import type { VariableCatalog } from './variableCatalog'

export interface RuntimeField {
  root: string
  segments: PathSegment[]
  expression: string
  value: unknown
}

/** Only inspect persisted values. Strings (including JSON-looking strings) remain strings. */
export function runtimeChildren(
  field: RuntimeField,
  limit = Number.POSITIVE_INFINITY,
): RuntimeField[] {
  const value = field.value
  if (!value || typeof value !== 'object') return []
  const keys = Array.isArray(value)
    ? Array.from({ length: Math.min(value.length, limit) }, (_, index) => String(index))
    : Object.keys(value).slice(0, limit)
  return keys.map(key => {
    const segment = Array.isArray(value) ? Number(key) : key
    const segments = [...field.segments, segment]
    return {
      root: field.root,
      segments,
      expression: fieldReference(field.root, segments),
      value: (value as Record<string, unknown>)[key],
    }
  })
}

export function runtimeChildCount(value: unknown): number {
  return Array.isArray(value)
    ? value.length
    : value && typeof value === 'object'
      ? Object.keys(value).length
      : 0
}

/** Static scope limits which observed roots are reusable; it never supplies inferred children. */
export function runtimeRoots(
  values: Record<string, unknown>,
  catalog: VariableCatalog,
): RuntimeField[] {
  const fields: RuntimeField[] = []
  const seen = new Set<string>()
  for (const candidate of catalog.references) {
    const input = candidate.source === 'input' && candidate.name.startsWith('inputs.')
    const root = input ? 'inputs' : candidate.name
    const segments = input ? [candidate.name.slice(7)] : []
    if (!Object.prototype.hasOwnProperty.call(values, root)) continue
    let value = values[root]
    if (input) {
      if (
        !value ||
        typeof value !== 'object' ||
        !Object.prototype.hasOwnProperty.call(value, segments[0])
      )
        continue
      value = (value as Record<string, unknown>)[segments[0]]
    }
    const expression = fieldReference(root, segments)
    if (!seen.has(expression)) fields.push({ root, segments, expression, value })
    seen.add(expression)
  }
  return fields
}

export function runtimePreview(value: unknown): string {
  if (Array.isArray(value)) return `[${value.length}]`
  if (value && typeof value === 'object') return `{${Object.keys(value).length}}`
  const text = JSON.stringify(value) ?? String(value)
  return text.length > 120 ? `${text.slice(0, 120)}…` : text
}
