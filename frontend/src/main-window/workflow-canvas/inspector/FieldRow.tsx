/**
 * FieldRow.tsx — Inspector v2 字段行（S1/D1/D7）。
 *
 * 结构：label 行（13px/500/--fg-1 + 必填 accent `*` + help 图标 hover title）
 * → control → hint（11px/--fg-3）→ error（11px/--error 带警示图标，**替换 hint 位不双显**）。
 * 布局：默认两列 grid（label 132px 定宽 + control 弹性列，label 列省略号 + title，
 * control 列永远优先保宽）；stack prop 或面板 <520px 时单列堆叠。
 * 堆叠阈值用容器查询实现（查询容器 = .wfp-panel，见 inspector-panel.css），
 * 不靠 JS 传宽——面板宽度变化零重渲染。
 * fieldPath 非空时根节点带 data-field-path / data-draft-field（草稿定位钩子）。
 */
import type { ReactNode } from 'react'
import { IconAlert, IconHelp } from './icons'
import './inspector-panel.css'

export interface FieldRowProps {
  label: ReactNode
  required?: boolean
  /** 帮助说明：渲染为 help 图标，hover 出原生 title */
  help?: string
  hint?: ReactNode
  /** 错误：与 hint 互斥，出现即替换 hint 位 */
  error?: ReactNode
  /** 强制单列堆叠（窄控件 / 复合控件如 Segmented、KeyValueRows） */
  stack?: boolean
  /**
   * 稳定字段路径（editorFields.editorFieldId 同规则，如 '/do/with/ratio'）。
   * 非空时根节点渲染 data-field-path 与 data-draft-field——草稿存储按路径
   * 刷新脏条目、focusEditorField 按路径定位并展开外层 details，
   * 迁移到 v2 结构后这两个属性是草稿系统不静默死亡的前提。
   */
  fieldPath?: string
  children: ReactNode
}

export function FieldRow({
  label,
  required,
  help,
  hint,
  error,
  stack,
  fieldPath,
  children,
}: FieldRowProps) {
  return (
    <div
      className={stack ? 'wfp-frow wfp-frow--stack' : 'wfp-frow'}
      data-field-path={fieldPath}
      data-draft-field={fieldPath}
    >
      <div className="wfp-frow__label">
        <span className="wfp-frow__text" title={typeof label === 'string' ? label : undefined}>
          {label}
        </span>
        {required && (
          <span className="wfp-frow__req" aria-hidden="true">
            *
          </span>
        )}
        {help && (
          <span className="wfp-frow__help" role="img" aria-label={help} title={help}>
            <IconHelp size={12} />
          </span>
        )}
      </div>
      <div className="wfp-frow__main">
        {children}
        {error ? (
          <div className="wfp-frow__err">
            <IconAlert size={11} />
            {error}
          </div>
        ) : hint ? (
          <div className="wfp-frow__hint">{hint}</div>
        ) : null}
      </div>
    </div>
  )
}
