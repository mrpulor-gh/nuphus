/**
 * Inspector.tsx — 右侧参数面板（设计文档 2.5/3.3）
 * 按 kind 生成表单；字段级编辑一律经 onPatch → update_fields IrEditOp 写回。
 * 表单约束前置 V9/V11 等规则（不允许输入非法值），结构校验由 validate.ts 呈现。
 */

import { useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
import type { WorkflowStep, Condition, VarRef, ToolSchema } from '../../core/types'
import { wfTools, listModels, type ModelInfo } from '../lib/api'
import { stepKind } from './projection'
import { skeletonFromSchema } from './toolSkeleton'

interface InspectorProps {
  step: WorkflowStep
  readOnly: boolean
  /** run_history 引用了本步骤 id（改 id 需确认，由父层 checkOp 裁决） */
  idReferenced: boolean
  /** 最近输出（StepRunOutput 最近 3 行 / run_history output_summary） */
  lastOutput?: string[]
  /** 新建节点待聚焦的工具输入框（step.id 匹配时 autofocus，消费一次） */
  focusToolId?: string | null
  onPatch: (patch: Partial<WorkflowStep>) => void
  onPatchAction: (action: WorkflowStep['do']) => void
  onClose: () => void
}

// ── 工具注册表：进入画布加载一次并缓存；失败 → null（静默回退纯文本模式） ──
let toolsCache: ToolSchema[] | null | undefined
let toolsInflight: Promise<ToolSchema[] | null> | null = null

/** 加载 wf_tools（模块级单例缓存；ToolPalette 复用，避免二次请求） */
export function loadToolsOnce(): Promise<ToolSchema[] | null> {
  if (toolsCache !== undefined) return Promise.resolve(toolsCache)
  if (!toolsInflight) {
    toolsInflight = wfTools()
      .then(t => {
        toolsCache = t ?? []
        return toolsCache
      })
      .catch(() => {
        toolsCache = null
        return null
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
  label, value, onCommit, readOnly, placeholder, mono, multiline, title, required,
}: {
  label: string
  value: string
  onCommit: (v: string) => void
  readOnly?: boolean
  placeholder?: string
  mono?: boolean
  multiline?: boolean
  title?: string
  /** 语法必填字段：label 追加「（必填）」标记（对齐 compiler.rs 校验） */
  required?: boolean
}) {
  const [draft, setDraft] = useState(value)
  useEffect(() => setDraft(value), [value])
  const commit = () => {
    if (draft !== value) onCommit(draft)
  }
  return (
    <label className="wfc-field" title={title}>
      <span className="wfc-field-label">
        {label}
        {required && <em className="wfc-required-mark">（必填）</em>}
      </span>
      {multiline ? (
        <textarea
          className={`wfc-input${mono ? ' wfc-input--mono' : ''}`}
          value={draft}
          rows={mono ? 8 : 3}
          readOnly={readOnly}
          placeholder={placeholder}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
        />
      ) : (
        <input
          className={`wfc-input${mono ? ' wfc-input--mono' : ''}`}
          value={draft}
          readOnly={readOnly}
          placeholder={placeholder}
          onChange={e => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={e => {
            if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
          }}
        />
      )}
    </label>
  )
}

type ComboItem = { kind: 'custom'; text: string } | { kind: 'tool'; tool: ToolSchema }

/** 可搜索工具 combobox：注册表驱动过滤 + 键盘导航 + 手输兜底（未匹配值也可提交） */
function ToolCombobox({
  label, value, tools, autoFocus, onCommit, required,
}: {
  label: string
  value: string
  tools: ToolSchema[]
  autoFocus?: boolean
  onCommit: (v: string) => void
  /** 语法必填字段：label 追加「（必填）」标记 */
  required?: boolean
}) {
  const [draft, setDraft] = useState(value)
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  // 向上翻转：Inspector body 是滚动容器（overflow-y:auto）会裁切下拉，
  // 字段靠近底部时向下展开最多露 2 项——空间不足改向上展开
  const [dropUp, setDropUp] = useState(false)
  const wrapRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const focusedOnceRef = useRef(false)
  useEffect(() => setDraft(value), [value])
  const openMenu = () => {
    const rect = wrapRef.current?.getBoundingClientRect()
    setDropUp(!!rect && rect.bottom + 250 > window.innerHeight)
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
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false)
    }
    window.addEventListener('pointerdown', onPointerDown, true)
    return () => window.removeEventListener('pointerdown', onPointerDown, true)
  }, [open])

  const pick = (item: ComboItem) => {
    const v = item.kind === 'custom' ? item.text : item.tool.name
    setDraft(v)
    setOpen(false)
    if (v !== value) onCommit(v)
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
        onChange={e => {
          setDraft(e.target.value)
          openMenu()
          setActive(0)
        }}
        onBlur={() => {
          setOpen(false)
          if (draft !== value) onCommit(draft)
        }}
        onKeyDown={e => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault()
              if (!open) openMenu()
            else if (items.length > 0) {
              setActive(i => (e.key === 'ArrowDown' ? (i + 1) % items.length : (i - 1 + items.length) % items.length))
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
      {open && (
        <div className={`wfc-combo-menu${dropUp ? ' wfc-combo-menu--up' : ''}`}>
          {items.length === 0 ? (
            <div className="wfc-combo-empty">无匹配工具</div>
          ) : (
            items.map((it, i) => (
              <button
                key={it.kind === 'custom' ? '__custom__' : it.tool.name}
                type="button"
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
                    {it.tool.description && <span className="wfc-combo-desc">{it.tool.description}</span>}
                  </>
                )}
              </button>
            ))
          )}
        </div>
      )}
    </div>
  )
}

/** 从 input_schema（JSON Schema）容错解析必填参数：缺失/非对象/无 required → 无必填 */
function parseRequired(schema: Record<string, unknown> | undefined): { name: string; desc?: string }[] {
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
  label, value, tools, readOnly, autoFocus, onCommit, required,
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
    return <TextField label={label} value={value} readOnly={readOnly} mono onCommit={onCommit} required={required} />
  }
  const list = tools ?? []
  return (
    <>
      <ToolCombobox label={label} value={value} tools={list} autoFocus={autoFocus} onCommit={onCommit} required={required} />
      <RequiredHint schema={list.find(t => t.name === value.trim())?.input_schema} />
    </>
  )
}

