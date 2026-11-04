/**
 * controls.tsx — Inspector v2 标准控件族（S3 控件选用矩阵 + S4 状态矩阵）。
 *
 * 单文件多导出；每个控件四态齐全（default/hover/focus-visible/active），
 * 焦点环统一 box-shadow 0 0 0 2px var(--accent-glow)（不挤布局），
 * 动效仅 var(--dur-fast)，图标一律 inline SVG stroke=currentColor（见 icons.tsx）。
 *
 * 编辑提交语义与现有 Inspector 字段一致：本地 draft 跟随 value，blur / Enter 才 onCommit，
 * 空值 = 默认（由调用方 hint 说明，控件不做隐式改写）。
 */
import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { IconAlert, IconBraces, IconCheck, IconPlus, IconTrash } from './icons'
import './inspector-panel.css'

/* ── Segmented（枚举 ≤4；禁用 select） ── */

export interface SegmentedOption<T extends string | number> {
  value: T
  label: string
  icon?: ReactNode
}

export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  disabled,
  label,
}: {
  options: SegmentedOption<T>[]
  value: T
  onChange: (value: T) => void
  disabled?: boolean
  /** radiogroup 可访问名（T2 传字段 label）；缺省则无名，仅测试/占位用 */
  label?: string
}) {
  return (
    <div className="wfp-seg" role="radiogroup" aria-label={label}>
      {options.map(option => {
        const active = option.value === value
        return (
          <button
            key={String(option.value)}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={disabled}
            className={active ? 'wfp-seg__item is-active' : 'wfp-seg__item'}
            onClick={() => onChange(option.value)}
          >
            {option.icon}
            {option.label}
          </button>
        )
      })}
    </div>
  )
}

/* ── Switch（布尔；禁用裸 checkbox） ── */

export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean
  onChange: (checked: boolean) => void
  disabled?: boolean
  label?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      className={checked ? 'wfp-sw is-on' : 'wfp-sw'}
      onClick={() => onChange(!checked)}
    >
      <span className="wfp-sw__track" aria-hidden="true" />
      {label && <span className="wfp-sw__label">{label}</span>}
    </button>
  )
}

/* ── NumberField（整数/小数；单位后缀；空 = 默认进 hint） ── */

export function NumberField({
  value,
  onCommit,
  min,
  max,
  integer,
  unit,
  disabled,
  invalid,
  label,
  onDraftChange,
}: {
  value: string
  onCommit: (value: string) => void
  min?: number
  max?: number
  integer?: boolean
  unit?: string
  disabled?: boolean
  /** 调用方校验不通过（如越界/非整数）：error 描边，具体文案由调用方 hint/error 位给 */
  invalid?: boolean
  /** 可访问名（T2a 迁移新增）：FieldRow 的 label 是 div、不与 input 关联，
   *  数字输入必须显式命名，否则读屏与 getByRole(name) 都取不到字段名 */
  label?: string
  /** 草稿通报（T2b 新增）：每次输入回调，供调用方把文本同步进画布草稿存储——
   *  否则「输入未失焦」期间 store.dirty 不置位，flush/保存门槛看不到这次编辑 */
  onDraftChange?: (value: string) => void
}) {
  const [draft, setDraft] = useState(value)
  useEffect(() => {
    setDraft(value)
  }, [value])
  const commit = () => {
    if (draft !== value) onCommit(draft)
  }
  /** 步进（S3）：整数 ±1、小数 ±0.1；空值从 min（缺省 0）起；结果钳制在 [min,max] */
  const stepBy = (direction: 1 | -1) => {
    const delta = (integer ? 1 : 0.1) * direction
    const base = draft.trim() === '' ? (min ?? 0) : Number(draft)
    if (!Number.isFinite(base)) return
    let next = base + delta
    if (min !== undefined) next = Math.max(min, next)
    if (max !== undefined) next = Math.min(max, next)
    const text = integer ? String(Math.round(next)) : String(Math.round(next * 1e6) / 1e6)
    setDraft(text)
    onCommit(text)
  }
  return (
    <div className={invalid ? 'wfp-num wfp-num--invalid' : 'wfp-num'}>
      <input
        className="wfp-ctl wfp-num__input"
        type="text"
        inputMode={integer ? 'numeric' : 'decimal'}
        value={draft}
        disabled={disabled}
        aria-label={label}
        aria-invalid={invalid || undefined}
        onChange={e => {
          setDraft(e.target.value)
          onDraftChange?.(e.target.value)
        }}
        onBlur={commit}
        onKeyDown={e => {
          if (e.key === 'Enter') {
            e.preventDefault()
            commit()
          } else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
            e.preventDefault()
            stepBy(e.key === 'ArrowUp' ? 1 : -1)
          }
        }}
      />
      {unit && <span className="wfp-num__unit">{unit}</span>}
    </div>
  )
}

/* ── PathField（路径/产物引用 + 插变量） ── */

