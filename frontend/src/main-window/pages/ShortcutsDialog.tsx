/**
 * ShortcutsDialog.tsx — 「快捷键」弹窗（通用页入口行打开；v1.5 支持行内编辑）。
 *
 * 版式（参照图 B）：标题 + 搜索框 + 列表 + 底部「恢复全部默认」。
 *   - 可编辑行（command）：点击进入录制态（「按下新组合键」；Esc 取消）；
 *     仅修饰键忽略；与其它项或系统保留键冲突 → 提示并拒绝；成功即落库并回传 App 层。
 *   - 不可编辑行（input / system）：行内标注类别。
 * 数据与 HelpPage 同源（pages/shortcuts.ts 唯一事实源，经覆盖合并），不在此处维护第二份清单。
 */
import { useEffect, useMemo, useState } from 'react'
import { CompactModal } from '../layout/CompactModal'
import { useLanguage } from '../../locales'
import { getShortcutOverrides, setShortcutOverrides } from '../lib/api'
import { buildShortcutItems } from './shortcuts'
import {
  detectShortcutConflict,
  formatShortcut,
  isValidGlobalShortcut,
  parseShortcut,
  shortcutFromKeyEvent,
  type ShortcutConflictEntry,
} from '../../ui/shortcutKeys'

export interface ShortcutsDialogProps {
  open: boolean
  onClose: () => void
  /** 保存 / 恢复成功后回传全量生效映射（App 层据此刷新 useKeyboard 生效链路） */
  onOverridesChange?: (overrides: Record<string, string>) => void
  /** 录制态广播（App 层挂起 Ctrl+K/F 等全局分支，避免录制组合被截胡） */
  onRecordingChange?: (recording: boolean) => void
}

