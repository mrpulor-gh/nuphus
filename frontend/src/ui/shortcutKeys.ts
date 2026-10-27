/**
 * shortcutKeys.ts — 快捷键 canonical 解析器与冲突检测（快捷键编辑 v1.5 的规则唯一源）。
 *
 * canonical 形式：修饰键（固定顺序 Ctrl,Alt,Shift）+ 主键，如 "Ctrl+Shift+K"。
 * 该形式是落库与比较的唯一口径；平台显示一律经 formatShortcutForDisplay 派生
 * （Mac 上 Ctrl→Cmd，与 ui/platformShortcut 同规则）。
 *
 * 消费者：
 *   - hooks/useKeyboard：存在覆盖时按解析结果替换默认键位（解析失败回落默认）；
 *   - pages/shortcuts：清单「默认 + 覆盖」合并展示；
 *   - pages/ShortcutsDialog：录制归一、冲突检测。
 */
import { isMacPlatform } from './platformShortcut'

/** 解析结果：匹配一次按键所需的最小信息（与 useKeyboard 的匹配口径一致）。 */
export interface ShortcutParts {
  key: string
  ctrl: boolean
  alt: boolean
  shift: boolean
}

/** 系统保留键：复制/粘贴/剪切/全选/撤销/关闭为系统级语义，命令绑定不得占用。 */
export const RESERVED_SHORTCUTS: readonly string[] = [
  'Ctrl+C',
  'Ctrl+V',
  'Ctrl+X',
  'Ctrl+A',
  'Ctrl+Z',
  'Ctrl+W',
]

/** 修饰键 token（大小写无关；含 Mac 写法与符号，统一归一为 ctrl/alt/shift）。 */
const MODIFIER_TOKENS: Record<string, 'ctrl' | 'alt' | 'shift'> = {
  ctrl: 'ctrl',
  control: 'ctrl',
  cmd: 'ctrl',
  command: 'ctrl',
  meta: 'ctrl',
  '⌘': 'ctrl',
  alt: 'alt',
  option: 'alt',
  '⌥': 'alt',
  shift: 'shift',
  '⇧': 'shift',
}

/** 主键中不易读的字面符号 → 可读名（保证 canonical 可用 '+' 连接）。 */
const MAIN_KEY_ALIASES: Record<string, string> = {
  ' ': 'Space',
  '+': 'Plus',
}

/** 主键归一：单字符统一大写（字母/数字/符号一致），多字符按原名。 */
function normalizeMainKey(key: string): string {
  if (key.length === 1) return key.toUpperCase()
  return MAIN_KEY_ALIASES[key] ?? key
}

/**
 * 解析 canonical。非法输入（空串、未知 token、重复修饰键、仅修饰键）→ null；
 * 解析失败的所有调用方一律按「默认键位」回落，不抛错。
 */
export function parseShortcut(canonical: string): ShortcutParts | null {
  const tokens = canonical.split('+').map(token => token.trim())
  if (tokens.some(token => !token)) return null
  const main = tokens[tokens.length - 1]
  if (!main || MODIFIER_TOKENS[main.toLowerCase()]) return null // 空主键 / 仅修饰键
  const parts: ShortcutParts = {
    key: normalizeMainKey(main),
    ctrl: false,
    alt: false,
    shift: false,
  }
  for (const token of tokens.slice(0, -1)) {
    const slot = MODIFIER_TOKENS[token.toLowerCase()]
    if (!slot || parts[slot]) return null // 未知修饰键 / 重复修饰键
    parts[slot] = true
  }
  return parts
}

/** 组装 canonical：修饰键固定顺序 Ctrl,Alt,Shift + 主键。 */
export function formatShortcut(parts: ShortcutParts): string {
  const tokens: string[] = []
  if (parts.ctrl) tokens.push('Ctrl')
  if (parts.alt) tokens.push('Alt')
  if (parts.shift) tokens.push('Shift')
  tokens.push(normalizeMainKey(parts.key))
  return tokens.join('+')
}

/**
 * 平台显示文案：canonical → 当前平台读法（Mac Ctrl→Cmd）。
 * platform 参数仅供测试注入；解析失败时原样返回（不吞错、不造第二种形状）。
 */
export function formatShortcutForDisplay(canonical: string, platform?: string): string {
  const parts = parseShortcut(canonical)
  if (!parts) return canonical
  const tokens: string[] = []
  if (parts.ctrl) tokens.push(isMacPlatform(platform) ? 'Cmd' : 'Ctrl')
  if (parts.alt) tokens.push('Alt')
  if (parts.shift) tokens.push('Shift')
  tokens.push(parts.key)
  return tokens.join('+')
}

/** 录制时不得作为组合主键的按键：修饰/锁定/取消/未知。 */
const NON_MAIN_KEYS = new Set([
  'Control',
  'Alt',
  'Shift',
  'Meta',
  'AltGraph',
  'CapsLock',
  'NumLock',
  'ScrollLock',
  'Escape', // Esc = 取消录制手势，不作为可绑主键
  'Dead',
  'Unidentified',
])

/**
 * 从键盘事件生成 canonical（录制态使用）。
 * 仅修饰键/锁定键/Esc/未知键 → null（调用方据此「拒绝仅修饰键」；Esc 由调用方先行处理取消）。
 */
export function shortcutFromKeyEvent(event: {
  key: string
  ctrlKey: boolean
  altKey: boolean
  shiftKey: boolean
  metaKey: boolean
}): string | null {
  if (!event.key || NON_MAIN_KEYS.has(event.key)) return null
  return formatShortcut({
    key: event.key,
    ctrl: event.ctrlKey || event.metaKey,
    alt: event.altKey,
    shift: event.shiftKey,
  })
}

/** 冲突检测的「有效键位」条目：id + 展示名 + 当前 canonical（override ?? 默认）。 */
export interface ShortcutConflictEntry {
  id: string
  label: string
  canonical: string
}

export type ShortcutConflict = { kind: 'reserved' } | { kind: 'item'; id: string; label: string }

/** 同一组合判定：按解析结果比较（大小写/顺序/写法无关）；解析失败按原文严格比较。 */
export function isSameShortcut(a: string, b: string): boolean {
  const pa = parseShortcut(a)
  const pb = parseShortcut(b)
  if (!pa || !pb) return a === b
  return (
    pa.key.toLowerCase() === pb.key.toLowerCase() &&
    pa.ctrl === pb.ctrl &&
    pa.alt === pb.alt &&
    pa.shift === pb.shift
  )
}

/**
 * 冲突检测：候选组合与保留键、其它可编辑项的有效键位任一重复即拒绝。
 * entries 传全部可编辑项（含自身也无妨——excludeId 精确排除自身）。
 */
export function detectShortcutConflict(
  candidate: string,
  entries: ShortcutConflictEntry[],
  excludeId?: string,
): ShortcutConflict | null {
  if (RESERVED_SHORTCUTS.some(reserved => isSameShortcut(candidate, reserved))) {
    return { kind: 'reserved' }
  }
  for (const entry of entries) {
    if (entry.id === excludeId) continue
    if (isSameShortcut(candidate, entry.canonical)) {
      return { kind: 'item', id: entry.id, label: entry.label }
    }
  }
  return null
}

/** 全局热键（system 类）合法性：必须含至少一个修饰键（Ctrl/Alt/Shift）。 */
export function isValidGlobalShortcut(parts: ShortcutParts): boolean {
  return parts.ctrl || parts.alt || parts.shift
}
