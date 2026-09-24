/**
 * Inspector.tsx — 右侧参数面板（设计文档 2.5/3.3）
 * 按 kind 生成表单；字段级编辑一律经 onPatch → update_fields IrEditOp 写回。
 * 表单约束前置 V9/V11 等规则（不允许输入非法值），结构校验由 validate.ts 呈现。
 */

import { createContext, useContext, useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { IconX } from '../../ui/Icons'
import type { WorkflowStep, Condition, VarRef, ToolSchema } from '../../core/types'
import { wfTools, listModels, type ModelInfo } from '../lib/api'
import { stepKind } from './projection'
import { ToolParameterForm } from './ToolParameterForm'
import {
  changeConditionOperator,
  numberError,
  parseExitCodes,
  switchLoopMode,
} from './inspectorValues'
import './inspector.css'
import { useInspectorDraft } from './inspectorDrafts'
import { VariablePicker } from './VariablePicker'
import { isVariableName, type VariableCatalog } from './variableCatalog'

const VariableContext = createContext<{
  catalog: VariableCatalog
  onConfigureInput?: (name: string) => void
}>({ catalog: { references: [], captures: [] } })

interface InspectorProps {
  step: WorkflowStep
  readOnly: boolean
  /** run_history 引用了本步骤 id（改 id 需确认，由父层 checkOp 裁决） */
  idReferenced: boolean
  /** 最近输出（StepRunOutput 最近 3 行 / run_history output_summary） */
  lastOutput?: string[]
  /** 新建节点待聚焦的工具输入框（step.id 匹配时 autofocus，消费一次） */
  focusToolId?: string | null
  variableCatalog?: VariableCatalog
  onConfigureInput?: (name: string) => void
  captureConsumers?: { id: string; name: string }[]
  onLocateReference?: (stepId: string) => void
  onPatch: (patch: Partial<WorkflowStep>) => void | Promise<boolean>
  onPatchAction: (action: WorkflowStep['do']) => void | Promise<boolean>
  onClose: () => void
}

// ── 工具注册表：进入画布加载一次并缓存；失败 → null（静默回退纯文本模式） ──
let toolsCache: ToolSchema[] | null | undefined
let toolsInflight: Promise<ToolSchema[] | null> | null = null

/** 加载 wf_tools（模块级单例缓存；ToolPalette 复用，避免二次请求） */
export function loadToolsOnce(): Promise<ToolSchema[] | null> {
  if (toolsCache != null) return Promise.resolve(toolsCache)
  if (!toolsInflight) {
    toolsInflight = wfTools()
      .then(t => {
        toolsCache = t ?? []
        return toolsCache
      })
      .catch(() => {
        return null
      })
      .finally(() => {
        toolsInflight = null
      })
  }
  return toolsInflight
}

// ── 模型注册表：同 tools 缓存策略；失败/为空 → chat 表单隐藏模型下拉（不报错） ──
let modelsCache: ModelInfo[] | null | undefined
let modelsInflight: Promise<ModelInfo[] | null> | null = null

function loadModelsOnce(): Promise<ModelInfo[] | null> {
  if (modelsCache !== undefined) return Promise.resolve(modelsCache)
  if (!modelsInflight) {
    modelsInflight = listModels()
      .then(m => {
        modelsCache = m ?? []
        return modelsCache
      })
      .catch(() => {
        modelsCache = null
        return null
      })
  }
  return modelsInflight
}

/** 本地文本缓冲：失焦/回车才提交（避免每次击键都重投影） */
function TextField({
  label,
  value,
  onCommit,
  readOnly,
  placeholder,
  mono,
  multiline,
  title,
  required,
  error,
  validate,
  references,
}: {
  label: string
  value: string
  onCommit: (v: string) => unknown
  readOnly?: boolean
  placeholder?: string
  mono?: boolean
  multiline?: boolean
  title?: string
  /** 语法必填字段：label 追加「（必填）」标记（对齐 compiler.rs 校验） */
  required?: boolean
  /** 字段级内联错误文案（D2：空名称/必填缺失时展示，与 JsonField 同款样式） */
  error?: string | null
  validate?: (value: string) => string | null
  references?: boolean
}) {
  const variables = useContext(VariableContext)
  const fieldId = useId()
  const textRef = useRef<HTMLTextAreaElement>(null)
  const {
    text: draft,
    setText: setDraft,
    commit,
    error: draftError,
  } = useInspectorDraft(label, value, {
    onCommit,
    validate: validate ?? (required ? v => (v.trim() ? null : '此项必填') : undefined),
  })
  const [expanded, setExpanded] = useState(false)
  const [variableQuery, setVariableQuery] = useState('')
  const validationError = draftError ?? validate?.(draft) ?? (draft === value ? error : null)
  return (
    <label className="wfc-field" title={title} data-draft-field={label} htmlFor={fieldId}>
      <span className="wfc-field-label">
        {label}
        {required && <em className="wfc-required-mark">（必填）</em>}
        {multiline && (
          <button type="button" className="wfc-expand-editor" onClick={() => setExpanded(v => !v)}>
            {expanded ? '收起编辑器' : '展开编辑器'}
          </button>
        )}
      </span>
      {multiline ? (
        <textarea
          id={fieldId}
          ref={textRef}
          className={`wfc-input${mono ? ' wfc-input--mono' : ''}${error ? ' wfc-input--error' : ''}`}
          value={draft}
          rows={expanded ? 22 : mono ? 8 : 3}
          readOnly={readOnly}
          placeholder={placeholder}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
        />
      ) : (
        <input
          id={fieldId}
          className={`wfc-input${mono ? ' wfc-input--mono' : ''}${error ? ' wfc-input--error' : ''}`}
          value={draft}
          readOnly={readOnly}
          placeholder={placeholder}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={e => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing)
              (e.target as HTMLInputElement).blur()
          }}
        />
      )}
      {validationError && <span className="wfc-field-error">{validationError}</span>}
      {references && !readOnly && (
        <VariablePicker
          value={variableQuery}
          mode="reference"
          catalog={variables.catalog}
          label={`插入变量到${label}`}
          onConfigureInput={variables.onConfigureInput}
          onChange={name => {
            setVariableQuery(name)
            if (!variables.catalog.references.some(v => v.name === name)) return
            const start = textRef.current?.selectionStart ?? draft.length
            const end = textRef.current?.selectionEnd ?? start
            const token = `{{${name}}}`
            setDraft(draft.slice(0, start) + token + draft.slice(end))
            setVariableQuery('')
            requestAnimationFrame(() => {
              textRef.current?.focus()
              textRef.current?.setSelectionRange(start + token.length, start + token.length)
            })
          }}
        />
      )}
    </label>
  )
}

