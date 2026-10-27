/** shortcutKeys 解析器 + 冲突检测单测（快捷键编辑 v1.5 规则源）。 */
import { describe, expect, it } from 'vitest'
import {
  detectShortcutConflict,
  formatShortcut,
  formatShortcutForDisplay,
  isSameShortcut,
  parseShortcut,
  shortcutFromKeyEvent,
} from './shortcutKeys'

describe('parseShortcut / formatShortcut', () => {
  it('解析 canonical：大小写无关、修饰键序无关，主键归一大写', () => {
    expect(parseShortcut('Ctrl+K')).toEqual({ key: 'K', ctrl: true, alt: false, shift: false })
    expect(parseShortcut('shift+ctrl+k')).toEqual({ key: 'K', ctrl: true, alt: false, shift: true })
    expect(parseShortcut('Cmd+Shift+W')).toEqual({ key: 'W', ctrl: true, alt: false, shift: true })
  })

  it('format 固定顺序 Ctrl,Alt,Shift + 主键', () => {
    expect(formatShortcut({ key: 'K', ctrl: true, alt: true, shift: true })).toBe(
      'Ctrl+Alt+Shift+K',
    )
    expect(formatShortcut({ key: 'k', ctrl: true, alt: false, shift: true })).toBe('Ctrl+Shift+K')
  })

  it('往返归一化：format(parse(x)) 得到规范形式', () => {
    const parts = parseShortcut('ctrl+shift+k')
    expect(parts && formatShortcut(parts)).toBe('Ctrl+Shift+K')
  })

  it('非法输入一律 null（空串 / 未知 token / 重复修饰键 / 仅修饰键 / 残缺）', () => {
    for (const bad of [
      '',
      'Ctrl',
      'Shift',
      'Ctrl+Shift',
      'Ctrl+',
      '+K',
      'Ctrl+Ctrl+K',
      'Ctrl+Control+K',
      'Foo+K',
    ]) {
      expect(parseShortcut(bad)).toBeNull()
    }
  })

  it('无修饰键的单键可解析（只拒绝「仅修饰键」，不额外加规则）', () => {
    expect(parseShortcut('K')).toEqual({ key: 'K', ctrl: false, alt: false, shift: false })
  })
})

describe('shortcutFromKeyEvent（录制归一）', () => {
  const ev = (
    key: string,
    mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {},
  ) => ({ key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods })

  it('metaKey 归一为 Ctrl（与 useKeyboard 匹配口径一致），主键大写', () => {
    expect(shortcutFromKeyEvent(ev('k', { ctrlKey: true }))).toBe('Ctrl+K')
    expect(shortcutFromKeyEvent(ev('k', { metaKey: true, shiftKey: true }))).toBe('Ctrl+Shift+K')
  })

  it('拒绝仅修饰键 / 锁定键 / Esc / 空键（保持录制态由调用方处理）', () => {
    expect(shortcutFromKeyEvent(ev('Control', { ctrlKey: true }))).toBeNull()
    expect(shortcutFromKeyEvent(ev('Shift', { shiftKey: true }))).toBeNull()
    expect(shortcutFromKeyEvent(ev('Escape'))).toBeNull()
    expect(shortcutFromKeyEvent(ev('CapsLock'))).toBeNull()
  })
})

describe('formatShortcutForDisplay', () => {
  it('Mac 平台 Ctrl→Cmd，其余平台保持 Ctrl', () => {
    expect(formatShortcutForDisplay('Ctrl+Shift+W', 'MacIntel')).toBe('Cmd+Shift+W')
    expect(formatShortcutForDisplay('Ctrl+Shift+W', 'Win32')).toBe('Ctrl+Shift+W')
  })
})

describe('detectShortcutConflict / isSameShortcut', () => {
  const entries = [
    { id: 'palette', label: '打开命令面板', canonical: 'Ctrl+K' },
    { id: 'focusInput', label: '聚焦输入框', canonical: 'Ctrl+L' },
  ]

  it('命中系统保留键 → reserved（写法归一后比较）', () => {
    expect(detectShortcutConflict('Ctrl+C', entries)).toEqual({ kind: 'reserved' })
    expect(detectShortcutConflict('ctrl+v', entries)).toEqual({ kind: 'reserved' })
  })

  it('与其它可编辑项重复 → item（带冲突对象）', () => {
    expect(detectShortcutConflict('ctrl+k', entries)).toEqual({
      kind: 'item',
      id: 'palette',
      label: '打开命令面板',
    })
  })

  it('排除自身：改自己为同组合不算冲突', () => {
    expect(detectShortcutConflict('Ctrl+K', entries, 'palette')).toBeNull()
  })

  it('无冲突 → null；isSameShortcut 解析失败按原文比较', () => {
    expect(detectShortcutConflict('Ctrl+Shift+J', entries)).toBeNull()
    expect(isSameShortcut('Ctrl+K', 'ctrl+k')).toBe(true)
    expect(isSameShortcut('garbage', 'garbage')).toBe(true)
    expect(isSameShortcut('garbage', 'Ctrl+K')).toBe(false)
  })
})
