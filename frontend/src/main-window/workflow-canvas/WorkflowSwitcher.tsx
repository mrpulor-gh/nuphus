import { useEffect, useRef, useState } from 'react'
import { ChevronDown } from 'lucide-react'
import type { WorkflowItem } from '../../core/types'
import { listWorkflows } from '../lib/api'
import { useLanguage } from '../../locales'

/**
 * 工作流切换器 —— 画布标题右侧的下拉，直接跳到另一张画布，不必退回列表页重新找。
 *
 * 数据：菜单**首次展开**才查一次 `wf_list`（懒加载，打开画布本身不付这次 IPC）。
 * 交互：点击外部收起；当前工作流只标记不隐藏（用户需要确认「我在哪」，点它等于不操作）。
 * 禁用：由调用方判定（运行中 / 历史回放中 —— 离开当前画布会丢运行上下文）。
 */
export function WorkflowSwitcher({
  currentId,
  onSwitch,
  disabled = false,
  disabledHint,
}: {
  currentId: string
  onSwitch: (id: string) => void
  disabled?: boolean
  disabledHint?: string
}) {
  const { lang } = useLanguage()
  const ui = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const [open, setOpen] = useState(false)
  const [list, setList] = useState<WorkflowItem[] | null>(null)
  const wrapRef = useRef<HTMLDivElement>(null)

  // 点击外部收起（与工具栏其它菜单同一模式）
  useEffect(() => {
    if (!open) return
    const onDown = (e: PointerEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false)
    }
    window.addEventListener('pointerdown', onDown, true)
    return () => window.removeEventListener('pointerdown', onDown, true)
  }, [open])

  const toggle = () => {
    if (!open && list === null) {
      // 失败也落 []：菜单给出「没有其它工作流」而不是永久「正在加载…」
      void listWorkflows()
        .then(setList)
        .catch(() => setList([]))
    }
    setOpen(o => !o)
  }

  return (
    <div className="wfc-wf-switch" ref={wrapRef}>
      <button
        type="button"
        className="wfc-icon-btn"
        onClick={toggle}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        title={
          disabled
            ? (disabledHint ?? ui('当前不可切换工作流', 'Cannot switch workflows now'))
            : ui('切换工作流', 'Switch workflow')
        }
        aria-label={ui('切换工作流', 'Switch workflow')}
      >
        <ChevronDown size={13} />
      </button>
      {open && (
        <div className="wfc-wf-menu" role="menu">
          {list === null && <div className="wfc-wf-menu-hint">{ui('正在加载…', 'Loading…')}</div>}
          {list?.length === 0 && (
            <div className="wfc-wf-menu-hint">{ui('没有其它工作流', 'No other workflows')}</div>
          )}
          {list?.map(w => (
            <button
              key={w.id}
              type="button"
              role="menuitem"
              className={`wfc-wf-menu-item${w.id === currentId ? ' is-current' : ''}`}
              title={w.title}
              onClick={() => {
                setOpen(false)
                if (w.id !== currentId) onSwitch(w.id)
              }}
            >
              <span className="wfc-wf-menu-name">{w.title}</span>
              {w.id === currentId && (
                <span className="wfc-wf-menu-tag">{ui('当前', 'Current')}</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
