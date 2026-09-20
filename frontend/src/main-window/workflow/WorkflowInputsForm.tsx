/**
 * WorkflowInputsForm — 工作流外部输入收集（运行前）
 *
 * 唯一实现：工作流运行确认弹窗（WorkflowRunModal）与画布运行路径（CanvasPage 的「运行」/ R 快捷键）
 * 共用本文件的 useWorkflowInputs + WorkflowInputsForm / WorkflowInputsDialog，禁止各自复制一份。
 *
 * 与后端 wf_run(inputs) 的契约：
 * - 只提交「已声明」字段；未声明字段一律不发送
 * - default 预填；required 且未填 → 阻断启动并在字段下方给出原因
 * - sensitive → 密码控件，编辑值只存在于组件内存（state），不落 localStorage / 日志 / URL
 */
import { useCallback, useMemo, useState } from 'react'
import type { WorkflowInputKind, WorkflowInputSpec } from '../../core/types'
import { IconWorkflow } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { CompactModal } from '../layout/CompactModal'
import '../../styles/workflow-modal.css'

/** 编辑态值：文本控件为 string，布尔控件为 boolean */
export type WorkflowInputEditValue = string | boolean

export interface WorkflowInputsState {
  /** 编辑态值（仅内存；sensitive 同样只在此处） */
  values: Record<string, WorkflowInputEditValue>
  /** 全部字段通过校验 → 可提交（required 未填 / 类型非法时为 false） */
  canSubmit: boolean
  /** 字段级阻断原因：name → 原因文案 */
  errors: Record<string, string>
  setValue: (name: string, value: WorkflowInputEditValue) => void
  /** 提交对象：仅已声明字段；未填（空）字段不发送，交由后端「未提供 → 不注入」语义处理 */
  payload: Record<string, unknown>
}

/** 稳定的空声明（避免每次渲染新建数组扰动 hooks 依赖） */
export const NO_INPUT_SPECS: WorkflowInputSpec[] = []

const REQUIRED_MSG = '必填输入，未填写'
const NUMBER_MSG = '请输入有效数字'
const JSON_MSG = 'JSON 格式无效'

function kindOf(spec: WorkflowInputSpec): WorkflowInputKind {
  return spec.type ?? 'string'
}

/** 声明 default → 编辑态文本（对象/数组 JSON 化，保证 json 类型可直接编辑） */
function defaultText(v: unknown): string {
  if (v === undefined || v === null) return ''
  if (typeof v === 'string') return v
  if (typeof v === 'number' || typeof v === 'boolean') return String(v)
  try {
    return JSON.stringify(v) ?? ''
  } catch {
    return ''
  }
}

/** 编辑态初值：boolean → 布尔（缺省 false），其余 → default 预填文本 */
function initialValues(specs: WorkflowInputSpec[]): Record<string, WorkflowInputEditValue> {
  const out: Record<string, WorkflowInputEditValue> = {}
  for (const spec of specs) {
    if (kindOf(spec) === 'boolean') {
      out[spec.name] = typeof spec.default === 'boolean' ? spec.default : false
    } else {
      out[spec.name] = defaultText(spec.default)
    }
  }
  return out
}

/** 单字段求值：error=阻断原因；skip=不发送（未填且非必填）；value=提交值 */
function resolveField(
  spec: WorkflowInputSpec,
  raw: WorkflowInputEditValue | undefined,
): { error?: string; skip?: boolean; value?: unknown } {
  const kind = kindOf(spec)
  if (kind === 'boolean') {
    // 布尔恒有取值（false 是合法值），不参与「未填」判定
    return { value: raw === true }
  }
  const text = typeof raw === 'string' ? raw : ''
  if (text.trim() === '') {
    return spec.required ? { error: REQUIRED_MSG } : { skip: true }
  }
  if (kind === 'number') {
    const n = Number(text)
    return Number.isFinite(n) ? { value: n } : { error: NUMBER_MSG }
  }
  if (kind === 'json') {
    try {
      return { value: JSON.parse(text) }
    } catch {
      return { error: JSON_MSG }
    }
  }
  return { value: text }
}

/**
 * 输入编辑态 + 校验 + 提交对象。
 * resetToken 变化（切换工作流 / 重新打开）→ 渲染期重置为 default 预填值，避免残留上一条工作流的输入。
 */
