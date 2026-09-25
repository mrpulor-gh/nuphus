export type PathSegment = string | number
export interface FieldReference {
  root: string
  segments: PathSegment[]
}

/** Bracket paths use JSON strings/indices, not JavaScript evaluation. */
export function parseFieldReference(source: string): FieldReference | null {
  const head = source.trim().match(/^([A-Za-z_][A-Za-z0-9_.]*)\s*(?=\[)/)
  if (!head) return null
  const segments: PathSegment[] = []
  let rest = source.trim().slice(head[0].length)
  while (rest) {
    const token = rest.match(/^\[\s*("(?:[^"\\]|\\.)*"|0|[1-9][0-9]*)\s*\]\s*/)
    if (!token) return null
    try {
      const value: unknown = JSON.parse(token[1])
      if (
        typeof value !== 'string' &&
        !(typeof value === 'number' && Number.isSafeInteger(value) && value >= 0)
      )
        return null
      segments.push(value as PathSegment)
    } catch {
      return null
    }
    rest = rest.slice(token[0].length)
  }
  return { root: head[1], segments }
}

export function fieldReference(root: string, segments: PathSegment[]): string {
  return root + segments.map(segment => `[${JSON.stringify(segment)}]`).join('')
}

export interface TemplateSpan {
  start: number
  end: number
  body: string
  bodyStart: number
}
export function templateSpans(text: string): TemplateSpan[] {
  const spans: TemplateSpan[] = []
  let offset = 0
  while (offset < text.length) {
    const start = text.indexOf('{{', offset)
    if (start < 0) break
    let quoted = false,
      escaped = false,
      found = false
    for (let i = start + 2; i < text.length - 1; i++) {
      const c = text[i]
      if (escaped) {
        escaped = false
        continue
      }
      if (quoted && c === '\\') {
        escaped = true
        continue
      }
      if (c === '"') {
        quoted = !quoted
        continue
      }
      if (!quoted && c === '}' && text[i + 1] === '}') {
        spans.push({ start, end: i + 2, bodyStart: start + 2, body: text.slice(start + 2, i) })
        offset = i + 2
        found = true
        break
      }
    }
    if (!found) break
  }
  return spans
}

export function referenceRoot(expression: string): { name: string; input?: boolean } | null {
  const parsed = parseFieldReference(expression)
  if (parsed) {
    if (parsed.root === 'inputs' && typeof parsed.segments[0] === 'string')
      return { name: parsed.segments[0], input: true }
    if (parsed.root.startsWith('inputs.'))
      return { name: parsed.root.slice(7).split('.')[0], input: true }
    if (parsed.root === 'params' || parsed.root === 'ENV') return null
    return { name: parsed.root }
  }
  const match = expression.trim().match(/^([A-Za-z_]\w*)(?:\.([A-Za-z_]\w*))?/)
  if (!match || match[1] === 'ENV' || match[1] === 'params') return null
  return match[1] === 'inputs' && match[2] ? { name: match[2], input: true } : { name: match[1] }
}