type ComboItem = { kind: 'custom'; text: string } | { kind: 'tool'; tool: ToolSchema }

/** 可搜索工具 combobox：注册表驱动过滤 + 键盘导航 + 手输兜底（未匹配值也可提交） */
function ToolCombobox({
  label,
  value,
  tools,
  autoFocus,
  onCommit,
  required,
}: {
  label: string
  value: string
  tools: ToolSchema[]
  autoFocus?: boolean
  onCommit: (v: string) => void
  /** 语法必填字段：label 追加「（必填）」标记 */
  required?: boolean
}) {
  const {
    text: draft,
    setText: setDraft,
    commit,
  } = useInspectorDraft(label, value, {
    onCommit,
    validate: required ? v => (v.trim() ? null : '请选择或输入工具名') : undefined,
  })
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  // 向上翻转：Inspector body 是滚动容器（overflow-y:auto）会裁切下拉，
  // 字段靠近底部时向下展开最多露 2 项——空间不足改向上展开
  const [menuRect, setMenuRect] = useState({ left: 0, top: 0, width: 300, maxHeight: 240 })
  const listId = useId()
  const menuRef = useRef<HTMLDivElement>(null)
  const wrapRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const focusedOnceRef = useRef(false)
  const positionMenu = () => {
    const rect = wrapRef.current?.getBoundingClientRect()
    if (!rect) return
    const below = window.innerHeight - rect.bottom - 8
    const height = Math.max(80, Math.min(240, below >= 160 ? below : rect.top - 8))
    setMenuRect({
      left: Math.max(8, Math.min(rect.left, window.innerWidth - rect.width - 8)),
      top: below >= 160 ? rect.bottom + 4 : Math.max(8, rect.top - height - 4),
      width: Math.min(rect.width, window.innerWidth - 16),
      maxHeight: height,
    })
  }
  const openMenu = () => {
    positionMenu()
    setOpen(true)
  }
  useEffect(() => {
    if (autoFocus && !focusedOnceRef.current) {
      focusedOnceRef.current = true
      inputRef.current?.focus()
    }
  }, [autoFocus])

  const q = draft.trim().toLowerCase()
  const matches = q
    ? tools.filter(t => t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q))
    : tools
  const exact = tools.some(t => t.name === draft.trim())
  const items: ComboItem[] = [
    // 手输兜底：输入未精确匹配注册表时，顶部提供「使用输入值」项（兼容未来/自定义工具）
    ...(q && !exact ? [{ kind: 'custom' as const, text: draft.trim() }] : []),
    ...matches.map(t => ({ kind: 'tool' as const, tool: t })),
  ]
  const activeIdx = items.length > 0 ? Math.min(active, items.length - 1) : 0

  // 点击外部关下拉
  useEffect(() => {
    if (!open) return
    const onPointerDown = (e: PointerEvent) => {
      if (
        !wrapRef.current?.contains(e.target as Node) &&
        !menuRef.current?.contains(e.target as Node)
      )
        setOpen(false)
    }
    window.addEventListener('resize', positionMenu)
    window.addEventListener('scroll', positionMenu, true)
    window.addEventListener('pointerdown', onPointerDown, true)
    return () => {
      window.removeEventListener('pointerdown', onPointerDown, true)
      window.removeEventListener('resize', positionMenu)
      window.removeEventListener('scroll', positionMenu, true)
    }
  }, [open])
  useEffect(() => {
    menuRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' })
  }, [activeIdx, open])

  const pick = (item: ComboItem) => {
    const v = item.kind === 'custom' ? item.text : item.tool.name
    setDraft(v)
    setOpen(false)
    void commit()
    inputRef.current?.focus()
  }

  return (
    <div className="wfc-field wfc-combo" ref={wrapRef}>
      <span className="wfc-field-label">
        {label}
        {required && <em className="wfc-required-mark">（必填）</em>}
      </span>
      <input
        ref={inputRef}
        className="wfc-input wfc-input--mono"
        value={draft}
        placeholder="搜索或输入工具名"
        role="combobox"
        aria-label={label}
        aria-controls={listId}
        aria-expanded={open}
        aria-activedescendant={open && items.length ? `${listId}-${activeIdx}` : undefined}
        onClick={openMenu}
        onChange={e => {
          setDraft(e.target.value)
          openMenu()
          setActive(0)
        }}
        onBlur={() => {
          setOpen(false)
          void commit()
        }}
        onKeyDown={e => {
          if (e.nativeEvent.isComposing) return
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault()
            if (!open) openMenu()
            else if (items.length > 0) {
              setActive(i =>
                e.key === 'ArrowDown'
                  ? (i + 1) % items.length
                  : (i - 1 + items.length) % items.length,
              )
            }
          } else if (e.key === 'Enter') {
            if (open && items.length > 0) {
              e.preventDefault()
              pick(items[activeIdx])
            } else {
              // 对齐 TextField：Enter 提交（经 blur）
              ;(e.target as HTMLInputElement).blur()
            }
          } else if (e.key === 'Escape' && open) {
            // 仅关下拉，不冒泡关 Inspector
            e.preventDefault()
            e.stopPropagation()
            setOpen(false)
          }
        }}
      />
      {open &&
        createPortal(
          <div
            className="wfc-combo-menu wfc-combo-menu--portal"
            ref={menuRef}
            id={listId}
            role="listbox"
            style={menuRect}
          >
            {items.length === 0 ? (
              <div className="wfc-combo-empty">无匹配工具</div>
            ) : (
              items.map((it, i) => (
                <button
                  key={it.kind === 'custom' ? '__custom__' : it.tool.name}
                  type="button"
                  role="option"
                  id={`${listId}-${i}`}
                  aria-selected={i === activeIdx}
                  className={`wfc-combo-item${i === activeIdx ? ' wfc-combo-item--active' : ''}`}
                  onPointerDown={e => e.preventDefault()}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => pick(it)}
                >
                  {it.kind === 'custom' ? (
                    <span className="wfc-combo-name">使用输入值 {it.text}</span>
                  ) : (
                    <>
                      <span className="wfc-combo-name">{it.tool.name}</span>
                      {it.tool.description && (
                        <span className="wfc-combo-desc">{it.tool.description}</span>
                      )}
                    </>
                  )}
                </button>
              ))
            )}
          </div>,
          document.body,
        )}
    </div>
  )
}