export function useWorkflowInputs(
  specList: WorkflowInputSpec[] | undefined,
  resetToken: string,
): WorkflowInputsState {
  const specs = specList ?? NO_INPUT_SPECS
  const [snapshot, setSnapshot] = useState<{
    token: string
    values: Record<string, WorkflowInputEditValue>
  }>(() => ({ token: resetToken, values: initialValues(specs) }))
  // React 官方「props 变化时重置 state」模式：渲染期直接改，避免闪现陈旧值
  if (snapshot.token !== resetToken) {
    setSnapshot({ token: resetToken, values: initialValues(specs) })
  }
  const values = snapshot.token === resetToken ? snapshot.values : initialValues(specs)

  const setValue = useCallback(
    (name: string, value: WorkflowInputEditValue) => {
      setSnapshot(prev => ({
        token: resetToken,
        values: {
          ...(prev.token === resetToken ? prev.values : initialValues(specs)),
          [name]: value,
        },
      }))
    },
    [resetToken, specs],
  )

  const { errors, payload } = useMemo(() => {
    const errors: Record<string, string> = {}
    const payload: Record<string, unknown> = {}
    for (const spec of specs) {
      const raw = values[spec.name] ?? (kindOf(spec) === 'boolean' ? false : '')
      const r = resolveField(spec, raw)
      if (r.error) errors[spec.name] = r.error
      else if (!r.skip) payload[spec.name] = r.value
    }
    return { errors, payload }
  }, [specs, values])

  return { values, canSubmit: Object.keys(errors).length === 0, errors, setValue, payload }
}

function inputType(spec: WorkflowInputSpec): 'text' | 'number' | 'password' {
  if (spec.sensitive) return 'password'
  return kindOf(spec) === 'number' ? 'number' : 'text'
}

function placeholderOf(spec: WorkflowInputSpec): string | undefined {
  switch (kindOf(spec)) {
    case 'number':
      return '数字'
    case 'path':
      return '文件/目录路径'
    case 'json':
      return '{ } 或 [ ] JSON'
    default:
      return spec.sensitive ? '敏感值（不落盘）' : '文本'
  }
}

interface WorkflowInputsFormProps {
  specs: WorkflowInputSpec[]
  state: WorkflowInputsState
}

/** 声明式输入表单（纯展示：状态由 useWorkflowInputs 提供，便于两个宿主复用） */
export function WorkflowInputsForm({ specs, state }: WorkflowInputsFormProps) {
  return (
    <div className="wcf-inputs">
      <div className="wcf-inputs-title">运行前需要填写外部输入</div>
      {specs.map(spec => {
        const kind = kindOf(spec)
        const fieldId = `wcf-input-${spec.name}`
        const error = state.errors[spec.name]
        return (
          <div className="wcf-input-field" key={spec.name}>
            <label className="wcf-input-label" htmlFor={fieldId}>
              <span className="wcf-input-name">{spec.name}</span>
              {spec.required && <span className="wcf-input-tag wcf-input-tag--required">必填</span>}
              <span className="wcf-input-tag">{kind}</span>
              {spec.sensitive && <span className="wcf-input-tag">敏感</span>}
            </label>
            {spec.description && <div className="wcf-input-hint">{spec.description}</div>}
            {kind === 'boolean' ? (
              <input
                id={fieldId}
                className="wcf-input-check"
                type="checkbox"
                checked={state.values[spec.name] === true}
                onChange={e => state.setValue(spec.name, e.target.checked)}
              />
            ) : kind === 'json' ? (
              <textarea
                id={fieldId}
                className="wcf-input-control wcf-input-textarea"
                rows={3}
                value={String(state.values[spec.name] ?? '')}
                placeholder={placeholderOf(spec)}
                autoComplete="off"
                spellCheck={false}
                onChange={e => state.setValue(spec.name, e.target.value)}
              />
            ) : (
              <input
                id={fieldId}
                className="wcf-input-control"
                type={inputType(spec)}
                value={String(state.values[spec.name] ?? '')}
                placeholder={placeholderOf(spec)}
                autoComplete="off"
                spellCheck={false}
                onChange={e => state.setValue(spec.name, e.target.value)}
              />
            )}
            {error && (
              <div className="wcf-input-error" role="alert">
                {error}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

interface WorkflowInputsDialogProps {
  open: boolean
  specs: WorkflowInputSpec[]
  /** 重置令牌（工作流 id 等）：变化即回到 default 预填态 */
  resetToken: string
  title?: string
  confirmLabel?: string
  running?: boolean
  onConfirm: (inputs: Record<string, unknown>) => void
  onCancel: () => void
}

/**
 * 运行前输入收集弹层（画布运行路径使用）。
 * 与运行确认弹窗共用 useWorkflowInputs + WorkflowInputsForm：表单实现只有一份。
 */
export function WorkflowInputsDialog({
  open,
  specs,
  resetToken,
  title = '启动工作流 · 外部输入',
  confirmLabel = '启动',
  running,
  onConfirm,
  onCancel,
}: WorkflowInputsDialogProps) {
  const state = useWorkflowInputs(specs, `${resetToken}|${open ? 'open' : 'closed'}`)

  return (
    <CompactModal
      open={open}
      onClose={onCancel}
      title={title}
      icon={<IconWorkflow size={14} />}
      size="auto"
      footer={
        <>
          <div className="wcf-footer-left">
            <Button variant="ghost" size="sm" onClick={onCancel}>
              取消
            </Button>
          </div>
          <div className="wcf-footer-right">
            <Button
              variant="primary"
              size="sm"
              loading={running}
              disabled={!state.canSubmit}
              onClick={() => onConfirm(state.payload)}
            >
              {confirmLabel}
            </Button>
          </div>
        </>
      }
    >
      <WorkflowInputsForm specs={specs} state={state} />
    </CompactModal>
  )
}
