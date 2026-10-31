/**
 * RowCtxEditor — 模型卡片内的 Context Window 展示 + 就地编辑。
 *
 *  paradigm：badge（只读）↔ 行内输入（编辑）互换，与卡片其它能力 chip 的
 * 「点击即改」同构：Enter / blur 提交，Escape 取消。未知 ctx 显示「—」
 * （不用裸 `?`：那个符号在设置页里已被「帮助」占用，语义会歧义）。
 *
 * ## 数值口径（2026-10-31 大王定案）
 * 模型界面一律显示**真实 token 数**（1,048,576，千分位），不缩写不四舍五入：
 * 这里是要拿来做校准的地方，`1.0M` 会藏掉 48,576 个 token；编辑框同单位
 * （tokens），避免「显示 1048576、编辑却按 K」的两套口径把值改错。
 * 读数位（状态栏 / 气泡 / 移动端）走紧凑口径，见 ui/tokenFormat。
 */
import { IconEdit3 } from '../../ui/Icons'
import { formatExactTokens } from '../../ui/tokenFormat'

type TFunc = (key: string, ...args: string[]) => string

const tr = (t: TFunc, key: string, fallback: string): string => {
  const v = t(key)
  return v === key ? fallback : v
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
          min={1}
          max={10000000}
          step={1}
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
        <span className="ctx-unit">tokens</span>
      </span>
    )
  }
  return (
    <>
      {ctx && ctx > 0 ? (
        <span
          className="model-badge model-badge--ctx"
          title={`${ctxCapTitle} ${formatExactTokens(ctx)} tokens`}
        >
          {formatExactTokens(ctx)}
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