/** 从 input_schema（JSON Schema）容错解析必填参数：缺失/非对象/无 required → 无必填 */
function parseRequired(
  schema: Record<string, unknown> | undefined,
): { name: string; desc?: string }[] {
  if (!schema || typeof schema !== 'object') return []
  const required = Array.isArray(schema.required)
    ? schema.required.filter((r): r is string => typeof r === 'string')
    : []
  const props =
    schema.properties && typeof schema.properties === 'object'
      ? (schema.properties as Record<string, unknown>)
      : {}
  return required.map(name => {
    const p = props[name]
    const desc =
      p && typeof p === 'object' && typeof (p as Record<string, unknown>).description === 'string'
        ? ((p as Record<string, unknown>).description as string)
        : undefined
    return { name, desc }
  })
}

function RequiredHint({ schema }: { schema?: Record<string, unknown> }) {
  if (!schema) return null
  const items = parseRequired(schema)
  return (
    <div className="wfc-required-hint">
      {items.length === 0 ? (
        <span>无必填参数</span>
      ) : (
        <>
          <span>必填:</span>
          {items.map(it => (
            <span key={it.name} className="wfc-required-chip" title={it.desc}>
              {it.name}
            </span>
          ))}
        </>
      )}
    </div>
  )
}

/**
 * 工具名字段：注册表 combobox + 必填参数提示。
 * tools === null（加载失败）或 readOnly 时退化为纯文本 TextField。
 */
function ToolField({
  label,
  value,
  tools,
  readOnly,
  autoFocus,
  onCommit,
  required,
}: {
  label: string
  value: string
  /** undefined=加载中 / null=加载失败 / 数组=注册表 */
  tools: ToolSchema[] | null | undefined
  readOnly?: boolean
  autoFocus?: boolean
  onCommit: (v: string) => void
  /** 语法必填字段：label 追加「（必填）」标记 */
  required?: boolean
}) {
  if (readOnly || tools === null) {
    return (
      <TextField
        label={label}
        value={value}
        readOnly={readOnly}
        mono
        onCommit={onCommit}
        required={required}
      />
    )
  }
  const list = tools ?? []
  return (
    <>
      <ToolCombobox
        label={label}
        value={value}
        tools={list}
        autoFocus={autoFocus}
        onCommit={onCommit}
        required={required}
      />
      <RequiredHint schema={list.find(t => t.name === value.trim())?.input_schema} />
    </>
  )
}

function JsonField({
  label,
  value,
  onCommit,
  readOnly,
}: {
  label: string
  value: unknown
  onCommit: (v: unknown) => void
  readOnly?: boolean
}) {
  const text = JSON.stringify(value ?? {}, null, 2)
  const {
    text: draft,
    setText: setDraft,
    commit,
    error: err,
  } = useInspectorDraft(label, text, {
    validate: v => {
      try {
        JSON.parse(v)
        return null
      } catch {
        return 'JSON 格式不正确，请检查引号、逗号和括号'
      }
    },
    onCommit: v => onCommit(JSON.parse(v)),
  })
  const [expanded, setExpanded] = useState(false)
  return (
    <label className="wfc-field" data-draft-field={label}>
      <span className="wfc-field-label">
        {label}
        <button type="button" className="wfc-expand-editor" onClick={() => setExpanded(v => !v)}>
          {expanded ? '收起编辑器' : '展开编辑器'}
        </button>
      </span>
      <textarea
        className="wfc-input wfc-input--mono"
        value={draft}
        rows={expanded ? 22 : 6}
        readOnly={readOnly}
        onChange={e => setDraft(e.target.value)}
        onBlur={commit}
      />
      {err && <span className="wfc-field-error">{err}</span>}
    </label>
  )
}

