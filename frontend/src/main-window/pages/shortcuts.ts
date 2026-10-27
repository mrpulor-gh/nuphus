/**
 * shortcuts.ts — 键盘快捷键清单（唯一事实源）。
 *
 * 消费者：HelpPage（帮助分区「快捷键」表格）、GeneralPage 的「快捷键」弹窗，
 * 与 App.tsx useKeyboard 的实际绑定一一对应（改绑定时两处同步——帮助/弹窗的
 * 价值就在于「说的都能用」）。
 *
 * 类别（category）：
 *   - command：应用内命令，**可编辑**（v1.5）。`id` 即 UserPreferences
 *     `shortcut_overrides` 的存储键（与 App.tsx useKeyboard 的 ShortcutDef.id 对齐）；
 *     `defaultCanonical` 为默认键位（canonical，固定顺序 Ctrl,Alt,Shift + 主键），
 *     显示经 formatShortcutForDisplay 按平台派生。
 *   - input：输入交互键（Enter / Shift+Enter / Esc），不可编辑，弹窗标注类别。
 *   - system：系统级全局热键（Ctrl+1/2/3，注册于后端；v1.5.1 起可编辑——改键由后端运行时重注册），弹窗标注类别。
 *
 * 合并展示：buildShortcutItems(overrides) = 默认清单 + 覆盖（覆盖不可解析时回落默认）；
 * HelpPage 与弹窗共用本函数产物，不复制清单。
 */
import { formatShortcut, formatShortcutForDisplay, parseShortcut } from '../../ui/shortcutKeys'
import { formatPrimaryShortcut } from '../../ui/platformShortcut'

export type ShortcutCategory = 'command' | 'input' | 'system'

export interface ShortcutItem {
  /** i18n key（操作名称） */
  key: string
  /** 当前键位文案（覆盖合并后，已按当前平台派生 Cmd/Ctrl） */
  keys: string
  category: ShortcutCategory
  /** command 类专有：override 存储键（与 App.tsx useKeyboard 的 ShortcutDef.id 对齐） */
  id?: string
  /** command 类专有：当前有效 canonical（默认或覆盖归一后） */
  canonical?: string
  /** command 类专有：当前键位与默认不同（展示「已自定义」标记） */
  overridden?: boolean
}

interface ShortcutBaseItem {
  key: string
  category: ShortcutCategory
  /** command 类：override 存储键 */
  id?: string
  /** command 类：默认 canonical */
  defaultCanonical?: string
  /** input / system 类：直接展示的键位文案（不可编辑） */
  fixedKeys?: string
}

/** 清单骨架：与 App.tsx useKeyboard 绑定顺序同源码；command 类的 id 与绑定的 id 对齐。 */
const BASE: ShortcutBaseItem[] = [
  { key: 'help.shortcut.send', category: 'input', fixedKeys: 'Enter' },
  {
    key: 'help.shortcut.newline',
    category: 'input',
    fixedKeys: `Shift+Enter / ${formatPrimaryShortcut('Enter')}`,
  },
  { key: 'help.shortcut.palette', category: 'command', id: 'palette', defaultCanonical: 'Ctrl+K' },
  {
    key: 'help.shortcut.workflowList',
    category: 'command',
    id: 'workflowList',
    defaultCanonical: 'Ctrl+F',
  },
  {
    key: 'help.shortcut.focusInput',
    category: 'command',
    id: 'focusInput',
    defaultCanonical: 'Ctrl+L',
  },
  { key: 'help.shortcut.newChat', category: 'command', id: 'newChat', defaultCanonical: 'Ctrl+N' },
  {
    key: 'help.shortcut.workflowPanel',
    category: 'command',
    id: 'workflowPanel',
    defaultCanonical: 'Ctrl+Shift+W',
  },
  { key: 'help.shortcut.esc', category: 'input', fixedKeys: 'Esc' },
  // 桌面工具全局热键（在主窗内与其它应用前台时均可用；注册于后端，v1.5.1 起可编辑——
  // id 与 main.rs 注册映射对齐，改键走同一 overrides 表、后端运行时重注册）
  {
    key: 'help.shortcut.toolScreenshot',
    category: 'system',
    id: 'global.tool.screenshot',
    defaultCanonical: 'Ctrl+1',
  },
  {
    key: 'help.shortcut.toolPicker',
    category: 'system',
    id: 'global.tool.picker',
    defaultCanonical: 'Ctrl+2',
  },
  {
    key: 'help.shortcut.toolColorPicker',
    category: 'system',
    id: 'global.tool.color_picker',
    defaultCanonical: 'Ctrl+3',
  },
]

/** 构建「默认 + 覆盖」合并清单（覆盖不可解析时回落默认；不改写清单形状）。 */
export function buildShortcutItems(overrides?: Record<string, string>): ShortcutItem[] {
  return BASE.map(item => {
    if (!item.id || !item.defaultCanonical) {
      return { key: item.key, keys: item.fixedKeys ?? '', category: item.category }
    }
    const override = overrides?.[item.id]
    const parsed = override ? parseShortcut(override) : null
    const canonical = parsed ? formatShortcut(parsed) : item.defaultCanonical
    return {
      key: item.key,
      keys: formatShortcutForDisplay(canonical),
      category: item.category,
      id: item.id,
      canonical,
      overridden: canonical !== item.defaultCanonical,
    }
  })
}
