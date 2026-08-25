import { ReactNode, ReactElement, useEffect, useRef, useState } from 'react'
import { IconX } from '../../ui/Icons'
import { IconButton } from '../../ui/Button'

interface CompactModalProps {
  open: boolean
  onClose: () => void
  title: string
  icon?: ReactElement
  size?: 'sm' | 'md' | 'lg' | 'xl' | 'auto'
  /** 追加到 modal 卡片的类（如 compact-modal--fit 高度自适应） */
  className?: string
  /** 固定底部操作区 — 渲染在滚动区之外，长内容时主操作始终可见 */
  footer?: ReactNode
  children: ReactNode
}

/** 出场动画时长（与 components.css 中 compactModalOut 一致） */
const EXIT_MS = 150

export function CompactModal({
  open,
  onClose,
  title,
  icon,
  size = 'auto',
  className,
  footer,
  children,
}: CompactModalProps) {
  const [closing, setClosing] = useState(false)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // 父组件把 open 置 false 时重置 closing 状态
  useEffect(() => {
    if (!open) setClosing(false)
  }, [open])

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current)
    },
    [],
  )

  if (!open) return null

  const requestClose = () => {
    if (closing) return
    setClosing(true)
    timerRef.current = setTimeout(onClose, EXIT_MS)
  }

  const sizeClass = size === 'auto' ? 'compact-modal--auto' : `compact-modal--${size}`

  return (
    <div
      className={closing ? 'compact-overlay compact-overlay--closing' : 'compact-overlay'}
      onClick={requestClose}
    >
      <div
        className={`compact-modal ${sizeClass}${className ? ` ${className}` : ''}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={e => e.stopPropagation()}
      >
        <div className="compact-header">
          {icon && <div className="compact-header-icon">{icon}</div>}
          <span className="compact-header-title">{title}</span>
          <IconButton variant="compact-header-close" label="关闭" onClick={requestClose}>
            <IconX size={14} />
          </IconButton>
        </div>
        <div className="compact-divider" />
        <div className="compact-body">{children}</div>
        {footer && (
          <>
            <div className="compact-divider" />
            <div className="compact-footer">{footer}</div>
          </>
        )}
      </div>
    </div>
  )
}
