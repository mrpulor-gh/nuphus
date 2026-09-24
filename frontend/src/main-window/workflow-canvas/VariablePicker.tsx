import { useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { isVariableName, type VariableCatalog } from './variableCatalog'
import './variable-picker.css'

export interface VariablePickerProps {
  value: string
  onChange: (value: string) => void
  mode: 'capture' | 'reference'
  catalog: VariableCatalog
  readOnly?: boolean
  onConfigureInput?: (name: string) => void
  label?: string
}

export function VariablePicker({
  value,
  onChange,
  mode,
  catalog,
  readOnly,
  onConfigureInput,
  label,
}: VariablePickerProps) {
  const id = useId()
  const input = useRef<HTMLInputElement>(null)
  const popup = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const [searching, setSearching] = useState(false)
  const [position, setPosition] = useState({ left: 0, top: 0, width: 0, maxHeight: 240 })
  const entries = mode === 'capture' ? catalog.captures : catalog.references
  const matches = entries.filter(
    entry =>
      !searching ||
      `${entry.name} ${entry.sourceLabel}`.toLowerCase().includes(value.toLowerCase()),
  )
  const exact = entries.find(entry => entry.name === value)
  const canCreate = mode === 'capture' && !!value && isVariableName(value) && !exact
  const count = matches.length + (canCreate ? 1 : 0)
  const choose = (index: number) => {
    const name = matches[index]?.name ?? (canCreate ? value : undefined)
    if (name !== undefined) onChange(name)
    setOpen(false)
  }
  useEffect(() => {
    if (!open) return
    const update = () => {
      const rect = input.current?.getBoundingClientRect()
      if (!rect) return
      const width = Math.min(Math.max(rect.width, 260), window.innerWidth - 16)
      const below = window.innerHeight - rect.bottom - 12
      const above = rect.top - 12
      const up = below < 160 && above > below
      const maxHeight = Math.max(64, Math.min(240, up ? above : below))
      setPosition({
        left: Math.max(8, Math.min(rect.left, window.innerWidth - width - 8)),
        top: up ? rect.top - maxHeight - 4 : rect.bottom + 4,
        width,
        maxHeight,
      })
    }
    const outside = (event: PointerEvent) => {
      if (
        !input.current?.contains(event.target as Node) &&
        !popup.current?.contains(event.target as Node)
      )
        setOpen(false)
    }
    update()
    window.addEventListener('resize', update)
    window.addEventListener('scroll', update, true)
    document.addEventListener('pointerdown', outside)
    return () => {
      window.removeEventListener('resize', update)
      window.removeEventListener('scroll', update, true)
      document.removeEventListener('pointerdown', outside)
    }
  }, [open])
  useEffect(() => {
    if (open) document.getElementById(`${id}-${active}`)?.scrollIntoView?.({ block: 'nearest' })
  }, [active, id, open])
  return (
    <div className="wfc-variable-picker">
      <input
        ref={input}
        className="wfc-input wfc-input--mono"
        value={value}
        readOnly={readOnly}
        role="combobox"
        aria-label={label ?? (mode === 'capture' ? '保存输出到变量' : '引用已有变量')}
        aria-expanded={open}
        aria-controls={open ? `${id}-list` : undefined}
        aria-autocomplete="list"
        aria-activedescendant={
          open && count > 0 ? `${id}-${Math.min(active, count - 1)}` : undefined
        }
        placeholder={
          mode === 'capture' ? '选择或输入新变量名；留空不保存' : '选择已有变量，也可手动输入表达式'
        }
        onClick={() => {
          if (!readOnly) {
            setSearching(false)
            const selected = entries.findIndex(entry => entry.name === value)
            setActive(selected < 0 ? 0 : selected)
            setOpen(true)
          }
        }}
        onChange={event => {
          onChange(event.target.value)
          setSearching(true)
          setActive(0)
          setOpen(true)
        }}
        onBlur={event => {
          if (!popup.current?.contains(event.relatedTarget as Node)) setOpen(false)
        }}
        onKeyDown={event => {
          if (readOnly || event.nativeEvent.isComposing) return
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault()
            setOpen(true)
            setActive(current =>
              Math.max(0, Math.min(count - 1, current + (event.key === 'ArrowDown' ? 1 : -1))),
            )
          } else if (event.key === 'Enter' && open) {
            event.preventDefault()
            choose(active)
          } else if (event.key === 'Escape') {
            event.stopPropagation()
            setOpen(false)
          }
        }}
      />
      {mode === 'capture' && exact && <small>将更新已有变量 · {exact.sourceLabel}</small>}
      {mode === 'capture' && value && !isVariableName(value) && (
        <small role="alert">变量名以字母或下划线开头，可包含数字和点。</small>
      )}
      {mode === 'reference' && exact?.maybeUnset && (
        <small>可能未赋值，请检查分支、循环或输入默认值。</small>
      )}
      {mode === 'reference' && value && !exact && (
        <small>
          未找到来源；保留手动表达式。
          {onConfigureInput && isVariableName(value) && (
            <button type="button" onClick={() => onConfigureInput(value.replace(/^inputs\./, ''))}>
              设置工作流输入
            </button>
          )}
        </small>
      )}
      {open &&
        !readOnly &&
        createPortal(
          <div
            ref={popup}
            id={`${id}-list`}
            role="listbox"
            aria-label="变量候选"
            className="wfc-variable-menu"
            style={position}
          >
            {matches.map((entry, index) => (
              <button
                key={entry.name}
                type="button"
                role="option"
                id={`${id}-${index}`}
                aria-selected={index === active}
                onMouseDown={event => event.preventDefault()}
                onClick={() => choose(index)}
              >
                <span title={entry.name}>{entry.name}</span>
                <small>
                  {entry.sourceLabel}
                  {entry.maybeUnset ? ' · 可能未赋值' : ''}
                </small>
              </button>
            ))}
            {canCreate && (
              <button
                type="button"
                role="option"
                id={`${id}-${matches.length}`}
                aria-selected={active === matches.length}
                onMouseDown={event => event.preventDefault()}
                onClick={() => choose(matches.length)}
              >
                ＋新建变量 {value}
              </button>
            )}
            {count === 0 && <small>没有匹配的变量，可继续手动输入。</small>}
          </div>,
          document.body,
        )}
    </div>
  )
}