const COND_OPS: [string, string][] = [
  ['equals', '等于'],
  ['not_equals', '不等于'],
  ['contains', '包含'],
  ['starts_with', '前缀是'],
  ['regex', '正则匹配'],
  ['not_empty', '非空'],
  ['empty', '为空'],
  ['gt', '>'],
  ['lt', '<'],
  ['gte', '≥'],
  ['lte', '≤'],
  ['always', '固定结果'],
]

function condOpOf(cond: Condition | undefined): string {
  if (!cond) return 'always'
  const c = cond as Record<string, unknown>
  for (const [key] of COND_OPS) if (key in c) return key
  return 'always'
}

function condOperandsOf(cond: Condition | undefined): VarRef[] {
  if (!cond) return []
  const c = cond as Record<string, unknown>
  const op = condOpOf(cond)
  const v = c[op]
  if (Array.isArray(v)) return v as VarRef[]
  if (v !== undefined && op !== 'always') return [v as VarRef]
  return []
}

function ConditionEditor({
  value,
  onChange,
  readOnly,
}: {
  value: Condition | undefined
  onChange: (c: Condition) => void
  readOnly?: boolean
}) {
  const op = condOpOf(value)
  const operands = condOperandsOf(value)
  const setOp = (nextOp: string) => {
    onChange(changeConditionOperator(nextOp === 'always' ? 'always_true' : nextOp, operands))
  }
  const setOperand = (i: number, v: VarRef) => {
    const next = [...operands]
    next[i] = v
    if (op === 'not_empty' || op === 'empty') onChange({ [op]: next[0] } as unknown as Condition)
    else onChange({ [op]: next } as unknown as Condition)
  }
  const unary = op === 'not_empty' || op === 'empty'
  const count = op === 'always' ? 0 : unary ? 1 : 2
  return (
    <div className="wfc-cond">
      <label className="wfc-field">
        <span className="wfc-field-label">条件</span>
        <select
          className="wfc-input"
          value={op}
          disabled={readOnly}
          onChange={e => setOp(e.target.value)}
        >
          {COND_OPS.map(([k, label]) => (
            <option key={k} value={k}>
              {label}（{k}）
            </option>
          ))}
        </select>
      </label>
      {op === 'always' && (
        <label className="wfc-field">
          <span className="wfc-field-label">固定结果</span>
          <select
            className="wfc-input"
            disabled={readOnly}
            value={String((value as { always?: boolean })?.always ?? true)}
            onChange={e => onChange({ always: e.target.value === 'true' })}
          >
            <option value="true">真（满足条件）</option>
            <option value="false">假（不满足条件）</option>
          </select>
        </label>
      )}
      {Array.from({ length: count }).map((_, i) => {
        const r = operands[i]
        const isVar = !!r && typeof r === 'object' && 'var' in r
        const text = isVar ? String((r as { var: string }).var) : typeof r === 'string' ? r : ''
        return (
          <div className="wfc-cond-operand" key={i}>
            <select
              className="wfc-input wfc-cond-kind"
              value={isVar ? 'var' : 'lit'}
              disabled={readOnly}
              onChange={e => setOperand(i, e.target.value === 'var' ? { var: text } : text)}
              aria-label={i === 0 ? '左侧值类型' : '右侧值类型'}
            >
              <option value="lit">固定值</option>
              <option value="var">变量</option>
            </select>
            {isVar ? (
              <ReferenceField
                value={text}
                readOnly={readOnly}
                label={i === 0 ? '左侧变量' : '右侧变量'}
                onCommit={v => setOperand(i, { var: v })}
              />
            ) : (
              <TextField
                label={i === 0 ? '左侧固定值' : '右侧固定值'}
                value={text}
                readOnly={readOnly}
                placeholder="字面量值"
                onCommit={v => setOperand(i, v)}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

type OnErrorValue = WorkflowStep['on_error']

function onErrorMode(oe: OnErrorValue): string {
  if (!oe || oe === 'abort') return 'abort'
  if (oe === 'skip') return 'skip'
  if (typeof oe === 'object' && 'retry' in oe) return 'retry'
  if (typeof oe === 'object' && 'allow_codes' in oe) return 'allow_codes'
  return 'abort'
}

export function Inspector({
  step,
  readOnly,
  idReferenced,
  lastOutput,
  focusToolId,
  variableCatalog = { references: [], captures: [] },
  onConfigureInput,
  captureConsumers,
  onLocateReference,
  onPatch,
  onPatchAction,
  onClose,
}: InspectorProps) {
  const kind = stepKind(step)
  const [parameterReset, setParameterReset] = useState(0)
  const d = step.do as Record<string, unknown>
  const panelRef = useRef<HTMLElement>(null)
  const [panelWidth, setPanelWidth] = useState(() => {
    try {
      return Math.max(
        360,
        Math.min(720, Number(localStorage.getItem('wfc-inspector-width')) || 420),
      )
    } catch {
      return 420
    }
  })
  const [availableWidth, setAvailableWidth] = useState(window.innerWidth)
  useEffect(() => {
    const parent = panelRef.current?.parentElement
    const update = () => setAvailableWidth(parent?.clientWidth || window.innerWidth)
    update()
    const observer = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(update) : null
    if (parent) observer?.observe(parent)
    window.addEventListener('resize', update)
    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', update)
    }
  }, [])
  const widthLimit = Math.max(240, availableWidth - 280)
  const visibleWidth = Math.min(panelWidth, widthLimit)
  const persistWidth = (width: number) => {
    setPanelWidth(width)
    try {
      localStorage.setItem('wfc-inspector-width', String(width))
    } catch {
      /* private browsing */
    }
  }
  // 工具注册表（模块级缓存，加载一次；失败 → null 回退纯文本）
  const [tools, setTools] = useState<ToolSchema[] | null | undefined>(toolsCache)
  const [toolChanged, setToolChanged] = useState(false)
  useEffect(() => {
    let alive = true
    void loadToolsOnce().then(t => {
      if (alive) setTools(t)
    })
    return () => {
      alive = false
    }
  }, [])
  // 模型注册表（同 tools 缓存策略；失败/为空 → chat 分支隐藏模型下拉，不报错）
  const [models, setModels] = useState<ModelInfo[] | null | undefined>(modelsCache)
  useEffect(() => {
    let alive = true
    void loadModelsOnce().then(m => {
      if (alive) setModels(m)
    })
    return () => {
      alive = false
    }
  }, [])
  const oeMode = onErrorMode(step.on_error)
  const retryCfg =
    typeof step.on_error === 'object' && step.on_error && 'retry' in step.on_error
      ? step.on_error.retry
      : null
  const allowCfg =
    typeof step.on_error === 'object' && step.on_error && 'allow_codes' in step.on_error
      ? step.on_error.allow_codes
      : null

  const patchActionKey = (key: string, v: unknown) => {
    return onPatchAction({ ...d, [key]: v } as WorkflowStep['do'])
  }

  // chat 步骤 with（ChatOpts）字段级写入：undefined = 删除该键（回退后端默认）
  const chatWith = (d.with && typeof d.with === 'object' ? d.with : {}) as Record<string, unknown>
  const patchChatWith = (key: string, v: unknown) => {
    const next = { ...chatWith }
    if (v === undefined) delete next[key]
    else next[key] = v
    return patchActionKey('with', next)
  }

  return (
    <VariableContext.Provider value={{ catalog: variableCatalog, onConfigureInput }}>
      <aside
        className="wfc-inspector"
        ref={panelRef}
        style={{ width: visibleWidth, maxWidth: '100%' }}
      >
        <div
          className="wfc-inspector-resize"
          role="separator"
          aria-orientation="vertical"
          aria-label="调整参数面板宽度"
          aria-valuemin={360}
          aria-valuemax={720}
          aria-valuenow={panelWidth}
          tabIndex={0}
          onKeyDown={e => {
            if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
              e.preventDefault()
              persistWidth(
                Math.max(360, Math.min(720, panelWidth + (e.key === 'ArrowLeft' ? 20 : -20))),
              )
            }
          }}
          onPointerDown={e => {
            e.preventDefault()
            e.currentTarget.setPointerCapture(e.pointerId)
          }}
          onPointerMove={e => {
            if (e.currentTarget.hasPointerCapture(e.pointerId)) {
              const right = panelRef.current?.getBoundingClientRect().right ?? window.innerWidth
              persistWidth(Math.max(360, Math.min(720, right - e.clientX)))
            }
          }}
          onPointerUp={e => {
            if (e.currentTarget.hasPointerCapture(e.pointerId))
              e.currentTarget.releasePointerCapture(e.pointerId)
          }}
        />
        <div className="wfc-inspector-head">
          <span className="wfc-inspector-title">{step.name || step.id}</span>
          <span className="wfc-badge">{kind}</span>
          <button type="button" className="wfc-icon-btn" onClick={onClose} title="关闭（Esc）">
            <IconX size={14} />
          </button>
        </div>

        <div className="wfc-inspector-body">
          <div className="wfc-inspector-section">基本信息</div>
          <TextField
            label="名称"
            value={step.name}
            readOnly={readOnly}
            onCommit={v => onPatch({ name: v })}
            required
            error={
              !readOnly && !step.name.trim() ? '名称必填：尚未命名无法保存/通过校验' : undefined
            }
          />
          <details className="wfc-inspector-advanced">
            <summary>
              高级设置
              {oeMode !== 'abort' || step.timeout_secs != null ? '（已自定义失败处理或超时）' : ''}
            </summary>
            <TextField
              label={idReferenced ? '步骤标识 ID（有历史记录，修改需确认）' : '步骤标识 ID'}
              value={step.id}
              readOnly={readOnly}
              mono
              required
              onCommit={v => onPatch({ id: v })}
            />
            <TextField
              label="步骤说明"
              value={step.description ?? ''}
              readOnly={readOnly}
              multiline
              onCommit={v => onPatch({ description: v })}
            />
            <TextField
              label="超时秒数（空为执行器默认）"
              value={step.timeout_secs != null ? String(step.timeout_secs) : ''}
              readOnly={readOnly}
              validate={v => numberError(v, { integer: true, min: 1, optional: true })}
              onCommit={v => {
                return onPatch({ timeout_secs: v.trim() ? Number(v) : undefined })
              }}
            />
            {!['seq', 'if', 'mcp'].includes(kind) && (
              <div className="wfc-inspector-hint">
                此类步骤暂不使用上述超时字段。
                {kind === 'script'
                  ? '脚本执行器固定超时为 120 秒。'
                  : kind === 'tool'
                    ? '工具的超时由具体工具决定。'
                    : '已保留历史配置，执行时使用对应动作的行为。'}
              </div>
            )}

            <label className="wfc-field">
              <span className="wfc-field-label">
                失败后怎么办 <small>on_error</small>
              </span>
              <select
                className="wfc-input"
                value={oeMode}
                disabled={readOnly}
                onChange={e => {
                  const m = e.target.value
                  if (m === 'retry') onPatch({ on_error: { retry: { max: 3, backoff_ms: 500 } } })
                  else if (m === 'allow_codes')
                    onPatch({ on_error: { allow_codes: { codes: [0] } } })
                  else onPatch({ on_error: m as OnErrorValue })
                }}
              >
                <option value="abort">终止工作流（默认）</option>
                <option value="skip">跳过失败步骤，继续执行</option>
                <option value="retry" disabled={kind !== 'tool'}>
                  重试（工具步骤）
                </option>
                <option value="allow_codes" disabled={kind !== 'tool'}>
                  允许指定退出码（工具步骤）
                </option>
              </select>
            </label>
            {kind === 'tool' && oeMode === 'abort' && (
              <div className="wfc-inspector-hint">工具执行器仍可能先自动重试，再终止工作流。</div>
            )}
            {kind !== 'tool' && (oeMode === 'retry' || oeMode === 'allow_codes') && (
              <div className="wfc-inspector-hint wfc-inspector-hint--warn">
                已保留历史配置；当前步骤执行器不支持此失败处理设置。
              </div>
            )}
            {oeMode === 'retry' && retryCfg && (
              <div className="wfc-field-row">
                <TextField
                  label="重试次数"
                  value={String(retryCfg.max)}
                  readOnly={readOnly}
                  validate={v => numberError(v, { integer: true, min: 0, max: 4294967295 })}
                  onCommit={v => {
                    const n = Number(v)
                    return onPatch({ on_error: { retry: { ...retryCfg, max: n } } })
                  }}
                />
                <TextField
                  label="重试间隔（毫秒）"
                  value={String(retryCfg.backoff_ms ?? 500)}
                  readOnly={readOnly}
                  validate={v => numberError(v, { integer: true, min: 0 })}
                  onCommit={v => {
                    const n = Number(v)
                    return onPatch({ on_error: { retry: { ...retryCfg, backoff_ms: n } } })
                  }}
                />
              </div>
            )}
            {oeMode === 'retry' && retryCfg?.backoff_ms === 0 && (
              <div className="wfc-inspector-hint">
                当前工具执行器会将 0 毫秒按默认 500 毫秒处理。
              </div>
            )}
            {oeMode === 'allow_codes' && allowCfg && (
              <TextField
                label="允许的退出码（逗号分隔）"
                value={(allowCfg.codes ?? []).join(',')}
                readOnly={readOnly}
                validate={v =>
                  parseExitCodes(v) ? null : '请输入完整整数，用英文逗号分隔；末尾不要加逗号'
                }
                onCommit={v => {
                  const codes = parseExitCodes(v)
                  if (codes) return onPatch({ on_error: { allow_codes: { ...allowCfg, codes } } })
                }}
              />
            )}
          </details>

          <div className="wfc-inspector-section">动作参数</div>

          {kind === 'tool' && (
            <>
              <ToolField
                label="工具名"
                value={String(d.tool ?? '')}
                tools={tools}
                readOnly={readOnly}
                autoFocus={focusToolId === step.id}
                required
                onCommit={v => {
                  setToolChanged(true)
                  return patchActionKey('tool', v)
                }}
              />
              {tools === null && (
                <div className="wfc-inspector-hint" role="status">
                  工具信息加载失败，仍可手动编辑。
                  <button
                    type="button"
                    onClick={() => {
                      setTools(undefined)
                      void loadToolsOnce().then(setTools)
                    }}
                  >
                    重试加载
                  </button>
                </div>
              )}
              {toolChanged && (
                <div className="wfc-inspector-hint">
                  已保留原工具参数，请检查是否适用于新工具。
                  <button
                    type="button"
                    disabled={readOnly}
                    onClick={() => {
                      void patchActionKey('with', {})
                      setParameterReset(version => version + 1)
                      setToolChanged(false)
                    }}
                  >
                    清空参数重新填写
                  </button>
                </div>
              )}
              <ToolParameterForm
                resetVersion={parameterReset}
                schema={tools?.find(t => t.name === d.tool)?.input_schema}
                value={d.with}
                readOnly={readOnly}
                onChange={v => patchActionKey('with', v)}
                variables={variableCatalog.references.map(v => ({
                  name: v.name,
                  label: `${v.sourceLabel}${v.maybeUnset ? ' · 可能未赋值' : ''}`,
                }))}
              />
            </>
          )}
          {kind === 'call' && (
            <>
              <TextField
                label="目标工作流 workflow_id"
                value={String(d.call ?? '')}
                readOnly={readOnly}
                mono
                required
                onCommit={v => patchActionKey('call', v)}
              />
              <JsonField
                label="with（参数 JSON）"
                value={d.with}
                readOnly={readOnly}
                onCommit={v => patchActionKey('with', v)}
              />
            </>
          )}
          {kind === 'chat' && (
            <>
              <TextField
                label="对话内容"
                value={String(d.chat ?? '')}
                readOnly={readOnly}
                multiline
                references
                required
                onCommit={v => patchActionKey('chat', v)}
              />
              {models != null && models.length > 0 && (
                <label className="wfc-field">
                  <span className="wfc-field-label">模型（registry 模型 ID）</span>
                  <select
                    className="wfc-input"
                    value={
                      typeof chatWith.model === 'string'
                        ? `${typeof chatWith.provider === 'string' ? chatWith.provider : ''}::${chatWith.model}`
                        : ''
                    }
                    disabled={readOnly}
                    onChange={e => {
                      const [provider, ...modelParts] = e.target.value.split('::')
                      const model = modelParts.join('::')
                      const next = { ...chatWith }
                      if (model) next.model = model
                      else delete next.model
                      if (provider) next.provider = provider
                      else delete next.provider
                      patchActionKey('with', next)
                    }}
                  >
                    <option value="">默认（主模型）</option>
                    {models.map(m => (
                      <option key={`${m.provider}::${m.id}`} value={`${m.provider}::${m.id}`}>
                        {m.id} · {m.provider}
                      </option>
                    ))}
                    {typeof chatWith.model === 'string' &&
                      chatWith.model !== '' &&
                      !models.some(m => m.id === chatWith.model) && (
                        <option
                          value={`${typeof chatWith.provider === 'string' ? chatWith.provider : ''}::${chatWith.model}`}
                        >
                          {chatWith.model}（不在 registry）
                        </option>
                      )}
                  </select>
                </label>
              )}
              <TextField
                label="随机程度 temperature（空为默认）"
                value={chatWith.temperature != null ? String(chatWith.temperature) : ''}
                readOnly={readOnly}
                validate={v => numberError(v, { optional: true })}
                onCommit={v => {
                  if (!v.trim()) return patchChatWith('temperature', undefined)
                  const n = Number(v)
                  return patchChatWith('temperature', n)
                }}
              />
              <TextField
                label="最大输出长度 max_tokens（空为默认）"
                value={chatWith.max_tokens != null ? String(chatWith.max_tokens) : ''}
                readOnly={readOnly}
                validate={v =>
                  numberError(v, { integer: true, min: 1, max: 4294967295, optional: true })
                }
                onCommit={v => {
                  if (!v.trim()) return patchChatWith('max_tokens', undefined)
                  return patchChatWith('max_tokens', Number(v))
                }}
              />
              <JsonField
                label="其他对话选项（JSON）"
                value={d.with}
                readOnly={readOnly}
                onCommit={v => patchActionKey('with', v)}
              />
            </>
          )}
          {kind === 'script' && (
            <>
              <label className="wfc-field">
                <span className="wfc-field-label">运行环境 runtime</span>
                <select
                  className="wfc-input"
                  value={String((d.script as Record<string, unknown>)?.runtime ?? 'python')}
                  disabled={readOnly}
                  onChange={e =>
                    patchActionKey('script', { ...(d.script as object), runtime: e.target.value })
                  }
                >
                  <option value="python">Python</option>
                  <option value="node">Node.js</option>
                  <option value="ahk">AutoHotkey（仅 Windows）</option>
                  <option value="pwsh">PowerShell（pwsh）</option>
                  {!['python', 'node', 'ahk', 'pwsh'].includes(
                    String((d.script as Record<string, unknown>)?.runtime ?? 'python'),
                  ) && (
                    <option value={String((d.script as Record<string, unknown>)?.runtime)}>
                      {String((d.script as Record<string, unknown>)?.runtime)}
                      （当前执行器不支持，已保留）
                    </option>
                  )}
                </select>
              </label>
              <TextField
                label="脚本代码 code"
                value={String((d.script as Record<string, unknown>)?.code ?? '')}
                readOnly={readOnly}
                mono
                multiline
                references
                required
                onCommit={v => patchActionKey('script', { ...(d.script as object), code: v })}
              />
            </>
          )}
          {kind === 'if' && (
            <ConditionEditor
              value={(d.if as { condition?: Condition })?.condition}
              readOnly={readOnly}
              onChange={c => patchActionKey('if', { ...(d.if as object), condition: c })}
            />
          )}
          {kind === 'assert' && (
            <>
              <ConditionEditor
                value={(d.assert as { condition?: Condition })?.condition}
                readOnly={readOnly}
                onChange={c => patchActionKey('assert', { ...(d.assert as object), condition: c })}
              />
              <TextField
                label="失败消息"
                value={String((d.assert as Record<string, unknown>)?.message ?? '')}
                readOnly={readOnly}
                onCommit={v =>
                  patchActionKey('assert', { ...(d.assert as object), message: v || undefined })
                }
              />
            </>
          )}
          {kind === 'loop' && <LoopEditor d={d} readOnly={readOnly} onPatch={patchActionKey} />}
          {kind === 'wait' && (
            <TextField
              label="等待目标"
              value={String(d.wait ?? '')}
              readOnly={readOnly}
              onCommit={v => patchActionKey('wait', v)}
            />
          )}
          {kind === 'mcp' && (
            <>
              <TextField
                label="MCP 服务器 server"
                value={String((d.mcp as Record<string, unknown>)?.server ?? '')}
                readOnly={readOnly}
                mono
                required
                onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), server: v })}
              />
              <TextField
                label="MCP 工具 tool"
                value={String((d.mcp as Record<string, unknown>)?.tool ?? '')}
                readOnly={readOnly}
                mono
                required
                onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), tool: v })}
              />
              <JsonField
                label="with（参数 JSON）"
                value={(d.mcp as Record<string, unknown>)?.with}
                readOnly={readOnly}
                onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), with: v })}
              />
            </>
          )}
          {kind === 'sleep' && (
            <TextField
              label="时长（秒）"
              value={String(d.sleep ?? 1)}
              readOnly={readOnly}
              validate={v => numberError(v, { min: 0 })}
              onCommit={v => {
                return patchActionKey('sleep', Number(v))
              }}
            />
          )}
          {(kind === 'seq' || kind === 'break' || kind === 'continue') && (
            <div className="wfc-inspector-hint">
              {kind === 'seq' ? '顺序容器：子步骤在画布子层编辑（双击节点进入）' : '无参数'}
            </div>
          )}
          {kind === 'custom' && (
            <div className="wfc-inspector-hint wfc-inspector-hint--warn">
              旧格式节点（不兼容 V2），画布只读。请通过 AI 通道重建为 V2 工作流。
            </div>
          )}

          {(['tool', 'script', 'chat', 'mcp'].includes(kind) || step.capture) && (
            <>
              <div className="wfc-inspector-section">输出变量</div>
              <CaptureField
                value={step.capture ?? ''}
                readOnly={readOnly}
                onCommit={v => onPatch({ capture: v || undefined })}
              />
              {!!captureConsumers?.length && (
                <div className="wfc-inspector-hint">
                  引用此输出的步骤（重命名后需手动更新引用）：
                  {captureConsumers.map(consumer => (
                    <button
                      key={consumer.id}
                      type="button"
                      className="wfc-reference-link"
                      onClick={() => onLocateReference?.(consumer.id)}
                      title={consumer.id}
                    >
                      {consumer.name}
                    </button>
                  ))}
                </div>
              )}
              {!['tool', 'script', 'chat', 'mcp'].includes(kind) && (
                <div className="wfc-inspector-hint">
                  已保留历史变量设置；此类步骤不直接产生输出。
                </div>
              )}
            </>
          )}

          {lastOutput && lastOutput.length > 0 && (
            <>
              <div className="wfc-inspector-section">最近输出</div>
              <pre className="wfc-output-preview">{lastOutput.join('\n')}</pre>
            </>
          )}
        </div>
      </aside>
    </VariableContext.Provider>
  )
}

