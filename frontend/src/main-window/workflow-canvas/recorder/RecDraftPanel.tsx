/**
 * RecDraftPanel.tsx — 草稿列表（RecorderBar idle 态「草稿」按钮打开的查看/编辑/删除层）
 *
 * 每行：序号 + 动作 + 意图（截断）+ 参数摘要 + 可变标记 + 截图证据标记 + 异常备注标记。
 * - 编辑：仅改 draft 语义字段（intent / variable / exception_note），不动画布节点
 *   （意图语义本就只存在于 draft；画布节点是真实 Action，不含这些标注）。
 * - 删除：草稿带 canvas_step_id → 先调画布 remove_step；成功/节点不在画布/无绑定均删 draft，
 *   由上层回调按结果提示。
 * - 清空草稿：先逐个联动删画布录制节点，再删 pending 文件 + 清空全部草稿（画布删除可 Ctrl+Z
 *   撤销），确认由上层 askConfirm 负责。
 */

import { useState } from 'react'
import { Check, Pencil, Trash2, X } from 'lucide-react'
import type { RecAction, RecDraft } from './recorderTypes'
import { ACTION_LABEL } from './recorderTypes'
import type { RecDraftPatch } from './useWorkflowRecorder'
import './recorder.css'

interface RecDraftPanelProps {
  drafts: RecDraft[]
  onClose: () => void
  onEdit: (index: number, patch: RecDraftPatch) => void
  onDelete: (index: number) => void | Promise<void>
  onClear: () => void
}

const BTN_LABEL: Record<string, string> = {
  left: '左键',
  right: '右键',
  middle: '中键',
}

const KEY_DISPLAY: Record<string, string> = {
  ctrl: 'Ctrl',
  shift: 'Shift',
  alt: 'Alt',
  win: 'Win',
  enter: 'Enter',
  esc: 'Esc',
  tab: 'Tab',
  space: 'Space',
  backspace: 'Backspace',
  delete: 'Delete',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : `${s.slice(0, n)}…`
}

function actionLabel(action: string): string {
  if (action === 'text_chat') return '输入文本 · chatagent'
  return ACTION_LABEL[action as RecAction] ?? action
}

function rectSummary(r: unknown): string | null {
  if (!r || typeof r !== 'object') return null
  const { x, y, width, height } = r as {
    x?: unknown
    y?: unknown
    width?: unknown
    height?: unknown
  }
  if (
    typeof x !== 'number' ||
    typeof y !== 'number' ||
    typeof width !== 'number' ||
    typeof height !== 'number'
  ) {
    return null
  }
  return `${width}×${height} @ (${x}, ${y})`
}

function nameOfPath(p: unknown): string {
  if (typeof p !== 'string') return ''
  const i = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'))
  return i >= 0 ? p.slice(i + 1) : p
}

function keyText(keys: unknown): string {
  if (!Array.isArray(keys) || keys.length === 0) return ''
  return keys
    .map(k => {
      const s = String(k).toLowerCase()
      return KEY_DISPLAY[s] ?? (s.length === 1 ? s.toUpperCase() : s)
    })
    .join(' + ')
}

