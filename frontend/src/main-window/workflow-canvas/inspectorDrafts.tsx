import { createContext, useContext, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { flushSync } from 'react-dom'

type Commit = (text: string) => unknown
type Entry = {
  nodeId: string
  field: string
  text: string
  baseline: string
  error: string | null
  validate?: (text: string) => string | null
  onCommit: Commit
  active: boolean
}

/** Canvas-owned drafts. A field remains dirty even when its editor is hidden. */
export class InspectorDraftStore {
  entries = new Map<string, Entry>()
  private listeners = new Set<() => void>()
  private revision = 0
  private pending = new Map<string, Promise<boolean>>()
  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }
  snapshot = () => this.revision
  changed = () => {
    this.revision++
    this.listeners.forEach(listener => listener())
  }
  get dirty() {
    return [...this.entries.values()].some(e => e.active && e.text !== e.baseline)
  }
  renameNode(from: string, to: string) {
    for (const [key, entry] of this.entries) {
      if (entry.nodeId !== from) continue
      this.entries.delete(key)
      entry.nodeId = to
      this.entries.set(`${to}\u0000${entry.field}`, entry)
    }
  }
  prune(nodeIds: Set<string>) {
    let removed = false
    for (const [key, entry] of this.entries) {
      if (!nodeIds.has(entry.nodeId)) {
        this.entries.delete(key)
        removed = true
      }
    }
    if (removed) this.changed()
  }
  commit(key: string): Promise<boolean> {
    const pending = this.pending.get(key)
    if (pending) return pending
    const task = this.commitEntry(key).finally(() => this.pending.delete(key))
    this.pending.set(key, task)
    return task
  }
  private async commitEntry(key: string) {
    const entry = this.entries.get(key)
    if (!entry || !entry.active || entry.text === entry.baseline) return true
    entry.error = entry.validate?.(entry.text) ?? null
    if (entry.error) {
      this.changed()
      return false
    }
    const submitted = entry.text
    let result: unknown
    // Each field sees the tree produced by the previous field, not a stale render.
    try {
      flushSync(() => {
        result = entry.onCommit(submitted)
      })
      if ((await result) === false) return false
    } catch (error) {
      entry.error = String(error)
      this.changed()
      return false
    }
    entry.baseline = submitted
    this.changed()
    return true
  }
  async flush(): Promise<{ nodeId: string; field: string; error: string } | null> {
    // Validate the entire batch before applying any of it.
    for (const entry of this.entries.values()) {
      if (!entry.active || entry.text === entry.baseline) continue
      entry.error = entry.validate?.(entry.text) ?? null
      if (entry.error) {
        this.changed()
        return { ...entry, error: entry.error }
      }
    }
    for (const [key, entry] of this.entries) {
      if (!(await this.commit(key)))
        return { ...entry, error: entry.error ?? '该修改尚未确认或不允许保存' }
    }
    return null
  }
}

export const InspectorDraftContext = createContext<{
  store: InspectorDraftStore
  nodeId: string
} | null>(null)

export function useInspectorDraft(
  field: string,
  value: string,
  options: { onCommit: Commit; validate?: (text: string) => string | null },
) {
  const context = useContext(InspectorDraftContext)
  const local = useRef(new InspectorDraftStore())
  const store = context?.store ?? local.current
  const key = `${context?.nodeId ?? ''}\u0000${field}`
  useSyncExternalStore(store.subscribe, store.snapshot, store.snapshot)
  const [, render] = useState(0)
  let entry = store.entries.get(key)
  if (!entry) {
    entry = {
      nodeId: context?.nodeId ?? '',
      field,
      text: value,
      baseline: value,
      error: null,
      active: true,
      ...options,
    }
    store.entries.set(key, entry)
  } else {
    // External undo/redo refreshes only clean fields; unfinished edits are never erased.
    if (entry.text === entry.baseline && value !== entry.baseline)
      entry.text = entry.baseline = value
    entry.onCommit = options.onCommit
    entry.validate = options.validate
  }
  useEffect(() => {
    entry.active = true
    return () => {
      entry.active = false
    }
  }, [entry])
  const setText = (text: string) => {
    entry.text = text
    entry.error = entry.validate?.(text) ?? null
    store.changed()
    render(n => n + 1)
  }
  const reset = (text: string) => {
    entry.text = entry.baseline = text
    entry.error = null
    store.changed()
  }
  return { text: entry.text, setText, reset, commit: () => store.commit(key), error: entry.error }
}