function CaptureField({
  value,
  readOnly,
  onCommit,
}: {
  value: string
  readOnly: boolean
  onCommit: (v: string) => unknown
}) {
  const { catalog } = useContext(VariableContext)
  const draft = useInspectorDraft('保存输出到变量', value, {
    onCommit,
    validate: v => (!v || isVariableName(v) ? null : '请输入合法的变量名'),
  })
  return (
    <div
      className="wfc-field"
      data-draft-field="保存输出到变量"
      onBlur={() => {
        void draft.commit()
      }}
    >
      <span className="wfc-field-label">保存输出到变量</span>
      <VariablePicker
        value={draft.text}
        onChange={draft.setText}
        mode="capture"
        catalog={catalog}
        readOnly={readOnly}
      />
      {draft.text !== value && value && (
        <span className="wfc-inspector-hint">
          修改名称不会自动替换其他步骤的引用，请同时检查引用此变量的步骤。
        </span>
      )}
      {draft.error && <span className="wfc-field-error">{draft.error}</span>}
    </div>
  )
}

function ReferenceField({
  value,
  readOnly,
  label,
  onCommit,
}: {
  value: string
  readOnly?: boolean
  label: string
  onCommit: (value: string) => unknown
}) {
  const variables = useContext(VariableContext)
  const draft = useInspectorDraft(label, value, { onCommit })
  return (
    <div
      className="wfc-field"
      data-draft-field={label}
      onBlur={() => {
        void draft.commit()
      }}
    >
      <VariablePicker
        label={label}
        value={draft.text}
        readOnly={readOnly}
        onChange={draft.setText}
        mode="reference"
        catalog={variables.catalog}
        onConfigureInput={variables.onConfigureInput}
      />
    </div>
  )
}

