import { useState, useEffect, useRef, useCallback, useMemo, type ReactNode } from 'react'
import { useLanguage } from '../locales'
import { playUiSound } from './sound'
import '../styles/cmd-palette.css'

interface CommandItem {
  id: string
  label: string
  desc?: string
  category: string
  action: () => void
}

interface CommandPaletteProps {
  open: boolean
  onClose: () => void
  items: CommandItem[]
  iconMap?: Record<string, ReactNode>
  placement?: 'center' | 'bottom'
  initialFilter?: string
}

export function CommandPalette({
  open,
  onClose,
  items,
  iconMap,
  placement = 'center',
  initialFilter,
}: CommandPaletteProps) {
  const { t } = useLanguage()
  const [query, setQuery] = useState(initialFilter || '')
  const [selectedIdx, setSelectedIdx] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const resultsRef = useRef<HTMLDivElement>(null)
  const itemRefs = useRef<(HTMLDivElement | null)[]>([])
  // 选择索引镜像：document 级键盘 handler 用 ref 读最新值（闭包不依赖 selectedIdx，
  // 避免 effect 依赖重挂；setState updater 保持纯函数，音效判断放外面）
  const selectedIdxRef = useRef(selectedIdx)
  useEffect(() => {
    selectedIdxRef.current = selectedIdx
  }, [selectedIdx])

  /** 选择移动反馈（键盘/鼠标共用）：索引实际变化才响；同步镜像 ref 防快速连按误响 */
  const moveSelection = useCallback((next: number) => {
    if (next !== selectedIdxRef.current) {
      selectedIdxRef.current = next
      playUiSound('session')
    }
    setSelectedIdx(next)
  }, [])

  /**
   * hover 选择：事件委托到结果容器（onMouseOver 基础事件冒泡，比 React onMouseEnter
   * 在 Tauri WebView2 中更可靠——enter 的 fromElement 判定偶发失效导致无声）。
   * 同一 item 内移动不重复响（moveSelection 索引判断过滤）。
   */
  const handleResultsMouseOver = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      const el = (e.target as HTMLElement).closest('.cmd-item')
      if (!el) return
      const idx = itemRefs.current.indexOf(el as HTMLDivElement)
      if (idx >= 0) moveSelection(idx)
    },
    [moveSelection],
  )

  const filtered = query
    ? items.filter(
        i =>
          i.label.toLowerCase().includes(query.toLowerCase()) ||
          i.desc?.toLowerCase().includes(query.toLowerCase()),
      )
    : items

  // 无查询时按分类分组；有查询时扁平过滤
  const grouped = useMemo(() => {
    if (query) return null
    const map = new Map<string, CommandItem[]>()
    for (const item of filtered) {
      const list = map.get(item.category)
      if (list) list.push(item)
      else map.set(item.category, [item])
    }
    return [...map.entries()]
  }, [filtered, query])

  useEffect(() => {
    if (open) {
      setQuery(initialFilter || '')
      setSelectedIdx(0)
      setTimeout(() => inputRef.current?.focus(), 50)
    }
  }, [open, initialFilter])

  useEffect(() => {
    const el = itemRefs.current[selectedIdx]
    if (el) el.scrollIntoView({ block: 'nearest' })
  }, [selectedIdx])

  const execute = useCallback(() => {
    if (filtered[selectedIdx]) {
      playUiSound('send')
      filtered[selectedIdx].action()
      onClose()
    }
  }, [filtered, selectedIdx, onClose])

  useEffect(() => {
    if (!open || placement !== 'bottom') return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose()
        return
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        moveSelection(Math.min(selectedIdxRef.current + 1, filtered.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        moveSelection(Math.max(selectedIdxRef.current - 1, 0))
        return
      }
      if (e.key === 'Enter') {
        e.preventDefault()
        execute()
        return
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [open, placement, filtered.length, execute, onClose, moveSelection])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      onClose()
      return
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      moveSelection(Math.min(selectedIdxRef.current + 1, filtered.length - 1))
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      moveSelection(Math.max(selectedIdxRef.current - 1, 0))
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      execute()
      return
    }
  }

  if (!open) return null

  const overlayCls = placement === 'bottom' ? 'cmd-overlay' : 'cmd-palette-overlay'

  return (
    <div className={overlayCls} onClick={onClose}>
      <div className="cmd-palette" onClick={e => e.stopPropagation()}>
        {placement === 'center' && (
          <div className="cmd-palette-input-wrap">
            <span className="cmd-palette-prefix">⌘</span>
            <input
              ref={inputRef}
              className="cmd-palette-input"
              placeholder={t('cmd.searchPlaceholder')}
              role="combobox"
              aria-expanded="true"
              aria-controls="cmd-palette-results"
              aria-autocomplete="list"
              aria-activedescendant={
                filtered[selectedIdx] ? `cmd-option-${filtered[selectedIdx].id}` : undefined
              }
              value={query}
              onChange={e => {
                setQuery(e.target.value)
                setSelectedIdx(0)
              }}
              onKeyDown={handleKeyDown}
              spellCheck={false}
              autoComplete="off"
            />
          </div>
        )}

        <div
          id="cmd-palette-results"
          className="cmd-palette-results"
          ref={resultsRef}
          role="listbox"
          aria-label={t('cmd.searchPlaceholder')}
          onMouseOver={handleResultsMouseOver}
          onWheel={e => {
            e.preventDefault()
            if (e.deltaY > 0) {
              setSelectedIdx(i => Math.min(i + 1, filtered.length - 1))
            } else {
              setSelectedIdx(i => Math.max(i - 1, 0))
            }
          }}
        >
          {filtered.length === 0 ? (
            <div className="cmd-palette-empty">{t('cmd.noResults')}</div>
          ) : grouped ? (
            grouped.map(([category, groupItems]) => (
              <div key={category} className="cmd-group" role="group" aria-label={category}>
                <div className="cmd-group-label" aria-hidden="true">
                  {category}
                </div>
                {groupItems.map(item => {
                  const flatIdx = filtered.indexOf(item)
                  return (
                    <div
                      key={item.id}
                      id={`cmd-option-${item.id}`}
                      ref={el => {
                        itemRefs.current[flatIdx] = el
                      }}
                      className={`cmd-item ${flatIdx === selectedIdx ? 'selected' : ''}`}
                      role="option"
                      aria-selected={flatIdx === selectedIdx}
                      onClick={() => {
                        playUiSound('send')
                        item.action()
                        onClose()
                      }}
                    >
                      {iconMap?.[item.id] && <span className="cmd-icon">{iconMap[item.id]}</span>}
                      <span className="cmd-label">{item.label}</span>
                      <span className="cmd-desc">{item.desc}</span>
                      {flatIdx === selectedIdx && <span className="cmd-arrow">↵</span>}
                    </div>
                  )
                })}
              </div>
            ))
          ) : (
            filtered.map((item, i) => (
              <div
                key={item.id}
                id={`cmd-option-${item.id}`}
                ref={el => {
                  itemRefs.current[i] = el
                }}
                className={`cmd-item ${i === selectedIdx ? 'selected' : ''}`}
                role="option"
                aria-selected={i === selectedIdx}
                onClick={() => {
                  playUiSound('send')
                  item.action()
                  onClose()
                }}
              >
                {iconMap?.[item.id] && <span className="cmd-icon">{iconMap[item.id]}</span>}
                <span className="cmd-label">{item.label}</span>
                <span className="cmd-desc">{item.desc}</span>
                {i === selectedIdx && <span className="cmd-arrow">↵</span>}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  )
}
