/**
 * RowCtxEditor — 模型卡片内的 Context Window 展示 + 就地编辑。
 *
 *  paradigm：badge（只读）↔ 行内输入（编辑）互换，与卡片其它能力 chip 的
 * 「点击即改」同构：Enter / blur 提交，Escape 取消。未知 ctx 显示「—」
 * （不用裸 `?`：那个符号在设置页里已被「帮助」占用，语义会歧义）。
 */
import { IconEdit3 } from '../../ui/Icons'

type TFunc = (key: string, ...args: string[]) => string

const tr = (t: TFunc, key: string, fallback: string): string => {
  const v = t(key)
  return v === key ? fallback : v
}

/** 上下文窗口格式化：1_000_000 → 1M，128_000 → 128K，32_768 → 33K；未知返回空串 */
export function formatContextWindow(n?: number): string {
  if (!n || n <= 0) return ''
  if (n >= 1_000_000) {
    const m = n / 1_000_000
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M`
  }
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`
  return `${n}`
}

export function RowCtxEditor({
  ctx,
  isEditing,
  value,
  onValueChange,
  onStart,
  onCommit,
  onCancel,
  t,
}: {
  ctx?: number
  isEditing: boolean
  value: string
  onValueChange: (v: string) => void
  onStart: () => void
  onCommit: (raw: string) => void
  onCancel: () => void
  t: TFunc
}) {
  const ctxCapTitle = tr(t, 'models.ctxCap', '上下文容量')
  const editTitle = tr(t, 'models.editContext', '修改上下文容量')
  if (isEditing) {
    return (
      <span className="ctx-inline-wrap" onClick={e => e.stopPropagation()}>
        <input
          autoFocus
          type="number"
          className="ctx-inline-input input-num"
          min={0.1}
          max={10000}
          step={0.001}
          value={value}
          onChange={e => onValueChange(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.stopPropagation()
              onCommit(value)
            } else if (e.key === 'Escape') {
              e.stopPropagation()
              onCancel()
            }
          }}
          onBlur={() => onCommit(value)}
        />
        <span className="ctx-unit">K</span>
      </span>
    )
  }
  return (
    <>
      {ctx && ctx > 0 ? (
        <span className="model-badge model-badge--ctx" title={ctxCapTitle}>
          {formatContextWindow(ctx)}
        </span>
      ) : (
        <span className="model-badge model-badge--ctx model-badge--ctx-unknown" title={ctxCapTitle}>
          —
        </span>
      )}
      <button
        type="button"
        className="icon-btn-ghost model-ctx-edit-btn"
        title={editTitle}
        aria-label={editTitle}
        onClick={e => {
          e.stopPropagation()
          onStart()
        }}
      >
        <IconEdit3 size={11} />
      </button>
    </>
  )
}