export function ShortcutsDialog({
  open,
  onClose,
  onOverridesChange,
  onRecordingChange,
}: ShortcutsDialogProps) {
  const { t } = useLanguage()
  const [overrides, setOverrides] = useState<Record<string, string>>({})
  const [recordingId, setRecordingId] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  // 开 / 关都重置录制态；打开时读取覆盖表（后端为唯一事实源）
  useEffect(() => {
    setRecordingId(null)
    setFeedback(null)
    if (!open) return
    void getShortcutOverrides()
      .then(setOverrides)
      .catch(() => {})
  }, [open])

  // 录制态广播（含录制结束与卸载清理，App 层据此挂起全局分支）
  useEffect(() => {
    if (recordingId == null) return
    onRecordingChange?.(true)
    return () => onRecordingChange?.(false)
  }, [recordingId, onRecordingChange])

  const items = useMemo(() => buildShortcutItems(overrides), [overrides])

  /** 冲突检测的有效键位清单（override ?? 默认），label 用当前语言的操作名。 */
  const conflictEntries = useMemo<ShortcutConflictEntry[]>(
    () =>
      items.flatMap(item =>
        item.category === 'command' && item.id && item.canonical
          ? [{ id: item.id, label: t(item.key), canonical: item.canonical }]
          : [],
      ),
    [items, t],
  )

  /** 分段：不可编辑（输入交互键）在上，可编辑（命令键 + 全局热键）在下。 */
  const readonlyItems = useMemo(() => items.filter(item => item.category === 'input'), [items])
  const editableItems = useMemo(() => items.filter(item => item.category !== 'input'), [items])

  /** 落库：乐观更新 + 成功后回传；失败回滚并提示（界面不得与落库背离）。 */
  const persist = (next: Record<string, string>, prev: Record<string, string>) => {
    setSaving(true)
    setOverrides(next)
    setFeedback(null)
    void setShortcutOverrides(next)
      .then(applied => {
        setOverrides(applied)
        onOverridesChange?.(applied)
      })
      .catch(e => {
        setOverrides(prev)
        const detail = e instanceof Error ? e.message : String(e)
        setFeedback(detail && detail.length < 120 ? detail : t('general.shortcuts.saveFailed'))
      })
      .finally(() => setSaving(false))
  }

  // 录制键盘：window 捕获，先于弹窗自身键处理与其它全局分支；Esc 取消
  useEffect(() => {
    if (recordingId == null) return
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      e.stopImmediatePropagation()
      if (e.key === 'Escape') {
        setRecordingId(null)
        setFeedback(null)
        return
      }
      const candidate = shortcutFromKeyEvent(e)
      if (!candidate) return // 仅修饰键 / 锁定键：忽略，保持录制
      const parsed = parseShortcut(candidate)
      if (!parsed) return
      const combo = formatShortcut(parsed)
      // 全局热键约束：必须含修饰键（系统级注册不允许裸键）
      const targetItem = items.find(item => item.id === recordingId)
      if (targetItem?.category === 'system' && !isValidGlobalShortcut(parsed)) {
        setFeedback(t('general.shortcuts.mustModifier'))
        return
      }
      const conflict = detectShortcutConflict(combo, conflictEntries, recordingId)
      if (conflict) {
        // 拒绝并保持录制，等待下一个组合（Esc 可随时取消）
        setFeedback(
          conflict.kind === 'reserved'
            ? t('general.shortcuts.conflictReserved')
            : t('general.shortcuts.conflictDuplicate'),
        )
        return
      }
      setRecordingId(null)
      persist({ ...overrides, [recordingId]: combo }, overrides)
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [recordingId, overrides, conflictEntries, t, items])

  /** 点击行：进入 / 退出录制（再点同行 = 取消）。 */
  const toggleRecording = (id: string) => {
    if (saving) return
    setFeedback(null)
    setRecordingId(current => (current === id ? null : id))
  }

  const hasOverrides = Object.keys(overrides).length > 0
  const resetAll = () => {
    if (saving || recordingId != null || !hasOverrides) return
    persist({}, overrides)
  }

  return (
    <CompactModal
      open={open}
      onClose={onClose}
      title={t('help.shortcuts')}
      size="md"
      layer="settings"
      className="compact-modal--fit"
      footer={
        <>
          <span className="shortcuts-note">{t('general.shortcuts.hint')}</span>
          <button
            type="button"
            className="shortcuts-reset"
            disabled={saving || recordingId != null || !hasOverrides}
            onClick={resetAll}
          >
            {t('general.shortcuts.reset')}
          </button>
        </>
      }
    >
      <div className="shortcuts-list">
        {feedback && (
          <div className="shortcuts-feedback" role="alert">
            {feedback}
          </div>
        )}
        {readonlyItems.length > 0 && (
          <>
            <div className="shortcuts-group-title">{t('general.shortcuts.groupReadonly')}</div>
            {readonlyItems.map(item => (
              <div key={item.key} className="shortcuts-row">
                <span className="shortcuts-name">{t(item.key)}</span>
                <span className="shortcuts-keys">{item.keys}</span>
              </div>
            ))}
          </>
        )}
        {editableItems.length > 0 && (
          <>
            <div className="shortcuts-group-title">{t('general.shortcuts.groupEditable')}</div>
            {editableItems.map(item => (
              <button
                type="button"
                key={item.key}
                className={`shortcuts-row is-editable${recordingId === item.id ? ' is-recording' : ''}`}
                aria-label={`${t(item.key)} ${item.keys}`}
                onClick={() => toggleRecording(item.id as string)}
              >
                <span className="shortcuts-name">
                  {t(item.key)}
                  <span className="shortcuts-cat">
                    {t(
                      item.category === 'command'
                        ? 'general.shortcuts.catCommand'
                        : 'general.shortcuts.catSystem',
                    )}
                  </span>
                  {item.overridden && (
                    <span className="shortcuts-badge">{t('general.shortcuts.overridden')}</span>
                  )}
                </span>
                <span className="shortcuts-keys">
                  {recordingId === item.id ? t('general.shortcuts.recording') : item.keys}
                </span>
              </button>
            ))}
          </>
        )}
      </div>
    </CompactModal>
  )
}
