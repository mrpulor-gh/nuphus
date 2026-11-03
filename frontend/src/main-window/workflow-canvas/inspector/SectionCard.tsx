/**
 * SectionCard.tsx — Inspector v2 分区卡（S1）。
 *
 * 结构：标题行（12px/600/--fg-2 + 可选 14px 线性图标 + 右侧 flag 微标 10px mono --fg-4）
 * → 1px --line-1 顶线 → 字段组（gap 12px）。分区之间靠容器边界（1px 线 + 圆角卡）分隔，
 * 禁止裸文字 div 当标题。
 */
import type { ReactNode } from 'react'
import './inspector-panel.css'

export interface SectionCardProps {
  title: string
  icon?: ReactNode
  /** 右侧微标（schema 路径 / 状态摘要），等宽 10px */
  flag?: string
  children: ReactNode
}

export function SectionCard({ title, icon, flag, children }: SectionCardProps) {
  return (
    <section className="wfp-sec">
      <div className="wfp-sec__head">
        {icon && <span className="wfp-sec__icon">{icon}</span>}
        <span className="wfp-sec__title">{title}</span>
        {flag && <span className="wfp-sec__flag">{flag}</span>}
      </div>
      <div className="wfp-sec__body">{children}</div>
    </section>
  )
}