/** 草稿参数摘要（一眼可读的展示文本，非可执行结构） */
function draftParamsSummary(d: RecDraft): string {
  const p = d.params ?? {}
  const parts: string[] = []
  switch (d.action) {
    case 'click':
      if (typeof p.button === 'string' && BTN_LABEL[p.button]) parts.push(BTN_LABEL[p.button])
      if (typeof p.source_capture === 'string') parts.push(`找图后点击（${p.source_capture}）`)
      if (typeof p.x === 'number' && typeof p.y === 'number') parts.push(`(${p.x}, ${p.y})`)
      if (typeof p.window_title === 'string' && p.window_title) parts.push(p.window_title)
      break
    case 'scroll':
      if (typeof p.direction === 'string') parts.push(p.direction === 'up' ? '上滚' : '下滚')
      if (typeof p.amount === 'number') parts.push(`${p.amount} 格`)
      if (typeof p.x === 'number' && typeof p.y === 'number') parts.push(`(${p.x}, ${p.y})`)
      break
    case 'text':
      if (typeof p.x === 'number' && typeof p.y === 'number') parts.push(`(${p.x}, ${p.y})`)
      if (typeof p.text === 'string') {
        parts.push(p.text === '{{input}}' ? '{{input}}' : `文本「${truncate(p.text, 20)}」`)
      }
      if (p.send === 'enter') parts.push('回车发送')
      break
    case 'text_chat':
      parts.push('chatagent 处理（内容由 WorkflowAgent 补全）')
      break
    case 'hotkey': {
      const k = keyText(p.keys)
      if (k) parts.push(k)
      break
    }
    case 'sleep':
      if (typeof p.seconds === 'number') parts.push(`${p.seconds} 秒`)
      break
    case 'region':
    case 'find_image': {
      const r = rectSummary(p.rect)
      if (r) parts.push(r)
      const n = nameOfPath(p.template_path)
      if (n) parts.push(n)
      if (p.click_after === 'double_click') parts.push('找图后双击')
      else if (p.click_after === 'click') parts.push('找图后单击')
      break
    }
    case 'browser_navigate':
      if (typeof p.url === 'string' && p.url.trim()) parts.push(p.url.trim())
      break
    case 'browser_click':
      if (typeof p.selector === 'string' && p.selector.trim()) {
        parts.push(`selector ${p.selector.trim()}`)
      }
      if (typeof p.text === 'string' && p.text.trim()) {
        parts.push(`「${truncate(p.text.trim(), 20)}」`)
      }
      if (typeof p.url === 'string' && p.url.trim()) parts.push(p.url.trim())
      break
    case 'browser_extract': {
      const note =
        typeof p.content_note === 'string' && p.content_note.trim() ? p.content_note.trim() : ''
      parts.push(note || '获取当前页面主要内容')
      break
    }
    default:
      break
  }
  return parts.filter(Boolean).join(' · ') || '（无参数）'
}

