import { useLayoutEffect, useState, type ReactNode, type RefObject } from 'react'
import { useLanguage } from '../../locales'

/** Move (never duplicate) lower-priority actions into the existing More menu. */
export function ToolbarOverflow({
  actions,
  children,
  menuRef,
}: {
  actions: ReactNode[]
  children: ReactNode
  menuRef: RefObject<HTMLDetailsElement>
}) {
  const { t } = useLanguage()
  const [hiddenCount, setHiddenCount] = useState(0)
  useLayoutEffect(() => {
    const toolbar = menuRef.current?.closest('.wfc-toolbar')
    if (!toolbar) return
    const measure = () => {
      const width = toolbar.getBoundingClientRect().width
      if (width > 0) setHiddenCount(width >= 1200 ? 0 : width >= 1080 ? 1 : width >= 960 ? 2 : 4)
    }
    measure()
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    observer?.observe(toolbar)
    window.addEventListener('resize', measure)
    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', measure)
    }
  }, [menuRef])
  return (
    <>
      {actions.slice(hiddenCount)}
      <details
        className="wfc-more"
        ref={menuRef}
        onKeyDown={e => {
          if (e.key === 'Escape') {
            e.currentTarget.open = false
            e.currentTarget.querySelector('summary')?.focus()
          }
        }}
      >
        <summary className="wfc-btn">{t('workflowEditor.toolbar.more')}</summary>
        <div
          className="wfc-more-menu"
          onClick={e => {
            if ((e.target as HTMLElement).closest('button') && menuRef.current)
              menuRef.current.open = false
          }}
        >
          {actions.slice(0, hiddenCount)}
          {children}
        </div>
      </details>
    </>
  )
}
