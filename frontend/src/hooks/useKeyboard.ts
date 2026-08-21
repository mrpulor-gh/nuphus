import { useEffect } from 'react'

interface ShortcutDef {
  key: string
  ctrl?: boolean
  alt?: boolean
  shift?: boolean
  handler: () => void
  enabled?: () => boolean
}

export function useKeyboard(shortcuts: ShortcutDef[]) {
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      for (const s of shortcuts) {
        if (s.enabled && !s.enabled()) continue

        const ctrl = s.ctrl !== undefined ? s.ctrl : false
        const alt = s.alt !== undefined ? s.alt : false
        const shift = s.shift !== undefined ? s.shift : false

        const matchCtrl = ctrl ? e.ctrlKey || e.metaKey : !e.ctrlKey && !e.metaKey
        const matchAlt = alt ? e.altKey : !e.altKey
        const matchShift = shift ? e.shiftKey : !e.shiftKey
        const matchKey = e.key.toLowerCase() === s.key.toLowerCase()

        if (matchCtrl && matchAlt && matchShift && matchKey) {
          e.preventDefault()
          e.stopPropagation()
          s.handler()
          return
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown, { capture: true })
    return () => window.removeEventListener('keydown', handleKeyDown, { capture: true })
  }, [shortcuts])
}
