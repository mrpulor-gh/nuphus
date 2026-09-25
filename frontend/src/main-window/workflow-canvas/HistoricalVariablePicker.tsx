import { useContext, useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { WorkflowTraceContext } from './WorkflowTraceContext'
import { ExecutionTraceViewer } from './ExecutionTraceViewer'
import type { VariableCatalog } from './variableCatalog'
import { useTraceText } from './traceMessages'

export function HistoricalVariablePicker({
  catalog,
  onSelect,
}: {
  catalog: VariableCatalog
  onSelect: (expression: string) => void
}) {
  const context = useContext(WorkflowTraceContext)
  const text = useTraceText()
  const id = useId()
  const [open, setOpen] = useState(false)
  const button = useRef<HTMLButtonElement>(null)
  const dialog = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const trigger = button.current
    dialog.current?.focus()
    return () => {
      trigger?.focus()
    }
  }, [open])
  if (!context?.workflowId) return null
  return (
    <>
      <button type="button" ref={button} className="wfc-trace-open" onClick={() => setOpen(true)}>
        {text('history')}
      </button>
      {open &&
        createPortal(
          <div
            className="wfc-trace-mask"
            onMouseDown={event => {
              if (event.target === event.currentTarget) setOpen(false)
            }}
          >
            <div
              ref={dialog}
              tabIndex={-1}
              className="wfc-trace-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby={id}
              onKeyDown={event => {
                event.stopPropagation()
                if (event.key === 'Escape') {
                  event.preventDefault()
                  setOpen(false)
                }
                if (event.key === 'Tab') {
                  const elements = Array.from(
                    dialog.current?.querySelectorAll<HTMLElement>(
                      'button:not(:disabled), select, input, [tabindex="0"]',
                    ) ?? [],
                  )
                  const first = elements[0],
                    last = elements[elements.length - 1]
                  if (
                    event.shiftKey &&
                    (document.activeElement === first || document.activeElement === dialog.current)
                  ) {
                    event.preventDefault()
                    last?.focus()
                  } else if (!event.shiftKey && document.activeElement === last) {
                    event.preventDefault()
                    first?.focus()
                  }
                }
              }}
            >
              <div className="wfc-trace-dialog-head">
                <h3 id={id}>{text('history')}</h3>
                <button type="button" onClick={() => setOpen(false)}>
                  {text('close')}
                </button>
              </div>
              <ExecutionTraceViewer
                catalog={catalog}
                onSelectReference={expression => {
                  onSelect(expression)
                  setOpen(false)
                }}
              />
            </div>
          </div>,
          document.body,
        )}
    </>
  )
}
