import { useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { useLanguage } from '../../locales'
import { isVariableName, type VariableCatalog } from './variableCatalog'
import { variableSourceLabel } from './presentation'
import { HistoricalVariablePicker } from './HistoricalVariablePicker'
import { parseFieldReference } from './fieldReferences'
import './variable-picker.css'

export interface VariablePickerProps {
  value: string
  onChange: (value: string) => void
  mode: 'capture' | 'reference'
  catalog: VariableCatalog
  readOnly?: boolean
  onConfigureInput?: (name: string) => void
  label?: string
  /** An explicit historical field choice; text insertion need not treat typing as selection. */
  onSelectReference?: (expression: string) => void
}

export function VariablePicker({
  value,
  onChange,
  mode,
  catalog,
  readOnly,
  onConfigureInput,
  label,
  onSelectReference,
}: VariablePickerProps) {
  const { t, lang } = useLanguage()
  const id = useId()
  const input = useRef<HTMLInputElement>(null)
  const popup = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const [searching, setSearching] = useState(false)
  const [position, setPosition] = useState({ left: 0, top: 0, width: 0, maxHeight: 240 })
  const entries = (mode === 'capture' ? catalog.captures : catalog.references).map(entry => ({
    ...entry,
    displaySource: variableSourceLabel(entry, t),
  }))
  const matches = entries.filter(
    entry =>
      !searching ||
      `${entry.name} ${entry.displaySource}`.toLowerCase().includes(value.toLowerCase()),
  )
  const exact = entries.find(entry => entry.name === value)
  const parsed = mode === 'reference' ? parseFieldReference(value) : null
  const referenceSource =
    exact ??
    (parsed &&
      entries.find(entry =>
        entry.source === 'input'
          ? parsed.root === 'inputs' && parsed.segments[0] === entry.name.slice(7)
          : entry.name === parsed.root,
      ))
  const canCreate = mode === 'capture' && !!value && isVariableName(value) && !exact
  const count = matches.length + (canCreate ? 1 : 0)
  const activeIndex = Math.max(0, Math.min(active, count - 1))
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
    if (open)
      document.getElementById(`${id}-${activeIndex}`)?.scrollIntoView?.({ block: 'nearest' })
  }, [activeIndex, id, open, lang])
  return (
    <div className="wfc-variable-picker">
      <input
        ref={input}
        className="wfc-input wfc-input--mono"
        value={value}
        readOnly={readOnly}
        role="combobox"
        aria-label={label ?? t(`workflowCanvas.variable.${mode}`)}
        aria-expanded={open}
        aria-controls={open ? `${id}-list` : undefined}
        aria-autocomplete="list"
        aria-activedescendant={open && count > 0 ? `${id}-${activeIndex}` : undefined}
        placeholder={t(`workflowCanvas.variable.${mode}Placeholder`)}
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
            setActive(
              Math.max(0, Math.min(count - 1, activeIndex + (event.key === 'ArrowDown' ? 1 : -1))),
            )
          } else if (event.key === 'Enter' && open) {
            event.preventDefault()
            choose(activeIndex)
          } else if (event.key === 'Escape') {
            event.stopPropagation()
            setOpen(false)
          }
        }}
      />
      {mode === 'capture' && exact && (
        <small>{t('workflowCanvas.variable.update', exact.displaySource)}</small>
      )}
      {mode === 'capture' && value && !isVariableName(value) && (
        <small role="alert">{t('workflowCanvas.variable.invalid')}</small>
      )}
      {mode === 'reference' && referenceSource?.maybeUnset && (
        <small>{t('workflowCanvas.variable.maybeUnsetHint')}</small>
      )}
      {mode === 'reference' && value && !referenceSource && (
        <small>
          {t('workflowCanvas.variable.missing')}
          {onConfigureInput && isVariableName(value) && (
            <button type="button" onClick={() => onConfigureInput(value.replace(/^inputs\./, ''))}>
              {t('workflowCanvas.variable.configure')}
            </button>
          )}
        </small>
      )}
      {mode === 'reference' && !readOnly && (
        <HistoricalVariablePicker
          catalog={catalog}
          onSelect={expression => {
            setOpen(false)
            ;(onSelectReference ?? onChange)(expression)
          }}
        />
      )}
      {open &&
        !readOnly &&
        createPortal(
          <div
            ref={popup}
            id={`${id}-list`}
            role="listbox"
            aria-label={t('workflowCanvas.variable.candidates')}
            className="wfc-variable-menu"
            style={position}
          >
            {matches.map((entry, index) => (
              <button
                key={entry.name}
                type="button"
                role="option"
                id={`${id}-${index}`}
                aria-selected={index === activeIndex}
                onMouseDown={event => event.preventDefault()}
                onClick={() => choose(index)}
              >
                <span className="wfc-variable-name-row">
                  <span className="wfc-variable-label">{t('workflowCanvas.variable.name')}:</span>
                  <span className="wfc-variable-name" title={entry.name}>
                    {entry.name}
                  </span>
                </span>
                <span className="wfc-variable-source-row" title={entry.displaySource}>
                  <span className="wfc-variable-label">{t('workflowCanvas.variable.source')}:</span>
                  <span className="wfc-variable-source">{entry.displaySource}</span>
                </span>
                {entry.maybeUnset && <small>{t('workflowCanvas.variable.maybeUnset')}</small>}
              </button>
            ))}
            {canCreate && (
              <button
                type="button"
                role="option"
                id={`${id}-${matches.length}`}
                aria-selected={activeIndex === matches.length}
                onMouseDown={event => event.preventDefault()}
                onClick={() => choose(matches.length)}
              >
                {t('workflowCanvas.variable.create', value)}
              </button>
            )}
            {count === 0 && <small>{t('workflowCanvas.variable.empty')}</small>}
          </div>,
          document.body,
        )}
    </div>
  )
}
