/** Keep user-facing progress distinct, while retaining legacy assistant folding. */
export function foldAssistantHistory<
  T extends {
    role: string
    content: string
    kind?: 'progress'
    message_id?: string
    traceItems?: unknown[]
  },
>(messages: T[]): T[] {
  const out: T[] = []
  const seen = new Set<string>()
  for (const message of messages) {
    if (message.message_id && seen.has(message.message_id)) continue
    if (message.message_id) seen.add(message.message_id)
    const prev = out[out.length - 1]
    if (
      message.role === 'assistant' &&
      prev?.role === 'assistant' &&
      message.kind !== 'progress' &&
      prev.kind !== 'progress'
    ) {
      out[out.length - 1] = {
        ...prev,
        ...message,
        content: message.content.trim() ? message.content : prev.content,
        traceItems: [...(prev.traceItems ?? []), ...(message.traceItems ?? [])],
      }
    } else {
      out.push(message)
    }
  }
  // Pure tool rounds are trace data, not empty chat bubbles. Attach their trace
  // to the following assistant so the execution details remain available.
  return out.reduce<T[]>((result, message, index) => {
    const hasMedia = ['images', 'audio'].some(key => {
      const value = (message as Record<string, unknown>)[key]
      return Array.isArray(value) && value.length > 0
    })
    if (message.role === 'assistant' && !message.content.trim() && !hasMedia) {
      const next = out[index + 1]?.role === 'assistant' ? out[index + 1] : undefined
      if (next && message.traceItems?.length) {
        // Do not mutate the input history objects.
        const nextIndex = out.indexOf(next)
        out[nextIndex] = {
          ...next,
          traceItems: [...message.traceItems, ...(next.traceItems ?? [])],
        }
      } else if (message.traceItems?.length && result[result.length - 1]?.role === 'assistant') {
        const previous = result[result.length - 1]
        result[result.length - 1] = {
          ...previous,
          traceItems: [...(previous.traceItems ?? []), ...message.traceItems],
        }
      }
    } else result.push(message)
    return result
  }, [])
}
