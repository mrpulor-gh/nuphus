/** Keep atomic progress receipts for history reconciliation, not separate UI bubbles. */
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

/** A presentation-only projection: retain receipt IDs in state for reconnect deduplication. */
export function composeAssistantReplies<
  T extends {
    id: string
    role: string
    content: string
    kind?: 'progress'
    message_id?: string
    reply_id?: string
    timestamp?: number
    traceItems?: unknown[]
    images?: string[]
    audio?: string[]
  },
>(messages: T[]): T[] {
  const out: T[] = []
  const seen = new Set<string>()
  let hasProgress = false
  for (const message of messages) {
    if (message.message_id && seen.has(message.message_id)) continue
    if (message.message_id) seen.add(message.message_id)
    const prev = out[out.length - 1]
    if (
      message.role === 'assistant' &&
      prev?.role === 'assistant' &&
      (hasProgress || message.kind === 'progress')
    ) {
      out[out.length - 1] = {
        ...prev,
        ...message,
        id: prev.id,
        timestamp: prev.timestamp ?? message.timestamp,
        content: [prev.content, message.content]
          .filter(text => text.trim())
          .map(text => text.replace(/^\n+|\n+$/g, ''))
          .join('\n\n'),
        traceItems: [...(prev.traceItems ?? []), ...(message.traceItems ?? [])],
        images: [...new Set([...(prev.images ?? []), ...(message.images ?? [])])],
        audio: [...new Set([...(prev.audio ?? []), ...(message.audio ?? [])])],
      }
      hasProgress = true
    } else {
      out.push(message.reply_id ? { ...message, id: message.reply_id } : message)
      hasProgress = message.kind === 'progress'
    }
  }
  // Tool-only placeholders have their own activity line, never an empty chat bubble.
  return out.filter(
    message =>
      message.role !== 'assistant' ||
      message.content.trim() ||
      message.images?.length ||
      message.audio?.length,
  )
}

/** Keep subsequent output after a newly appended user message, without moving old text. */
export function continueReplyAfterUser<
  T extends {
    id: string
    role: string
    content: string
    reply_id?: string
    timestamp?: number
    runtime?: 'live' | 'done'
    streaming?: boolean
    images?: string[]
    audio?: string[]
    traceItems?: unknown[]
  },
>(messages: T[], draftId: string): T[] {
  const index = messages.findIndex(message => message.id === draftId)
  if (index < 0) return messages
  const user = messages
    .slice(index + 1)
    .reverse()
    .find(message => message.role === 'user')
  if (!user) return messages
  const draft = messages[index]
  const previousId = `${draftId}:before:${user.id}`
  const previous = messages.map(message => {
    if (message.id === draftId)
      return { ...message, id: previousId, runtime: 'done' as const, streaming: false }
    if (message.reply_id === draftId) return { ...message, reply_id: previousId }
    return message
  })
  return [
    ...previous,
    {
      ...draft,
      content: '',
      timestamp: user.timestamp ?? Date.now(),
      images: undefined,
      audio: undefined,
      traceItems: [],
    },
  ]
}