export function RecDraftPanel({ drafts, onClose, onEdit, onDelete, onClear }: RecDraftPanelProps) {
  const [editingIndex, setEditingIndex] = useState<number | null>(null)
  const [intent, setIntent] = useState('')
  const [variable, setVariable] = useState(false)
  const [exception, setException] = useState('')

  const startEdit = (i: number) => {
    const d = drafts[i]
    if (!d) return
    setEditingIndex(i)
    setIntent(d.intent ?? '')
    setVariable(!!d.params?.variable)
    setException(d.exception_note ?? '')
  }

  const saveEdit = () => {
    if (editingIndex === null) return
    onEdit(editingIndex, { intent, variable, exceptionNote: exception })
    setEditingIndex(null)
  }

  const handleDelete = async (i: number) => {
    // 删除会令其后行下标前移：编辑中的行若在被删行或其后，直接退出编辑态防错位
    if (editingIndex !== null && editingIndex >= i) setEditingIndex(null)
    await onDelete(i)
  }

  return (
    <div className="rec-modal-backdrop" onClick={onClose}>
      <div className="rec-draft" onClick={e => e.stopPropagation()}>
        <div className="rec-intent-head">
          <h3 className="rec-intent-title">
            草稿列表
            <span className="rec-draft-count">{drafts.length} 步</span>
          </h3>
          <button type="button" className="rec-intent-close" onClick={onClose} title="关闭">
            <X size={15} />
          </button>
        </div>

        {drafts.length === 0 ? (
          <div className="rec-draft-empty">暂无草稿步骤</div>
        ) : (
          <div className="rec-draft-body">
            {drafts.map((d, i) => {
              const editing = editingIndex === i
              const variableOn = !!d.params?.variable
              const screenshot = d.evidence?.screenshot
              return (
                <div key={i} className={`rec-draft-row${editing ? ' is-editing' : ''}`}>
                  {editing ? (
                    <div className="rec-draft-edit">
                      <div className="rec-draft-edit-note">
                        第 {i + 1} 步 · 仅修改意图语义（intent/可变/异常备注），不影响画布节点
                      </div>
                      <div className="rec-section">
                        <div className="rec-section-title">操作意图（这步在做什么）</div>
                        <input
                          className="rec-input"
                          value={intent}
                          onChange={e => setIntent(e.target.value)}
                          placeholder="必填：描述这一步的目的"
                        />
                      </div>
                      <div className="rec-section">
                        <label className="rec-check">
                          <input
                            type="checkbox"
                            checked={variable}
                            onChange={e => setVariable(e.target.checked)}
                          />
                          <span>
                            可变参数标记
                            <span className="rec-check-hint">
                              （此值每次运行可能变化，需参数化）
                            </span>
                          </span>
                        </label>
                      </div>
                      <div className="rec-section">
                        <div className="rec-section-title">异常分支备注（可选）</div>
                        <textarea
                          className="rec-input rec-input--area"
                          rows={2}
                          value={exception}
                          onChange={e => setException(e.target.value)}
                          placeholder="如「若弹出登录框则中止 / 找不到元素时重试一次」"
                        />
                      </div>
                      <div className="rec-draft-edit-actions">
                        <button
                          type="button"
                          className="rec-btn"
                          onClick={() => setEditingIndex(null)}
                        >
                          取消
                        </button>
                        <button
                          type="button"
                          className="rec-btn rec-btn--primary"
                          disabled={!intent.trim()}
                          onClick={saveEdit}
                        >
                          <Check size={13} /> 保存修改
                        </button>
                      </div>
                    </div>
                  ) : (
                    <>
                      <div className="rec-draft-main">
                        <div className="rec-draft-headline">
                          <span className="rec-draft-stepno">{i + 1}</span>
                          <span className="rec-draft-action">{actionLabel(d.action)}</span>
                          <span className="rec-draft-intent" title={d.intent}>
                            {truncate(d.intent || '（未填意图）', 44)}
                          </span>
                        </div>
                        <div className="rec-draft-meta">
                          <span className="rec-draft-summary">{draftParamsSummary(d)}</span>
                          {variableOn && (
                            <span className="rec-draft-tag rec-draft-tag--var">可(参数化)</span>
                          )}
                          {screenshot ? (
                            <span className="rec-draft-tag rec-draft-tag--shot" title={screenshot}>
                              截图
                            </span>
                          ) : null}
                          {d.exception_note ? (
                            <span
                              className="rec-draft-tag rec-draft-tag--ex"
                              title={d.exception_note}
                            >
                              异常备注
                            </span>
                          ) : null}
                        </div>
                      </div>
                      <div className="rec-draft-ops">
                        <button
                          type="button"
                          className="rec-btn"
                          title="编辑意图/可变/异常备注（不影响画布节点）"
                          onClick={() => startEdit(i)}
                        >
                          <Pencil size={12} /> 编辑
                        </button>
                        <button
                          type="button"
                          className="rec-btn rec-btn--danger"
                          title="删除草稿（有画布节点绑定则同步删除该节点）"
                          onClick={() => void handleDelete(i)}
                        >
                          <Trash2 size={12} /> 删除
                        </button>
                      </div>
                    </>
                  )}
                </div>
              )
            })}
          </div>
        )}

        <div className="rec-draft-foot">
          <button
            type="button"
            className="rec-btn rec-btn--danger"
            disabled={drafts.length === 0 || editingIndex !== null}
            title="清空全部草稿与待恢复进度，并同步删除画布上对应录制节点（可 Ctrl+Z 撤销）"
            onClick={onClear}
          >
            清空草稿
          </button>
          <button type="button" className="rec-btn rec-btn--primary" onClick={onClose}>
            关闭
          </button>
        </div>
      </div>
    </div>
  )
}
