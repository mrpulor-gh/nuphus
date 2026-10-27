import { useEffect } from 'react'
import { parseShortcut } from '../ui/shortcutKeys'

export interface ShortcutDef {
  /** 可编辑命令的稳定 id（与 pages/shortcuts.ts 的 ShortcutItem.id 对齐）；存在覆盖时生效 */
  id?: string
  key: string
  ctrl?: boolean
  alt?: boolean
  shift?: boolean
  handler: () => void
  enabled?: () => boolean
}

/**
 * 全局键盘绑定（window 捕获）。
 *
 * v1.5 最小 resolver：ShortcutDef 携带 id 且有可解析的覆盖键位（canonical，如
 * "Ctrl+Shift+K"）时，以解析结果替换该项的 key/ctrl/alt/shift；无覆盖或解析失败
 * 一律回落默认键位。overrides 由 App 层持有，弹窗保存后即时刷新（无需重启）。
 */
export function useKeyboard(shortcuts: ShortcutDef[], overrides?: Record<string, string>) {
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      for (const s of shortcuts) {
        if (s.enabled && !s.enabled()) continue

        let key = s.key
        let ctrl = s.ctrl !== undefined ? s.ctrl : false
        let alt = s.alt !== undefined ? s.alt : false
        let shift = s.shift !== undefined ? s.shift : false
        if (s.id && overrides && overrides[s.id]) {
          const parsed = parseShortcut(overrides[s.id])
          if (parsed) {
            key = parsed.key
            ctrl = parsed.ctrl
            alt = parsed.alt
            shift = parsed.shift
          }
        }

        const matchCtrl = ctrl ? e.ctrlKey || e.metaKey : !e.ctrlKey && !e.metaKey
        const matchAlt = alt ? e.altKey : !e.altKey
        const matchShift = shift ? e.shiftKey : !e.shiftKey
        const matchKey = e.key.toLowerCase() === key.toLowerCase()

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
  }, [shortcuts, overrides])
}
