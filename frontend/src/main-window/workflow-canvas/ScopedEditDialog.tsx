import { useEffect, useRef, useState } from 'react'
import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { useLanguage } from '../../locales'
import {
  applyScopedEdit,
  createScopedEditRequest,
  requestScopedEdit,
  scopedEditDiff,
  scopedRevision,
  type ScopedEditProposal,
  type ScopedEditRequest,
} from './scopedEdit'

export interface ScopedEditDialogProps {
  selectedIds: readonly string[] | ReadonlySet<string>
  steps: WorkflowStep[]
  inputs?: WorkflowInputSpec[]
  disabled?: boolean
  onApply: (nextSteps: WorkflowStep[]) => void
  onClose: () => void
}

const words = {
  zh: {
    title: 'AI 修改选中节点',
    scope: '仅修改选中的 {count} 个节点；容器的子节点需要单独选中。',
    hint: '使用工作流配置的模型生成提案。只发送选中节点、变量声明和工具参数说明，不发送运行记录或运行时变量值。',
    instruction: '希望如何修改',
    placeholder: '例如：完善这几个步骤的提示词，保持输入输出变量不变。',
    generate: '生成修改提案',
    generating: '正在生成提案…',
    apply: '应用修改',
    close: '取消',
    regenerate: '重新生成',
    before: '修改前',
    after: '修改后',
    empty: '提案没有字段变化。',
    stale: '工作流已发生变化，此提案已过期，请重新生成。',
    invalid: '提案超出了选中范围或格式不正确，请重新生成。',
    undo: '应用后可使用画布的撤销按钮恢复。',
    missing: '（未设置）',
    selected: '选中节点',
  },
  en: {
    title: 'AI edit selected steps',
    scope:
      'Only the {count} selected steps can change; select child steps explicitly to edit them.',
    hint: 'Uses the configured workflow model. Sends selected steps, variable declarations and tool schemas. Run history and runtime variable values are excluded.',
    instruction: 'Requested changes',
    placeholder: 'For example: improve these prompts while preserving input and output variables.',
    generate: 'Generate proposal',
    generating: 'Generating proposal…',
    apply: 'Apply changes',
    close: 'Cancel',
    regenerate: 'Generate again',
    before: 'Before',
    after: 'After',
    empty: 'The proposal contains no field changes.',
    stale: 'The workflow changed. This proposal is stale; generate it again.',
    invalid: 'The proposal is invalid or outside the selected scope. Generate it again.',
    undo: 'Use the canvas Undo button to revert after applying.',
    missing: '(not set)',
    selected: 'Selected steps',
  },
}

export function ScopedEditDialog({
  selectedIds,
  steps,
  inputs = [],
  disabled = false,
  onApply,
  onClose,
}: ScopedEditDialogProps) {
  const { lang } = useLanguage()
  const text = lang.startsWith('en') ? words.en : words.zh
  const ids = [...selectedIds]
  const [instruction, setInstruction] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [result, setResult] = useState<{
    request: ScopedEditRequest
    proposal: ScopedEditProposal
  } | null>(null)
  const generation = useRef(0)
  useEffect(
    () => () => {
      generation.current += 1
    },
    [],
  )
  const selectionKey = JSON.stringify([...ids].sort())
  const revision = scopedRevision(steps, inputs)
  const stale =
    !!result &&
    (result.proposal.base_revision !== revision ||
      JSON.stringify([...result.request.selected_ids].sort()) !== selectionKey)
  const preview = (() => {
    if (!result || stale) return { changes: [], invalid: false }
    try {
      return { changes: scopedEditDiff(steps, ids, result.proposal, inputs), invalid: false }
    } catch {
      return { changes: [], invalid: true }
    }
  })()
  const close = () => {
    generation.current += 1
    onClose()
  }
  const generate = async () => {
    const token = ++generation.current
    setBusy(true)
    setError('')
    setResult(null)
    try {
      const request = createScopedEditRequest(steps, ids, instruction.trim(), inputs)
      const proposal = await requestScopedEdit(request)
      if (generation.current !== token) return
      // Validate against the captured snapshot first. A later editor change is surfaced as stale.
      scopedEditDiff(request.steps, request.selected_ids, proposal, inputs)
      setResult({ request, proposal })
    } catch (cause) {
      if (generation.current === token)
        setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      if (generation.current === token) setBusy(false)
    }
  }
  const apply = () => {
    if (!result || stale || disabled || busy || preview.invalid) return
    try {
      onApply(applyScopedEdit(steps, ids, result.proposal, inputs))
      close()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    }
  }
  const format = (value: unknown) =>
    value === undefined
      ? text.missing
      : typeof value === 'string'
        ? value
        : JSON.stringify(value, null, 2)
  return (
    <div className="wfc-intent-mask" onClick={close}>
      <div
        className="wfc-intent"
        role="dialog"
        aria-modal="true"
        aria-labelledby="wfc-scoped-edit-title"
        onClick={event => event.stopPropagation()}
        onKeyDown={event => {
          if (event.key === 'Escape') {
            event.stopPropagation()
            close()
          }
        }}
      >
        <div className="wfc-intent-head">
          <h3 id="wfc-scoped-edit-title">{text.title}</h3>
        </div>
        <div className="wfc-intent-body">
          <p>{text.scope.replace('{count}', String(ids.length))}</p>
          <p>{text.hint}</p>
          <p aria-label={text.selected}>
            <code>{ids.join(', ')}</code>
          </p>
          <label htmlFor="wfc-scoped-edit-instruction">{text.instruction}</label>
          <textarea
            id="wfc-scoped-edit-instruction"
            autoFocus
            rows={4}
            maxLength={16000}
            value={instruction}
            placeholder={text.placeholder}
            disabled={busy || disabled}
            style={{ width: '100%', resize: 'vertical' }}
            onChange={event => {
              setInstruction(event.target.value)
              setResult(null)
            }}
          />
          {busy && <p role="status">{text.generating}</p>}
          {error && (
            <p role="alert">
              {['outside_scope', 'invalid_proposal', 'invalid_selection'].includes(error)
                ? text.invalid
                : error === 'stale_revision'
                  ? text.stale
                  : error}
            </p>
          )}
          {stale && <p role="alert">{text.stale}</p>}
          {preview.invalid && <p role="alert">{text.invalid}</p>}
          {result && !stale && !preview.invalid && (
            <div>
              <p>{result.proposal.summary}</p>
              {!preview.changes.length && <p>{text.empty}</p>}
              {preview.changes.map(change => (
                <section key={`${change.stepId}:${change.field}`}>
                  <strong>
                    {change.stepId} · {change.field}
                  </strong>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                    <div>
                      <span>{text.before}</span>
                      <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
                        {format(change.before)}
                      </pre>
                    </div>
                    <div>
                      <span>{text.after}</span>
                      <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
                        {format(change.after)}
                      </pre>
                    </div>
                  </div>
                </section>
              ))}
              <p>{text.undo}</p>
            </div>
          )}
        </div>
        <div className="wfc-intent-foot">
          <button type="button" className="wfc-btn" onClick={close}>
            {text.close}
          </button>
          <button
            type="button"
            className="wfc-btn"
            disabled={busy || disabled || !instruction.trim() || !ids.length}
            onClick={() => {
              void generate()
            }}
          >
            {result ? text.regenerate : text.generate}
          </button>
          <button
            type="button"
            className="wfc-btn wfc-btn--primary"
            disabled={
              busy || disabled || !result || stale || preview.invalid || !preview.changes.length
            }
            onClick={apply}
          >
            {text.apply}
          </button>
        </div>
      </div>
    </div>
  )
}
