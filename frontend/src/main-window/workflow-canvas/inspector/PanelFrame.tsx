/**
 * PanelFrame.tsx — Inspector v2 面板外框（三段式骨架：身份头 / 摘要行 / body）。
 *
 * 设计标准 D1/S1：身份头（title + badge + 关闭）→ 摘要行（超宽省略 + title）→ 分区组 body。
 * resize 逻辑从现有 Inspector.tsx L1024-1055 原样迁入（360–720、persistWidth、指针捕获），
 * 行为零变更；Esc 关闭收敛为「焦点在面板内」时触发，不劫持画布全局键盘表
 * （CanvasPage 的 Escape 优先级链保持唯一入口，避免与 addMenu/edgeInsert 等弹层争抢）。
 */
import { useRef, type ReactNode } from 'react'
import { IconX } from '../../../ui/Icons'
import './inspector-panel.css'

/** 与现有 Inspector 一致的宽度边界（Inspector.tsx L1029-1031） */
const PANEL_MIN_WIDTH = 360
const PANEL_MAX_WIDTH = 720
/** 键盘单步（Inspector.tsx L1037：手柄在面板左缘，左键 = 加宽） */
const PANEL_KEY_STEP = 20

export interface PanelFrameProps {
  title: ReactNode
  badge?: ReactNode
  summary?: ReactNode
  width: number
  onWidthPersist: (w: number) => void
  onClose: () => void
  children: ReactNode
  /**
   * 附加到根节点的类名。T2 迁移时传 "wfc-inspector" 承接既有绝对定位骨架
   * （workflow-canvas.css:1322-1334），PanelFrame 自身只负责面板内部结构。
   */
  className?: string
}

export function PanelFrame({
  title,
  badge,
  summary,
  width,
  onWidthPersist,
  onClose,
  children,
  className,
}: PanelFrameProps) {
  const panelRef = useRef<HTMLElement>(null)
  const clampWidth = (w: number) => Math.max(PANEL_MIN_WIDTH, Math.min(PANEL_MAX_WIDTH, w))

  return (
    <aside
      ref={panelRef}
      className={className ? `wfp-panel ${className}` : 'wfp-panel'}
      style={{ width, maxWidth: '100%' }}
      onKeyDown={e => {
        // 仅焦点在面板内时响应：不与画布全局 Escape 优先级链冲突
        if (e.key === 'Escape') {
          e.preventDefault()
          onClose()
        }
      }}
    >
      <div
        className="wfp-panel__resize"
        role="separator"
        aria-orientation="vertical"
        aria-label="调整参数面板宽度"
        aria-valuemin={PANEL_MIN_WIDTH}
        aria-valuemax={PANEL_MAX_WIDTH}
        aria-valuenow={width}
        tabIndex={0}
        onKeyDown={e => {
          if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
            e.preventDefault()
            onWidthPersist(
              clampWidth(width + (e.key === 'ArrowLeft' ? PANEL_KEY_STEP : -PANEL_KEY_STEP)),
            )
          }
        }}
        onPointerDown={e => {
          e.preventDefault()
          e.currentTarget.setPointerCapture(e.pointerId)
        }}
        onPointerMove={e => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) {
            const right = panelRef.current?.getBoundingClientRect().right ?? window.innerWidth
            onWidthPersist(clampWidth(right - e.clientX))
          }
        }}
        onPointerUp={e => {
          if (e.currentTarget.hasPointerCapture(e.pointerId))
            e.currentTarget.releasePointerCapture(e.pointerId)
        }}
      />
      <div className="wfp-panel__head">
        <span className="wfp-panel__title">{title}</span>
        {badge}
        <button
          type="button"
          className="wfp-panel__close"
          onClick={onClose}
          title="关闭（Esc）"
          aria-label="关闭（Esc）"
        >
          <IconX size={14} />
        </button>
      </div>
      {summary !== undefined && summary !== null && (
        <div
          className="wfp-panel__summary"
          title={typeof summary === 'string' ? summary : undefined}
        >
          {summary}
        </div>
      )}
      <div className="wfp-panel__body">{children}</div>
    </aside>
  )
}