function LoopEditor({
  d,
  readOnly,
  onPatch,
}: {
  d: Record<string, unknown>
  readOnly: boolean
  onPatch: (key: string, v: unknown) => void
}) {
  const def = (d.loop ?? {}) as Record<string, unknown>
  const mode = def.for_each
    ? 'for_each'
    : def.repeat != null
      ? 'repeat'
      : def.until
        ? 'until'
        : 'repeat'
  const modeDrafts = useRef<Record<string, unknown>>({})
  modeDrafts.current[mode] = def[mode]
  const setMode = (m: string) => {
    const next = switchLoopMode(def, m, modeDrafts.current)
    onPatch('loop', next)
  }
  return (
    <>
      <label className="wfc-field">
        <span className="wfc-field-label">循环方式</span>
        <select
          className="wfc-input"
          value={mode}
          disabled={readOnly}
          onChange={e => setMode(e.target.value)}
        >
          <option value="for_each">遍历列表</option>
          <option value="repeat">固定次数</option>
          <option value="until">直到条件满足</option>
        </select>
      </label>
      {mode === 'for_each' && (
        <>
          <div className="wfc-field">
            <span className="wfc-field-label">遍历列表 items（变量名）</span>
            <ReferenceField
              label="遍历列表"
              value={(() => {
                const items = (def.for_each as Record<string, unknown>)?.items
                return items && typeof items === 'object' && 'var' in items
                  ? String(items.var)
                  : typeof items === 'string'
                    ? items
                    : ''
              })()}
              readOnly={readOnly}
              onCommit={v =>
                onPatch('loop', {
                  ...def,
                  for_each: { ...(def.for_each as object), items: { var: v } },
                })
              }
            />
          </div>
          <TextField
            label="当前项变量 as"
            value={String((def.for_each as Record<string, unknown>)?.as ?? 'item')}
            readOnly={readOnly}
            mono
            onCommit={v =>
              onPatch('loop', {
                ...def,
                for_each: { ...(def.for_each as object), as: v || 'item' },
              })
            }
          />
        </>
      )}
      {mode === 'repeat' && (
        <TextField
          label="重复次数"
          value={String(def.repeat ?? 1)}
          readOnly={readOnly}
          validate={v => numberError(v, { integer: true, min: 0, max: 4294967295 })}
          onCommit={v => {
            return onPatch('loop', { ...def, repeat: Number(v) })
          }}
        />
      )}
      {mode === 'until' && (
        <ConditionEditor
          value={def.until as Condition}
          readOnly={readOnly}
          onChange={c => onPatch('loop', { ...def, until: c })}
        />
      )}
      <TextField
        label="最多循环次数 max（默认 100）"
        value={String(def.max ?? 100)}
        readOnly={readOnly}
        validate={v => numberError(v, { integer: true, min: 1, max: 4294967295 })}
        onCommit={v => {
          return onPatch('loop', { ...def, max: Number(v) })
        }}
      />
    </>
  )
}
