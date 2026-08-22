import type { ReactNode } from 'react'

/* ═══════════════════════════════════════════
   PageLayout — 页面骨架组件（Section / FormRow）

   控制面板页面的统一骨架：
   - Section  分区容器（标题 + 描述 + 操作区 + 内容）
   - FormRow  设置行（label 左 + control 右 / stacked 纵向）

   样式定义：styles/page-layout.css
   设计规范：docs/DESIGN-RATIONALE.md
   ═══════════════════════════════════════════ */

interface SectionProps {
  title?: ReactNode
  description?: ReactNode
  /** 右侧操作区（按钮/链接等） */
  actions?: ReactNode
  /** 内容是否去掉内边距（列表/表格场景） */
  flush?: boolean
  className?: string
  children: ReactNode
}

export function Section({ title, description, actions, flush, className, children }: SectionProps) {
  const hasHeader = title || description || actions
  return (
    <section className={['section', className].filter(Boolean).join(' ')}>
      {hasHeader && (
        <header className="section-header">
          <div className="section-header-text">
            {title && <h3 className="section-title">{title}</h3>}
            {description && <p className="section-desc">{description}</p>}
          </div>
          {actions && <div className="section-actions">{actions}</div>}
        </header>
      )}
      <div className={flush ? 'section-body section-body--flush' : 'section-body'}>{children}</div>
    </section>
  )
}

interface FormRowProps {
  label: ReactNode
  /** 辅助说明（label 下方弱文本） */
  hint?: ReactNode
  /** 右侧控件（input/switch/select/button 等） */
  control?: ReactNode
  /** 纵向布局：label 在上，control 占满整行 */
  stacked?: boolean
  className?: string
  children?: ReactNode
}

export function FormRow({ label, hint, control, stacked, className, children }: FormRowProps) {
  const ctl = control ?? children
  return (
    <div
      className={['form-row', stacked && 'form-row--stacked', className].filter(Boolean).join(' ')}
    >
      <div className="form-row-info">
        <div className="form-row-label">{label}</div>
        {hint && <div className="form-row-hint">{hint}</div>}
      </div>
      {ctl && <div className="form-row-control">{ctl}</div>}
    </div>
  )
}
