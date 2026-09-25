import { useLanguage } from '../../locales'
import { editorText } from './editorText'
import { useEffect, useMemo, useState } from 'react'
import { ArrowDown, ArrowUp, Braces, Plus, Trash2 } from 'lucide-react'
import type { WorkflowInputKind, WorkflowInputSpec } from '../../core/types'
import { Button } from '../../ui/Button'
import { CompactModal } from '../layout/CompactModal'

const KINDS: WorkflowInputKind[] = ['string', 'number', 'boolean', 'path', 'json']
const RESERVED = new Set(['inputs', 'params', '_index'])
const NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/

interface DraftInput extends Omit<WorkflowInputSpec, 'default'> {
  type: WorkflowInputKind
  hasDefault: boolean
  defaultText: string
  defaultBoolean: boolean
}

function kindOf(spec: WorkflowInputSpec): WorkflowInputKind {
  return spec.type ?? 'string'
}

export function inputValueMatchesKind(kind: WorkflowInputKind, value: unknown): boolean {
  if (kind === 'json') return true
  if (kind === 'number') return typeof value === 'number' && Number.isFinite(value)
  if (kind === 'boolean') return typeof value === 'boolean'
  return typeof value === 'string'
}

export function changeInputKind(
  spec: WorkflowInputSpec,
  type: WorkflowInputKind,
): WorkflowInputSpec {
  if (spec.default !== undefined && !inputValueMatchesKind(type, spec.default)) {
    const { default: _discarded, ...rest } = spec
    return { ...rest, type }
  }
  return { ...spec, type }
}

function toDraft(spec: WorkflowInputSpec): DraftInput {
  const type = kindOf(spec)
  const hasDefault = spec.default !== undefined
  const defaultText =
    !hasDefault || type === 'boolean'
      ? ''
      : type === 'json'
        ? JSON.stringify(spec.default, null, 2)
        : String(spec.default)
  return {
    name: spec.name,
    type,
    required: !!spec.required,
    description: spec.description ?? '',
    sensitive: !!spec.sensitive,
    hasDefault,
    defaultText,
    defaultBoolean: spec.default === true,
  }
}

function fromDraft(draft: DraftInput): WorkflowInputSpec {
  const spec: WorkflowInputSpec = {
    name: draft.name.trim(),
    type: draft.type,
    required: !!draft.required,
    ...(draft.description?.trim() ? { description: draft.description.trim() } : {}),
    sensitive: !!draft.sensitive,
  }
  if (!draft.hasDefault) return spec
  if (draft.type === 'boolean') spec.default = draft.defaultBoolean
  else if (draft.type === 'number') spec.default = Number(draft.defaultText)
  else if (draft.type === 'json') spec.default = JSON.parse(draft.defaultText)
  else spec.default = draft.defaultText
  return spec
}

function draftError(draft: DraftInput, all: DraftInput[]): string | null {
  const name = draft.name.trim()
  if (!NAME_RE.test(name)) return '名称必须匹配 [A-Za-z_][A-Za-z0-9_]*'
  if (RESERVED.has(name)) return '该名称由运行时保留'
  if (all.filter(item => item.name.trim() === name).length > 1) return '名称重复'
  if (!draft.hasDefault) return null
  if (
    draft.type === 'number' &&
    (draft.defaultText.trim() === '' || !Number.isFinite(Number(draft.defaultText)))
  ) {
    return '默认值必须是数字'
  }
  if (draft.type === 'json') {
    try {
      JSON.parse(draft.defaultText)
    } catch {
      return '默认值必须是合法 JSON'
    }
  }
  return null
}

interface WorkflowInputsEditorProps {
  open: boolean
  specs: WorkflowInputSpec[]
  readOnly: boolean
  focusName?: string | null
  onApply: (specs: WorkflowInputSpec[]) => void
  onCancel: () => void
}

