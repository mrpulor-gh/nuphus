import { useState, useEffect, useRef, useCallback, useMemo, type ReactNode } from 'react'
import { useLanguage } from '../locales'
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
        setSelectedIdx(i => Math.min(i + 1, filtered.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSelectedIdx(i => Math.max(i - 1, 0))
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
  }, [open, placement, filtered.length, execute, onClose])

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      onClose()
      return
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setSelectedIdx(i => Math.min(i + 1, filtered.length - 1))
      return
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault()
      setSelectedIdx(i => Math.max(i - 1, 0))
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
          className="cmd-palette-results"
          ref={resultsRef}
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
              <div key={category} className="cmd-group">
                <div className="cmd-group-label">{category}</div>
                {groupItems.map(item => {
                  const flatIdx = filtered.indexOf(item)
                  return (
                    <div
                      key={item.id}
                      ref={el => {
                        itemRefs.current[flatIdx] = el
                      }}
                      className={`cmd-item ${flatIdx === selectedIdx ? 'selected' : ''}`}
                      onClick={() => {
                        item.action()
                        onClose()
                      }}
                      onMouseEnter={() => setSelectedIdx(flatIdx)}
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
                ref={el => {
                  itemRefs.current[i] = el
                }}
                className={`cmd-item ${i === selectedIdx ? 'selected' : ''}`}
                onClick={() => {
                  item.action()
                  onClose()
                }}
                onMouseEnter={() => setSelectedIdx(i)}
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
