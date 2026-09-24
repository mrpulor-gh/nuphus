/** Only declared defaults are deliberate values. Required fields remain visibly unset. */
export function skeletonFromSchema(
  schema: Record<string, unknown> | undefined,
): Record<string, unknown> {
  if (
    !schema?.properties ||
    typeof schema.properties !== 'object' ||
    Array.isArray(schema.properties)
  )
    return {}
  const out: Record<string, unknown> = {}
  for (const [name, property] of Object.entries(schema.properties)) {
    if (
      property &&
      typeof property === 'object' &&
      Object.prototype.hasOwnProperty.call(property, 'default')
    ) {
      out[name] = structuredClone((property as Record<string, unknown>).default)
    }
  }
  return out
}