export function WorkflowInputsEditor({
  open,
  specs,
  readOnly,
  focusName,
  onApply,
  onCancel,
}: WorkflowInputsEditorProps) {
  const { t } = useLanguage()
  const [drafts, setDrafts] = useState<DraftInput[]>([])

  useEffect(() => {
    if (!open) return
    const next = specs.map(toDraft)
    if (focusName && !next.some(item => item.name === focusName)) {
      next.push(toDraft({ name: focusName, type: 'string' }))
    }
    setDrafts(next)
  }, [open, specs, focusName])

  useEffect(() => {
    if (!open || !focusName) return
    const index = drafts.findIndex(item => item.name === focusName)
    if (index < 0) return
    const frame = requestAnimationFrame(() => {
      const input = document.getElementById(`wfc-input-name-${index}`) as HTMLInputElement | null
      input?.scrollIntoView({ block: 'center' })
      input?.focus()
      input?.select()
    })
    return () => cancelAnimationFrame(frame)
  }, [open, focusName, drafts.length])

  const errors = useMemo(() => drafts.map(draft => draftError(draft, drafts)), [drafts])
  const update = (index: number, patch: Partial<DraftInput>) =>
    setDrafts(items => items.map((item, i) => (i === index ? { ...item, ...patch } : item)))
  const move = (index: number, delta: number) =>
    setDrafts(items => {
      const target = index + delta
      if (target < 0 || target >= items.length) return items
      const next = [...items]
      ;[next[index], next[target]] = [next[target], next[index]]
      return next
    })

  return (
    <CompactModal
      open={open}
      onClose={onCancel}
      title={editorText('外部输入', t)}
      icon={<Braces size={14} />}
      size="xl"
      className="wfc-input-editor-modal"
      footer={
        <>
          <div className="wcf-footer-left">
            <Button variant="ghost" size="sm" onClick={onCancel}>
              {editorText('取消', t)}
            </Button>
          </div>
          <div className="wcf-footer-right">
            <Button
              variant="primary"
              size="sm"
              disabled={readOnly || errors.some(Boolean)}
              onClick={() => onApply(drafts.map(fromDraft))}
            >
              {editorText('应用', t)}
            </Button>
          </div>
        </>
      }
    >
      <div className="wfc-input-editor">
        <div className="wfc-input-editor-head">
          <span>{editorText('输入声明', t)}</span>
          <button
            type="button"
            className="wfc-btn"
            disabled={readOnly}
            onClick={() =>
              setDrafts(items => [
                ...items,
                {
                  name: `input_${items.length + 1}`,
                  type: 'string',
                  required: false,
                  description: '',
                  sensitive: false,
                  hasDefault: false,
                  defaultText: '',
                  defaultBoolean: false,
                },
              ])
            }
          >
            <Plus size={13} /> {editorText('新增', t)}
          </button>
        </div>
        {drafts.length === 0 && (
          <div className="wfc-input-editor-empty">{editorText('暂无外部输入声明', t)}</div>
        )}
        {drafts.map((draft, index) => (
          <div className="wfc-input-editor-row" key={index}>
            <div className="wfc-input-editor-order">
              <button
                type="button"
                className="wfc-icon-btn"
                title={editorText('上移', t)}
                disabled={readOnly || index === 0}
                onClick={() => move(index, -1)}
              >
                <ArrowUp size={14} />
              </button>
              <button
                type="button"
                className="wfc-icon-btn"
                title={editorText('下移', t)}
                disabled={readOnly || index === drafts.length - 1}
                onClick={() => move(index, 1)}
              >
                <ArrowDown size={14} />
              </button>
            </div>
            <div className="wfc-input-editor-fields">
              <label>
                {editorText('名称', t)}
                <input
                  id={`wfc-input-name-${index}`}
                  className="wfc-input wfc-input--mono"
                  disabled={readOnly}
                  value={draft.name}
                  onChange={e => update(index, { name: e.target.value })}
                />
              </label>
              <label>
                {editorText('类型', t)}
                <select
                  className="wfc-input"
                  disabled={readOnly}
                  value={draft.type}
                  onChange={e => {
                    const type = e.target.value as WorkflowInputKind
                    let current: WorkflowInputSpec = { name: draft.name, type: draft.type }
                    try {
                      current = fromDraft(draft)
                    } catch {
                      // Invalid draft defaults are incompatible with every target type.
                    }
                    const changed = changeInputKind(current, type)
                    update(index, {
                      type,
                      hasDefault: changed.default !== undefined,
                      defaultText:
                        changed.default === undefined || type === 'boolean'
                          ? ''
                          : type === 'json'
                            ? JSON.stringify(changed.default, null, 2)
                            : String(changed.default),
                      defaultBoolean: changed.default === true,
                    })
                  }}
                >
                  {KINDS.map(kind => (
                    <option key={kind} value={kind}>
                      {kind}
                    </option>
                  ))}
                </select>
              </label>
              <label className="wfc-input-editor-wide">
                {editorText('说明', t)}
                <input
                  className="wfc-input"
                  disabled={readOnly}
                  value={draft.description ?? ''}
                  onChange={e => update(index, { description: e.target.value })}
                />
              </label>
              <label className="wfc-input-editor-check">
                <input
                  type="checkbox"
                  disabled={readOnly}
                  checked={!!draft.required}
                  onChange={e => update(index, { required: e.target.checked })}
                />
                {editorText('必填', t)}
              </label>
              <label className="wfc-input-editor-check">
                <input
                  type="checkbox"
                  disabled={readOnly}
                  checked={!!draft.sensitive}
                  onChange={e => update(index, { sensitive: e.target.checked })}
                />
                {editorText('敏感', t)}
              </label>
              <label className="wfc-input-editor-check">
                <input
                  type="checkbox"
                  disabled={readOnly}
                  checked={draft.hasDefault}
                  onChange={e => update(index, { hasDefault: e.target.checked })}
                />
                {editorText('默认值', t)}
              </label>
              {draft.hasDefault &&
                (draft.type === 'boolean' ? (
                  <label className="wfc-input-editor-check">
                    <input
                      type="checkbox"
                      disabled={readOnly}
                      checked={draft.defaultBoolean}
                      onChange={e => update(index, { defaultBoolean: e.target.checked })}
                    />
                    {editorText('启用', t)}
                  </label>
                ) : (
                  <label className="wfc-input-editor-default">
                    {editorText('默认值', t)}
                    {draft.type === 'json' ? (
                      <textarea
                        className="wfc-input wfc-input--mono"
                        rows={2}
                        disabled={readOnly}
                        value={draft.defaultText}
                        onChange={e => update(index, { defaultText: e.target.value })}
                      />
                    ) : (
                      <input
                        className="wfc-input"
                        type={draft.type === 'number' ? 'number' : 'text'}
                        disabled={readOnly}
                        value={draft.defaultText}
                        onChange={e => update(index, { defaultText: e.target.value })}
                      />
                    )}
                  </label>
                ))}
              {errors[index] && (
                <div className="wfc-input-editor-error">{editorText(errors[index]!, t)}</div>
              )}
            </div>
            <button
              type="button"
              className="wfc-icon-btn"
              title={editorText('删除', t)}
              disabled={readOnly}
              onClick={() => setDrafts(items => items.filter((_, i) => i !== index))}
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </div>
    </CompactModal>
  )
}