function JsonField({
  label, value, onCommit, readOnly,
}: {
  label: string
  value: unknown
  onCommit: (v: unknown) => void
  readOnly?: boolean
}) {
  const text = JSON.stringify(value ?? {}, null, 2)
  const [draft, setDraft] = useState(text)
  const [err, setErr] = useState<string | null>(null)
  useEffect(() => {
    setDraft(JSON.stringify(value ?? {}, null, 2))
    setErr(null)
  }, [value])
  const commit = () => {
    try {
      const parsed = JSON.parse(draft || '{}')
      setErr(null)
      if (draft !== text) onCommit(parsed)
    } catch (e) {
      setErr(`JSON 解析失败: ${(e as Error).message}`)
    }
  }
  return (
    <label className="wfc-field">
      <span className="wfc-field-label">{label}</span>
      <textarea
        className="wfc-input wfc-input--mono"
        value={draft}
        rows={6}
        readOnly={readOnly}
        onChange={e => setDraft(e.target.value)}
        onBlur={commit}
      />
      {err && <span className="wfc-field-error">{err}</span>}
    </label>
  )
}

const COND_OPS: [string, string][] = [
  ['equals', '等于'], ['not_equals', '不等于'], ['contains', '包含'], ['starts_with', '前缀是'],
  ['regex', '正则匹配'], ['not_empty', '非空'], ['empty', '为空'], ['gt', '>'],
  ['lt', '<'], ['gte', '≥'], ['lte', '≤'], ['always', '恒真'],
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
  value, onChange, readOnly,
}: {
  value: Condition | undefined
  onChange: (c: Condition) => void
  readOnly?: boolean
}) {
  const op = condOpOf(value)
  const operands = condOperandsOf(value)
  const setOp = (nextOp: string) => {
    if (nextOp === 'always') onChange({ always: true } as Condition)
    else if (nextOp === 'not_empty' || nextOp === 'empty') onChange({ [nextOp]: '' } as unknown as Condition)
    else onChange({ [nextOp]: ['', ''] } as unknown as Condition)
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
        <select className="wfc-input" value={op} disabled={readOnly} onChange={e => setOp(e.target.value)}>
          {COND_OPS.map(([k, label]) => (
            <option key={k} value={k}>{label}（{k}）</option>
          ))}
        </select>
      </label>
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
            >
              <option value="lit">字面量</option>
              <option value="var">变量</option>
            </select>
            <input
              className="wfc-input wfc-input--mono"
              value={text}
              readOnly={readOnly}
              placeholder={isVar ? '变量名（可带 a.b 路径）' : '字面量值'}
              onChange={e => setOperand(i, isVar ? { var: e.target.value } : e.target.value)}
            />
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

export function Inspector({ step, readOnly, idReferenced, lastOutput, focusToolId, onPatch, onPatchAction, onClose }: InspectorProps) {
  const kind = stepKind(step)
  const d = step.do as Record<string, unknown>
  // 工具注册表（模块级缓存，加载一次；失败 → null 回退纯文本）
  const [tools, setTools] = useState<ToolSchema[] | null | undefined>(toolsCache)
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
    onPatchAction({ ...d, [key]: v } as WorkflowStep['do'])
  }

  // chat 步骤 with（ChatOpts）字段级写入：undefined = 删除该键（回退后端默认）
  const chatWith = (d.with && typeof d.with === 'object' ? d.with : {}) as Record<string, unknown>
  const patchChatWith = (key: string, v: unknown) => {
    const next = { ...chatWith }
    if (v === undefined) delete next[key]
    else next[key] = v
    patchActionKey('with', next)
  }

  return (
    <aside className="wfc-inspector">
      <div className="wfc-inspector-head">
        <span className="wfc-inspector-title">{step.name || step.id}</span>
        <span className="wfc-badge">{kind}</span>
        <button type="button" className="wfc-icon-btn" onClick={onClose} title="关闭（Esc）">
          <X size={14} />
        </button>
      </div>

      <div className="wfc-inspector-body">
        <TextField label="名称" value={step.name} readOnly={readOnly} onCommit={v => onPatch({ name: v })} required />
        <TextField
          label={idReferenced ? 'ID（有历史运行记录，修改需确认）' : 'ID'}
          value={step.id}
          readOnly={readOnly}
          mono
          required
          onCommit={v => onPatch({ id: v })}
        />
        <TextField label="描述" value={step.description ?? ''} readOnly={readOnly} multiline onCommit={v => onPatch({ description: v })} />
        <TextField
          label="capture（输出写入变量）"
          value={step.capture ?? ''}
          readOnly={readOnly}
          mono
          placeholder="变量名，如 wins"
          onCommit={v => onPatch({ capture: v || undefined })}
        />
        <TextField
          label="超时（秒，空为不限）"
          value={step.timeout_secs != null ? String(step.timeout_secs) : ''}
          readOnly={readOnly}
          onCommit={v => {
            const n = Number(v)
            onPatch({ timeout_secs: v && Number.isFinite(n) && n > 0 ? Math.floor(n) : undefined })
          }}
        />

        <label className="wfc-field">
          <span className="wfc-field-label">on_error</span>
          <select
            className="wfc-input"
            value={oeMode}
            disabled={readOnly}
            onChange={e => {
              const m = e.target.value
              if (m === 'retry') onPatch({ on_error: { retry: { max: 3, backoff_ms: 500 } } })
              else if (m === 'allow_codes') onPatch({ on_error: { allow_codes: { codes: [0] } } })
              else onPatch({ on_error: m as OnErrorValue })
            }}
          >
            <option value="abort">abort（终止）</option>
            <option value="skip">skip（跳过继续）</option>
            <option value="retry">retry（重试）</option>
            <option value="allow_codes">allow_codes（退出码白名单）</option>
          </select>
        </label>
        {oeMode === 'retry' && retryCfg && (
          <div className="wfc-field-row">
            <TextField
              label="重试次数"
              value={String(retryCfg.max)}
              readOnly={readOnly}
              onCommit={v => {
                const n = Math.max(0, Math.floor(Number(v) || 0))
                onPatch({ on_error: { retry: { ...retryCfg, max: n } } })
              }}
            />
            <TextField
              label="间隔 ms"
              value={String(retryCfg.backoff_ms ?? 500)}
              readOnly={readOnly}
              onCommit={v => {
                const n = Math.max(0, Math.floor(Number(v) || 0))
                onPatch({ on_error: { retry: { ...retryCfg, backoff_ms: n } } })
              }}
            />
          </div>
        )}
        {oeMode === 'allow_codes' && allowCfg && (
          <TextField
            label="允许的退出码（逗号分隔）"
            value={(allowCfg.codes ?? []).join(',')}
            readOnly={readOnly}
            onCommit={v => {
              const codes = v.split(',').map(s => Number(s.trim())).filter(n => Number.isFinite(n))
              onPatch({ on_error: { allow_codes: { codes } } })
            }}
          />
        )}

        <div className="wfc-inspector-section">参数（{kind}）</div>

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
                // 切换工具：仅当 with 为空时按 input_schema 预填骨架（不覆盖用户已填参数）
                const schema = Array.isArray(tools)
                  ? tools.find(t => t.name === v.trim())?.input_schema
                  : undefined
                const withEmpty =
                  d.with == null ||
                  (typeof d.with === 'object' &&
                    !Array.isArray(d.with) &&
                    Object.keys(d.with as object).length === 0)
                if (schema && withEmpty) {
                  onPatchAction({ ...d, tool: v, with: skeletonFromSchema(schema) } as WorkflowStep['do'])
                } else {
                  patchActionKey('tool', v)
                }
              }}
            />
            <JsonField label="with（参数 JSON）" value={d.with} readOnly={readOnly} onCommit={v => patchActionKey('with', v)} />
          </>
        )}
        {kind === 'call' && (
          <>
            <TextField label="目标 workflow_id" value={String(d.call ?? '')} readOnly={readOnly} mono required onCommit={v => patchActionKey('call', v)} />
            <JsonField label="with（参数 JSON）" value={d.with} readOnly={readOnly} onCommit={v => patchActionKey('with', v)} />
          </>
        )}
        {kind === 'chat' && (
          <>
            <TextField label="对话内容" value={String(d.chat ?? '')} readOnly={readOnly} multiline required onCommit={v => patchActionKey('chat', v)} />
            {models != null && models.length > 0 && (
              <label className="wfc-field">
                <span className="wfc-field-label">模型（registry 模型 ID）</span>
                <select
                  className="wfc-input"
                  value={typeof chatWith.model === 'string' ? chatWith.model : ''}
                  disabled={readOnly}
                  onChange={e => patchChatWith('model', e.target.value || undefined)}
                >
                  <option value="">默认（主模型）</option>
                  {models.map(m => (
                    <option key={m.id} value={m.id}>
                      {m.id} · {m.provider}
                    </option>
                  ))}
                  {typeof chatWith.model === 'string' &&
                    chatWith.model !== '' &&
                    !models.some(m => m.id === chatWith.model) && (
                      <option value={chatWith.model}>{chatWith.model}（不在 registry，按裸模型名回退主模型）</option>
                    )}
                </select>
              </label>
            )}
            <TextField
              label="temperature（空为默认）"
              value={chatWith.temperature != null ? String(chatWith.temperature) : ''}
              readOnly={readOnly}
              onCommit={v => {
                if (!v.trim()) return patchChatWith('temperature', undefined)
                const n = Number(v)
                if (Number.isFinite(n)) patchChatWith('temperature', n)
              }}
            />
            <TextField
              label="max_tokens（空为默认）"
              value={chatWith.max_tokens != null ? String(chatWith.max_tokens) : ''}
              readOnly={readOnly}
              onCommit={v => {
                if (!v.trim()) return patchChatWith('max_tokens', undefined)
                const n = Math.floor(Number(v))
                if (Number.isFinite(n) && n > 0) patchChatWith('max_tokens', n)
              }}
            />
            <JsonField label="with（ChatOpts JSON）" value={d.with} readOnly={readOnly} onCommit={v => patchActionKey('with', v)} />
          </>
        )}
        {kind === 'script' && (
          <>
            <label className="wfc-field">
              <span className="wfc-field-label">runtime</span>
              <select
                className="wfc-input"
                value={String((d.script as Record<string, unknown>)?.runtime ?? 'python')}
                disabled={readOnly}
                onChange={e => patchActionKey('script', { ...(d.script as object), runtime: e.target.value })}
              >
                <option value="python">python</option>
                <option value="node">node</option>
                <option value="shell">shell</option>
                <option value="powershell">powershell</option>
              </select>
            </label>
            <TextField
              label="code"
              value={String((d.script as Record<string, unknown>)?.code ?? '')}
              readOnly={readOnly}
              mono
              multiline
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
              onCommit={v => patchActionKey('assert', { ...(d.assert as object), message: v || undefined })}
            />
          </>
        )}
        {kind === 'loop' && (
          <LoopEditor d={d} readOnly={readOnly} onPatch={patchActionKey} />
        )}
        {kind === 'wait' && (
          <TextField label="等待目标" value={String(d.wait ?? '')} readOnly={readOnly} onCommit={v => patchActionKey('wait', v)} />
        )}
        {kind === 'mcp' && (
          <>
            <TextField label="server" value={String((d.mcp as Record<string, unknown>)?.server ?? '')} readOnly={readOnly} mono required onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), server: v })} />
            <TextField label="tool" value={String((d.mcp as Record<string, unknown>)?.tool ?? '')} readOnly={readOnly} mono required onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), tool: v })} />
            <JsonField label="with（参数 JSON）" value={(d.mcp as Record<string, unknown>)?.with} readOnly={readOnly} onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), with: v })} />
          </>
        )}
        {kind === 'sleep' && (
          <TextField
            label="时长（秒）"
            value={String(d.sleep ?? 1)}
            readOnly={readOnly}
            onCommit={v => {
              const n = Number(v)
              if (Number.isFinite(n) && n >= 0) patchActionKey('sleep', n)
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

        {lastOutput && lastOutput.length > 0 && (
          <>
            <div className="wfc-inspector-section">最近输出</div>
            <pre className="wfc-output-preview">{lastOutput.join('\n')}</pre>
          </>
        )}
      </div>
    </aside>
  )
}

function LoopEditor({
  d, readOnly, onPatch,
}: {
  d: Record<string, unknown>
  readOnly: boolean
  onPatch: (key: string, v: unknown) => void
}) {
  const def = (d.loop ?? {}) as Record<string, unknown>
  const mode = def.for_each ? 'for_each' : def.repeat != null ? 'repeat' : def.until ? 'until' : 'repeat'
  const setMode = (m: string) => {
    const next: Record<string, unknown> = { max: def.max ?? 100, do: def.do ?? [] }
    if (m === 'for_each') next.for_each = { items: '', as: 'item' }
    else if (m === 'repeat') next.repeat = 1
    else next.until = { always: false }
    onPatch('loop', next)
  }
  return (
    <>
      <label className="wfc-field">
        <span className="wfc-field-label">循环方式</span>
        <select className="wfc-input" value={mode} disabled={readOnly} onChange={e => setMode(e.target.value)}>
          <option value="for_each">for_each（遍历变量）</option>
          <option value="repeat">repeat（固定次数）</option>
          <option value="until">until（直到条件）</option>
        </select>
      </label>
      {mode === 'for_each' && (
        <>
          <TextField
            label="items（变量名）"
            value={(() => {
              const items = (def.for_each as Record<string, unknown>)?.items
              return items && typeof items === 'object' && 'var' in items ? String(items.var) : typeof items === 'string' ? items : ''
            })()}
            readOnly={readOnly}
            mono
            onCommit={v => onPatch('loop', { ...def, for_each: { ...(def.for_each as object), items: { var: v } } })}
          />
          <TextField
            label="as（item 变量名）"
            value={String((def.for_each as Record<string, unknown>)?.as ?? 'item')}
            readOnly={readOnly}
            mono
            onCommit={v => onPatch('loop', { ...def, for_each: { ...(def.for_each as object), as: v || 'item' } })}
          />
        </>
      )}
      {mode === 'repeat' && (
        <TextField
          label="重复次数"
          value={String(def.repeat ?? 1)}
          readOnly={readOnly}
          onCommit={v => {
            const n = Math.max(0, Math.floor(Number(v) || 0))
            onPatch('loop', { ...def, repeat: n })
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
        label="max（上限，默认 100）"
        value={String(def.max ?? 100)}
        readOnly={readOnly}
        onCommit={v => {
          const n = Math.max(1, Math.floor(Number(v) || 100))
          onPatch('loop', { ...def, max: n })
        }}
      />
    </>
  )
}