export function PathField({
  value,
  onCommit,
  disabled,
  references = true,
  onInsertVariable,
}: {
  value: string
  onCommit: (value: string) => void
  disabled?: boolean
  /** 本字段可引用上游变量时才显示「插变量」按钮 */
  references?: boolean
  /** 插入动作回调（T2 接入 VariablePicker）；缺省则不渲染按钮，不留死按钮 */
  onInsertVariable?: () => void
}) {
  const [draft, setDraft] = useState(value)
  useEffect(() => {
    setDraft(value)
  }, [value])
  const commit = () => {
    if (draft !== value) onCommit(draft)
  }
  return (
    <div className="wfp-path">
      <input
        className="wfp-ctl wfp-path__input"
        type="text"
        value={draft}
        disabled={disabled}
        placeholder="相对路径，或 {{变量}} 引用上游产物"
        onChange={e => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={e => {
          if (e.key === 'Enter') {
            e.preventDefault()
            commit()
          }
        }}
      />
      {references && onInsertVariable && (
        <button
          type="button"
          className="wfp-varbtn"
          disabled={disabled}
          title="插入变量引用"
          aria-label="插入变量引用"
          onClick={onInsertVariable}
        >
          <IconBraces size={11} />
          插变量
        </button>
      )}
    </div>
  )
}

/* ── JsonField（结构化逃生舱：只读预览 + 格式校验徽标；editable 逃生口留给 T2） ── */

export function JsonField({
  value,
  onCommit,
  disabled,
  editable = false,
}: {
  value: string
  onCommit: (value: string) => void
  disabled?: boolean
  /** true = 等宽 textarea 编辑（blur 提交，行为对齐调用方现有 JsonField）；默认只读预览 */
  editable?: boolean
}) {
  const valid = useMemo(() => {
    if (value.trim() === '') return true
    try {
      JSON.parse(value)
      return true
    } catch {
      return false
    }
  }, [value])
  const [draft, setDraft] = useState(value)
  useEffect(() => {
    setDraft(value)
  }, [value])
  const commit = () => {
    if (draft !== value) onCommit(draft)
  }
  return (
    <div className="wfp-json">
      <div className="wfp-json__bar">
        <span>JSON</span>
        <span
          className={
            valid ? 'wfp-json__badge wfp-json__badge--ok' : 'wfp-json__badge wfp-json__badge--err'
          }
        >
          {valid ? (
            <>
              <IconCheck size={10} />
              格式正确
            </>
          ) : (
            <>
              <IconAlert size={10} />
              格式错误
            </>
          )}
        </span>
      </div>
      {editable ? (
        <textarea
          className="wfp-json__input"
          value={draft}
          disabled={disabled}
          spellCheck={false}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={e => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
              e.preventDefault()
              commit()
            }
          }}
        />
      ) : (
        <pre className="wfp-json__pre">{value}</pre>
      )}
    </div>
  )
}

/* ── KeyValueRows（headers/params 键值对；可增删行） ── */

export interface KeyValueRow {
  key: string
  value: string
}

export function KeyValueRows({
  rows,
  onChange,
  keyPlaceholder = '键',
  valuePlaceholder = '值',
  disabled,
}: {
  rows: KeyValueRow[]
  onChange: (rows: KeyValueRow[]) => void
  keyPlaceholder?: string
  valuePlaceholder?: string
  disabled?: boolean
}) {
  const patch = (index: number, next: Partial<KeyValueRow>) =>
    onChange(rows.map((row, i) => (i === index ? { ...row, ...next } : row)))
  return (
    <div className="wfp-kv">
      {rows.map((row, index) => (
        <div className="wfp-kv__row" key={index}>
          <input
            className="wfp-ctl"
            type="text"
            value={row.key}
            disabled={disabled}
            placeholder={keyPlaceholder}
            onChange={e => patch(index, { key: e.target.value })}
          />
          <input
            className="wfp-ctl"
            type="text"
            value={row.value}
            disabled={disabled}
            placeholder={valuePlaceholder}
            onChange={e => patch(index, { value: e.target.value })}
          />
          <button
            type="button"
            className="wfp-kv__del"
            disabled={disabled}
            title="删除该项"
            aria-label="删除该项"
            onClick={() => onChange(rows.filter((_, i) => i !== index))}
          >
            <IconTrash size={11} />
          </button>
        </div>
      ))}
      <button
        type="button"
        className="wfp-kv__add"
        disabled={disabled}
        onClick={() => onChange([...rows, { key: '', value: '' }])}
      >
        <IconPlus size={10} />
        添加一项
      </button>
    </div>
  )
}

/* ── ConditionRow（if 条件行：左值 / 运算符徽标 / 右值） ── */

export function ConditionRow({
  left,
  op,
  right,
}: {
  left: ReactNode
  op: ReactNode
  right: ReactNode
}) {
  return (
    <div className="wfp-cond">
      {left}
      <span className="wfp-cond__op">{op}</span>
      {right}
    </div>
  )
}
