import { editorText, editorValidationText } from './editorText'
/**
 * Inspector.tsx — 右侧参数面板（设计文档 2.5/3.3）
 * 按 kind 生成表单；字段级编辑一律经 onPatch → update_fields IrEditOp 写回。
 * 表单约束前置 V9/V11 等规则（不允许输入非法值），结构校验由 validate.ts 呈现。
 */

import { createContext, useContext, useEffect, useId, useRef, useState, type ChangeEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { IconType, IconImage, IconChevronRight } from '../../ui/Icons'
import { Film as IconFilm } from 'lucide-react'
import type { WorkflowStep, Condition, VarRef, ToolSchema } from '../../core/types'
import { wfTools, listModels, type ModelInfo } from '../lib/api'
import { selectableModels, type CapabilityFilter } from '../lib/modelCapability'
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
import { useLanguage } from '../../locales'
import { NodeKindBadge } from './NodeKindBadge'
import { variableSourceLabel } from './presentation'
import { actionSummary } from './actionSummary'
import { editorFieldId, editorFieldLabel } from './editorFields'
import { WorkflowTraceContext } from './WorkflowTraceContext'
import { ExecutionTraceViewer } from './ExecutionTraceViewer'
import { PanelFrame } from './inspector/PanelFrame'
import { SectionCard } from './inspector/SectionCard'
import { FieldRow } from './inspector/FieldRow'
import { ConditionRow, JsonField as JsonFieldV2, NumberField, Segmented } from './inspector/controls'

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
  loopConditionCatalog?: VariableCatalog
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

/** 本地文本缓冲：失焦/回车才提交（避免每次击键都重投影）。
 *  v2 结构（T2a）：FieldRow（label + 必填 accent * + error/hint 位）+ 原生 control；
 *  references 变量插入、expand-editor 全屏编辑、validate、mono、multiline、
 *  画布草稿存储（fieldPath → data-draft-field 定位）全部保留。
 *  aria-label 承载完整可访问名（label + 必填后缀），与迁移前 label 元素文本一致——
 *  骨架变化不改变既有 label 查询契约。 */
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
  hint,
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
  /** 字段级内联错误文案（D2：空名称/必填缺失时展示，走 FieldRow error 位） */
  error?: string | null
  validate?: (value: string) => string | null
  references?: boolean
  /** 字段说明（v2：FieldRow hint 位；原文经 editorText 查表，缺省回退原文） */
  hint?: string
}) {
  const { t } = useLanguage()
  const variables = useContext(VariableContext)
  const fieldId = useId()
  const textRef = useRef<HTMLTextAreaElement>(null)
  const {
    text: draft,
    setText: setDraft,
    commit,
    error: draftError,
  } = useInspectorDraft(editorFieldId(label), value, {
    onCommit,
    validate:
      validate ??
      (required ? v => (v.trim() ? null : t('workflowEditor.field.requiredError')) : undefined),
  })
  const [expanded, setExpanded] = useState(false)
  const [variableQuery, setVariableQuery] = useState('')
  const validationError = draftError ?? validate?.(draft) ?? (draft === value ? error : null)
  const labelText = editorFieldLabel(label, t)
  const inputClass = `wfp-ctl${mono ? ' wfp-ctl--mono' : ''}${validationError ? ' wfp-ctl--invalid' : ''}`
  // 技术名 title 与完整可访问名（label + 必填后缀）落在 control 上——骨架变化不改变
  // 既有 label 查询契约（getByLabelText / getByRole name）
  const controlExtras = {
    title,
    'aria-label': required ? `${labelText}${t('workflowEditor.field.required')}` : labelText,
  }
  const control = multiline ? (
    <textarea
      id={fieldId}
      ref={textRef}
      className={inputClass}
      value={draft}
      rows={expanded ? 22 : mono ? 8 : 3}
      readOnly={readOnly}
      placeholder={placeholder}
      {...controlExtras}
      onChange={e => setDraft(e.target.value)}
      onBlur={commit}
    />
  ) : (
    <input
      id={fieldId}
      className={inputClass}
      value={draft}
      readOnly={readOnly}
      placeholder={placeholder}
      {...controlExtras}
      onChange={e => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={e => {
        if (e.key === 'Enter' && !e.nativeEvent.isComposing)
          (e.target as HTMLInputElement).blur()
      }}
    />
  )
  const picker =
    references && !readOnly ? (
      <VariablePicker
        value={variableQuery}
        mode="reference"
        catalog={variables.catalog}
        label={t('workflowCanvas.variable.insert', label)}
        onConfigureInput={variables.onConfigureInput}
        onSelectReference={name => {
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
    ) : null

  return (
    <FieldRow
      label={labelText}
      required={required}
      fieldPath={editorFieldId(label)}
      hint={hint ? editorText(hint, t) : undefined}
      error={validationError ? editorValidationText(validationError, t) : undefined}
    >
      <div className="wfp-tf">
        {multiline && (
          <button type="button" className="wfp-tf__expand" onClick={() => setExpanded(v => !v)}>
            {t(expanded ? 'workflowEditor.field.collapse' : 'workflowEditor.field.expand')}
          </button>
        )}
        {control}
        {picker}
      </div>
    </FieldRow>
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
  const { t } = useLanguage()
  const {
    text: draft,
    setText: setDraft,
    commit,
  } = useInspectorDraft(editorFieldId(label), value, {
    onCommit,
    validate: required ? v => (v.trim() ? null : editorText('请选择或输入工具名', t)) : undefined,
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
    <div className="wfc-combo" data-draft-field={editorFieldId(label)} ref={wrapRef}>
      <input
        ref={inputRef}
        className="wfp-ctl wfp-ctl--mono"
        value={draft}
        placeholder={editorText('搜索或输入工具名', t)}
        role="combobox"
        aria-label={editorFieldLabel(label, t)}
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
              <div className="wfc-combo-empty">{editorText('无匹配工具', t)}</div>
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
                    <span className="wfc-combo-name">
                      {editorText('使用输入值', t)} {it.text}
                    </span>
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
  const { t } = useLanguage()
  if (!schema) return null
  const items = parseRequired(schema)
  return (
    <div className="wfc-required-hint">
      {items.length === 0 ? (
        <span>{editorText('无必填参数', t)}</span>
      ) : (
        <>
          <span>{editorText('必填:', t)}</span>
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
 * 工具名字段（T2b）：注册表 combobox（>4 动态列表，S3 矩阵保留 Combobox）+ 必填参数提示，
 * 外包 FieldRow；tools === null（加载失败）或 readOnly 时退化为纯文本 TextField（已是 v2 结构）。
 */
function ToolField({
  label,
  value,
  tools,
  readOnly,
  autoFocus,
  onCommit,
  required,
  hint,
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
  /** 字段位提示（加载失败重试 / 参数保留清理，由调用方以 ReactNode 传入） */
  hint?: ReactNode
}) {
  const { t } = useLanguage()
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
    <FieldRow
      label={editorFieldLabel(label, t)}
      required={required}
      fieldPath={editorFieldId(label)}
      hint={hint}
    >
      <ToolCombobox
        label={label}
        value={value}
        tools={list}
        autoFocus={autoFocus}
        onCommit={onCommit}
        required={required}
      />
      <RequiredHint schema={list.find(t => t.name === value.trim())?.input_schema} />
    </FieldRow>
  )
}

// ── model 步骤：模型选择（只选不配；按模态筛选） ──

/** 选择值编码 `{provider}::{model}`（与 chat 下拉一致）；空串 = 用模型界面绑定的生成能力 */
function modelSelectionValue(models: ModelInfo[], modelWith: Record<string, unknown>): string {
  if (typeof modelWith.model !== 'string' || !modelWith.model) return ''
  const provider = typeof modelWith.provider === 'string' ? modelWith.provider : ''
  return `${provider}::${modelWith.model}`
}

/** 当前覆盖不在（按模态筛选后的）候选里时返回原值供「保留选项」展示，否则 null */
function modelSelectionFallback(
  models: ModelInfo[],
  modelWith: Record<string, unknown>,
  filter: CapabilityFilter | undefined,
): string | null {
  const current = modelSelectionValue(models, modelWith)
  if (!current) return null
  const candidates = selectableModels(models, filter).map(m => `${m.provider}::${m.id}`)
  return candidates.includes(current) ? null : current
}

/** 生成类型 → 能力过滤维度：image→image_generation，video→video_generation，text→不过滤。
 *  生成类两种维度在 modelCapability.ts 里共用 `supports_image_generation`（注册表只有
 *  一个生成能力声明字段），video 复用 image 过滤是既有约定，不新增注册表字段。 */
function modelCapabilityFilter(kind: string): CapabilityFilter | undefined {
  if (kind === 'image') return 'image_generation'
  if (kind === 'video') return 'video_generation'
  return undefined
}

// ── model 步骤：画面规格选项（值与后端 step_schema.json 白名单一一对应） ──

/** 生成类型候选：图标化分段三选一，无空白项（text 是缺省模态，也始终显示为可选项）。 */
const MODEL_KIND_OPTIONS = [
  { value: 'text', label: '文本', icon: <IconType size={13} /> },
  { value: 'image', label: '图片', icon: <IconImage size={13} /> },
  { value: 'video', label: '视频', icon: <IconFilm size={13} /> },
] as const

/** 图片尺寸校验：只校验正整数——实际上限由所选模型决定，越界由生成协议层报错 */
function imageDimensionError(value: string): string | null {
  return numberError(value, { integer: true, min: 1, optional: true })
}

interface SpecOption {
  value: string
  label: string
  /** 画面比例的 CSS aspect-ratio（图形化方块按真实宽高比渲染） */
  shape?: string
}

/** image 全量 7 项比例 */
const IMAGE_RATIO_OPTIONS: SpecOption[] = [
  { value: '1:1', label: '1:1', shape: '1:1' },
  { value: '4:3', label: '4:3', shape: '4:3' },
  { value: '3:4', label: '3:4', shape: '3:4' },
  { value: '3:2', label: '3:2', shape: '3:2' },
  { value: '2:3', label: '2:3', shape: '2:3' },
  { value: '16:9', label: '16:9', shape: '16:9' },
  { value: '9:16', label: '9:16', shape: '9:16' },
]
/** video 只暴露最常用的 3 项（其余比例视频生成端不稳定，需要时走高级 params 逃生舱） */
const VIDEO_RATIO_OPTIONS: SpecOption[] = IMAGE_RATIO_OPTIONS.filter(o =>
  ['1:1', '16:9', '9:16'].includes(o.value),
)
const MODEL_COUNT_OPTIONS: SpecOption[] = ['1', '2', '3', '4'].map(value => ({ value, label: value }))
const MODEL_DURATION_OPTIONS: SpecOption[] = ['5', '6', '8', '10'].map(value => ({
  value,
  label: `${value}s`,
}))

/** 规格缺省值：只决定「未设置时控件显示哪一项」，不写回 with（后端同样不补缺省，
 *  不设的字段不落盘）——显示默认与生成端自身默认一致时才敢这么做。 */
const MODEL_SPEC_DEFAULTS = {
  ratio: { image: '1:1', video: '16:9' },
  count: '1',
  duration: '6',
} as const

/** 规格单选组（T2a）：T1 Segmented（radiogroup + chip，S3 枚举矩阵）。
 *  当前值不在候选里时（跨模态残留）末尾追加静态保留 chip——只展示、不可交互、
 *  不触发 patch（与迁移前「保留展示不清空」语义一致）。 */
function SpecChips({
  label,
  fieldPath,
  options,
  value,
  onChange,
  readOnly,
  hint,
}: {
  label: string
  fieldPath: string
  options: SpecOption[]
  value: string
  onChange: (value: string) => void
  readOnly?: boolean
  hint?: string
}) {
  const { t } = useLanguage()
  const labelText = editorFieldLabel(label, t)
  const retained = value !== '' && !options.some(option => option.value === value)
  return (
    <FieldRow
      label={labelText}
      fieldPath={fieldPath}
      hint={hint ? editorText(hint, t) : undefined}
    >
      <div className="wfp-spec">
        <Segmented
          label={labelText}
          options={options.map(option => ({
            value: option.value,
            label: option.label,
            icon: option.shape ? (
              <span className="wfc-ratio-shape" style={{ aspectRatio: option.shape }} />
            ) : undefined,
          }))}
          value={value}
          onChange={onChange}
          disabled={readOnly}
        />
        {retained && (
          <span
            className="wfp-seg__item is-active wfp-seg__item--static"
            role="radio"
            aria-checked
            aria-disabled="true"
          >
            {value}
          </span>
        )}
      </div>
    </FieldRow>
  )
}

/** 数字字段（T2a）：T1 NumberField（单位后缀 / 步进 / 空=默认）+ 画布草稿存储桥接。
 *  blur / Enter / 方向键步进 → 写回 store 并提交，validate 与提交语义和迁移前
 *  TextField 完全一致（校验失败保留脏草稿、错误经 FieldRow error 位展示）。 */
function NumberFieldDraft({
  label,
  value,
  onCommit,
  readOnly,
  validate,
  min,
  max,
  integer,
  help,
  hint,
}: {
  label: string
  value: string
  onCommit: (v: string) => unknown
  readOnly?: boolean
  validate?: (v: string) => string | null
  min?: number
  max?: number
  integer?: boolean
  /** 技术名（temperature 等）：help 图标 hover 展示 */
  help?: string
  hint?: string
}) {
  const { t } = useLanguage()
  const { text, setText, commit, error } = useInspectorDraft(editorFieldId(label), value, {
    onCommit,
    validate,
  })
  return (
    <FieldRow
      label={editorFieldLabel(label, t)}
      help={help}
      fieldPath={editorFieldId(label)}
      hint={hint ? editorText(hint, t) : undefined}
      error={error ? editorValidationText(error, t) : undefined}
    >
      <NumberField
        value={text}
        min={min}
        max={max}
        integer={integer}
        disabled={readOnly}
        invalid={!!error}
        label={editorFieldLabel(label, t)}
        onDraftChange={setText}
        onCommit={v => {
          setText(v)
          void commit()
        }}
      />
    </FieldRow>
  )
}

/** JSON 逃生舱（T2a）：T1 JsonField（editable：校验徽标 + 等宽编辑）+ 草稿存储桥接。
 *  validate = JSON.parse（与迁移前本地 JsonField 一致），blur 提交；
 *  旧展开/收起按钮随 T1 editable 形态取消（textarea 可手动拖高）。 */
function JsonEscapeField({
  label,
  value,
  onCommit,
  readOnly,
  hint,
}: {
  label: string
  value: unknown
  onCommit: (v: unknown) => void
  readOnly?: boolean
  hint?: string
}) {
  const { t } = useLanguage()
  const text = JSON.stringify(value ?? {}, null, 2)
  const { text: draft, setText, commit, error } = useInspectorDraft(editorFieldId(label), text, {
    validate: v => {
      try {
        JSON.parse(v)
        return null
      } catch {
        return t('workflowEditor.field.jsonError')
      }
    },
    onCommit: v => onCommit(JSON.parse(v)),
  })
  return (
    <FieldRow
      label={editorFieldLabel(label, t)}
      fieldPath={editorFieldId(label)}
      hint={hint ? editorText(hint, t) : undefined}
      error={error ? editorValidationText(error, t) : undefined}
    >
      <JsonFieldV2
        value={draft}
        editable
        disabled={readOnly}
        onCommit={v => {
          setText(v)
          void commit()
        }}
      />
    </FieldRow>
  )
}

/** 原生 select 字段行（S3：枚举 >4 / 动态列表）。
 *  aria-label 保持迁移前「label 包裹 select」的可访问名——既有 findByLabelText 契约不变。 */
function SelectField({
  label,
  value,
  onChange,
  disabled,
  children,
  hint,
  help,
  fieldPath,
}: {
  label: string
  value: string
  onChange: (e: ChangeEvent<HTMLSelectElement>) => void
  disabled?: boolean
  children: ReactNode
  hint?: ReactNode
  help?: string
  fieldPath?: string
}) {
  return (
    <FieldRow label={label} hint={hint} help={help} fieldPath={fieldPath}>
      <select
        className="wfp-ctl"
        aria-label={label}
        value={value}
        disabled={disabled}
        onChange={onChange}
      >
        {children}
      </select>
    </FieldRow>
  )
}

/** 选择模型：一次 patch 同时写 model + provider（精确路由对）；空值删键回退绑定能力 */
function setModelSelection(
  value: string,
  modelWith: Record<string, unknown>,
  patchActionKey: (key: string, v: unknown) => unknown,
): void {
  const next = { ...modelWith }
  if (!value) {
    delete next.model
    delete next.provider
  } else {
    const [provider, ...rest] = value.split('::')
    next.model = rest.join('::')
    if (provider) next.provider = provider
    else delete next.provider
  }
  void patchActionKey('with', next)
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

/** 条件操作数（T2b）：值类型 select + 字面量输入 / 变量选择，行内形态（ConditionRow 单元格）。
 *  字面量输入自行持有 draft（data-draft-field 挂wrapper：focusEditorField 要在容器内找控件），
 *  不再借 TextField.hideLabel——旧分支随之删除。 */
function LiteralOperand({
  label,
  value,
  readOnly,
  onChange,
}: {
  label: string
  value: string
  readOnly?: boolean
  onChange: (v: string) => void
}) {
  const { t } = useLanguage()
  const { text, setText, commit } = useInspectorDraft(editorFieldId(label), value, {
    onCommit: onChange,
  })
  return (
    <span className="wfp-cond-literal" data-draft-field={editorFieldId(label)}>
      <input
        className="wfp-ctl"
        aria-label={editorFieldLabel(label, t)}
        value={text}
        readOnly={readOnly}
        placeholder={editorText('字面量值', t)}
        onChange={e => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={e => {
          if (e.key === 'Enter' && !e.nativeEvent.isComposing)
            (e.target as HTMLInputElement).blur()
        }}
      />
    </span>
  )
}

function ConditionOperand({
  index,
  operand,
  readOnly,
  onChange,
}: {
  index: number
  operand: VarRef | undefined
  readOnly?: boolean
  onChange: (v: VarRef) => void
}) {
  const { t } = useLanguage()
  const isVar = !!operand && typeof operand === 'object' && 'var' in operand
  const text = isVar
    ? String((operand as { var: string }).var)
    : typeof operand === 'string'
      ? operand
      : ''
  return (
    <div className="wfp-cond-operand">
      <select
        className="wfp-ctl wfp-cond-operand__kind"
        aria-label={editorText(index === 0 ? '左侧值类型' : '右侧值类型', t)}
        value={isVar ? 'var' : 'lit'}
        disabled={readOnly}
        onChange={e => onChange(e.target.value === 'var' ? { var: text } : text)}
      >
        <option value="lit">{editorText('固定值', t)}</option>
        <option value="var">{editorText('变量', t)}</option>
      </select>
      {isVar ? (
        <ReferenceField
          value={text}
          readOnly={readOnly}
          label={index === 0 ? '左侧变量' : '右侧变量'}
          onCommit={v => onChange({ var: v })}
          hideLabel
        />
      ) : (
        <LiteralOperand
          label={index === 0 ? '左侧固定值' : '右侧固定值'}
          value={text}
          readOnly={readOnly}
          onChange={onChange}
        />
      )}
    </div>
  )
}

function ConditionEditor({
  fieldPath,
  value,
  onChange,
  readOnly,
}: {
  fieldPath: string
  value: Condition | undefined
  onChange: (c: Condition) => void
  readOnly?: boolean
}) {
  const { t } = useLanguage()
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
  // 比较符 select 两种模态都在场（always 时与「固定结果」上下排列），
  // 供既有查询契约 getByLabelText('条件') 稳定成立
  const condOpSelect = (
    <select
      className="wfp-ctl"
      aria-label={editorText('条件', t)}
      value={op}
      disabled={readOnly}
      onChange={e => setOp(e.target.value)}
    >
      {COND_OPS.map(([k, label]) => (
        <option key={k} value={k}>
          {editorText(label, t)} ({k})
        </option>
      ))}
    </select>
  )
  return (
    <div className="wfp-cond-editor" data-field-path={fieldPath} data-draft-field={fieldPath}>
      {op === 'always' ? (
        <>
          <SelectField
            label={editorText('条件', t)}
            value={op}
            disabled={readOnly}
            onChange={e => setOp(e.target.value)}
          >
            {COND_OPS.map(([k, label]) => (
              <option key={k} value={k}>
                {editorText(label, t)} ({k})
              </option>
            ))}
          </SelectField>
          <SelectField
            label={editorText('固定结果', t)}
            value={String((value as { always?: boolean })?.always ?? true)}
            disabled={readOnly}
            onChange={e => onChange({ always: e.target.value === 'true' })}
          >
            <option value="true">{editorText('真（满足条件）', t)}</option>
            <option value="false">{editorText('假（不满足条件）', t)}</option>
          </SelectField>
        </>
      ) : (
        <ConditionRow
          left={
            count > 0 ? (
              <ConditionOperand
                index={0}
                operand={operands[0]}
                readOnly={readOnly}
                onChange={v => setOperand(0, v)}
              />
            ) : null
          }
          op={condOpSelect}
          right={
            count > 1 ? (
              <ConditionOperand
                index={1}
                operand={operands[1]}
                readOnly={readOnly}
                onChange={v => setOperand(1, v)}
              />
            ) : null
          }
        />
      )}
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
  loopConditionCatalog,
  onConfigureInput,
  captureConsumers,
  onLocateReference,
  onPatch,
  onPatchAction,
  onClose,
}: InspectorProps) {
  const { t } = useLanguage()
  const traceContext = useContext(WorkflowTraceContext)
  const [traceOpen, setTraceOpen] = useState(false)
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

  // model 步骤 with（ModelOpts）字段级写入：与 chat 同款合并语义——
  // 只改目标键，未列出的键（含未知字段）原样保留；切模态不清空其他字段。
  const modelWith = (d.with && typeof d.with === 'object' ? d.with : {}) as Record<string, unknown>
  const patchModelWith = (key: string, v: unknown) => {
    const next = { ...modelWith }
    if (v === undefined) delete next[key]
    else next[key] = v
    return patchActionKey('with', next)
  }
  const modelKind = typeof modelWith.kind === 'string' ? modelWith.kind : 'text'

  // 模型候选：按生成类型走 modelCapability 的 selectableModels（image→image_generation，
  // video→video_generation，text→不过滤）；候选为空时给出去模型界面绑定能力的指引。
  const modelCandidates = selectableModels(models ?? [], modelCapabilityFilter(modelKind))
  const modelRetained =
    Array.isArray(models) && models.length > 0
      ? modelSelectionFallback(models, modelWith, modelCapabilityFilter(modelKind))
      : null

  // params 逃生舱内的键级读写：宽高 / 返回格式 / 清晰度没有具名字段，
  // 键名与取值由所选提供商的生成协议决定
  // （schema 未新增），只能落在 params 里；undefined = 删键。
  const modelParamsObject = (
    modelWith.params && typeof modelWith.params === 'object' && !Array.isArray(modelWith.params)
      ? modelWith.params
      : {}
  ) as Record<string, unknown>
  const modelParamsValue = (key: string): string => {
    const value = modelParamsObject[key]
    return typeof value === 'string' || typeof value === 'number' ? String(value) : ''
  }
  const patchModelParamsKey = (key: string, v: unknown) => {
    const next: Record<string, unknown> = { ...modelParamsObject }
    if (v === undefined || v === '') delete next[key]
    else next[key] = v
    return patchModelWith('params', Object.keys(next).length ? next : undefined)
  }

  return (
    <VariableContext.Provider value={{ catalog: variableCatalog, onConfigureInput }}>
      <>
        <PanelFrame
          className="wfc-inspector"
          title={step.name || step.id}
          badge={<NodeKindBadge kind={kind} />}
          summary={actionSummary(step, t)}
          width={visibleWidth}
          onWidthPersist={persistWidth}
          onClose={onClose}
        >
          {/* ① 基础（S1 固定顺序）：名称 / 步骤说明（说明从高级分区挪入） */}
          <SectionCard title={editorText('基础', t)}>
            <TextField
              label="名称"
              value={step.name}
              readOnly={readOnly}
              onCommit={v => onPatch({ name: v })}
              required
              error={
                !readOnly && !step.name.trim()
                  ? editorText('名称必填：尚未命名无法保存/通过校验', t)
                  : undefined
              }
            />
            <TextField
              label="步骤说明"
              value={step.description ?? ''}
              readOnly={readOnly}
              multiline
              onCommit={v => onPatch({ description: v })}
            />
          </SectionCard>

          {/* ② 动作主体：按 kind 的核心参数（本期范围外 kind 保持原渲染） */}
          <SectionCard title={editorText('动作主体', t)}>

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
                hint={
                  tools === null || toolChanged ? (
                    <>
                      {tools === null && (
                        <div role="status">
                          {editorText('工具信息加载失败，仍可手动编辑。', t)}
                          <button
                            type="button"
                            onClick={() => {
                              setTools(undefined)
                              void loadToolsOnce().then(setTools)
                            }}
                          >
                            {editorText('重试加载', t)}
                          </button>
                        </div>
                      )}
                      {toolChanged && (
                        <div>
                          {editorText('已保留原工具参数，请检查是否适用于新工具。', t)}
                          <button
                            type="button"
                            disabled={readOnly}
                            onClick={() => {
                              void patchActionKey('with', {})
                              setParameterReset(version => version + 1)
                              setToolChanged(false)
                            }}
                          >
                            {editorText('清空参数重新填写', t)}
                          </button>
                        </div>
                      )}
                    </>
                  ) : undefined
                }
              />
              <ToolParameterForm
                resetVersion={parameterReset}
                schema={tools?.find(t => t.name === d.tool)?.input_schema}
                value={d.with}
                readOnly={readOnly}
                onChange={v => patchActionKey('with', v)}
                variables={variableCatalog.references.map(v => ({
                  name: v.name,
                  label: `${t('workflowCanvas.variable.name')}: ${v.name} · ${t('workflowCanvas.variable.source')}: ${variableSourceLabel(v, t)}${v.maybeUnset ? ` · ${t('workflowCanvas.variable.maybeUnset')}` : ''}`,
                }))}
              />
            </>
          )}
          {kind === 'call' && (
            <TextField
              label="目标工作流 workflow_id"
              value={String(d.call ?? '')}
              readOnly={readOnly}
              mono
              required
              title="workflow_id"
              onCommit={v => patchActionKey('call', v)}
            />
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
                <SelectField
                  label={editorText('模型', t)}
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
                  <option value="">{editorText('默认（主模型）', t)}</option>
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
                        {chatWith.model}
                        {editorText('（不在 registry）', t)}
                      </option>
                    )}
                </SelectField>
              )}
              {/* 文案治理：技术名进 help 图标，「空为默认」说明进 hint */}
              <NumberFieldDraft
                label="随机程度"
                help="temperature"
                value={chatWith.temperature != null ? String(chatWith.temperature) : ''}
                readOnly={readOnly}
                validate={v => numberError(v, { optional: true })}
                onCommit={v => {
                  if (!v.trim()) return patchChatWith('temperature', undefined)
                  const n = Number(v)
                  return patchChatWith('temperature', n)
                }}
                hint="越低越稳定；留空使用模型默认"
              />
              <NumberFieldDraft
                label="最大输出长度"
                help="max_tokens"
                integer
                min={1}
                max={4294967295}
                value={chatWith.max_tokens != null ? String(chatWith.max_tokens) : ''}
                readOnly={readOnly}
                validate={v =>
                  numberError(v, { integer: true, min: 1, max: 4294967295, optional: true })
                }
                onCommit={v => {
                  if (!v.trim()) return patchChatWith('max_tokens', undefined)
                  return patchChatWith('max_tokens', Number(v))
                }}
                hint="单位 token；留空使用模型默认"
              />
              {/* 「其他对话选项（JSON）」移至高级分区（S1/S3：主区零 JSON 裸框） */}
            </>
          )}
          {kind === 'model' && (
            <>
              <TextField
                label="提示词（任务描述）"
                value={String(d.model ?? '')}
                readOnly={readOnly}
                multiline
                references
                required
                onCommit={v => patchActionKey('model', v)}
                hint={
                  modelKind === 'video'
                    ? editorText('写动作和镜头怎么动，不要描写静态状态。', t)
                    : undefined
                }
              />
              {/* 生成类型：必选三选一，无空白项（Segmented，S3 枚举 ≤4） */}
              <FieldRow
                label={editorFieldLabel('生成类型', t)}
                required
                fieldPath="/do/with/kind"
                hint={editorText('切换类型不会清空已填写的其他字段。', t)}
              >
                <Segmented
                  label={editorFieldLabel('生成类型', t)}
                  options={MODEL_KIND_OPTIONS.map(option => ({
                    value: option.value,
                    label: editorText(option.label, t),
                    icon: option.icon,
                  }))}
                  value={modelKind}
                  onChange={v => patchModelWith('kind', v)}
                  disabled={readOnly}
                />
              </FieldRow>
              {/* 画面规格：按模态显示，全部选项化控件（无裸输入框、无空白项） */}
              {modelKind !== 'text' && (
                <>
                  <SpecChips
                    label="画面比例"
                    fieldPath="/do/with/ratio"
                    options={modelKind === 'video' ? VIDEO_RATIO_OPTIONS : IMAGE_RATIO_OPTIONS}
                    value={
                      typeof modelWith.ratio === 'string' && modelWith.ratio
                        ? modelWith.ratio
                        : modelKind === 'video'
                          ? MODEL_SPEC_DEFAULTS.ratio.video
                          : MODEL_SPEC_DEFAULTS.ratio.image
                    }
                    readOnly={readOnly}
                    onChange={v => patchModelWith('ratio', v)}
                  />
                  {modelKind === 'image' && (
                    <SpecChips
                      label="生成数量"
                      fieldPath="/do/with/count"
                      options={MODEL_COUNT_OPTIONS}
                      value={
                        typeof modelWith.count === 'number'
                          ? String(modelWith.count)
                          : MODEL_SPEC_DEFAULTS.count
                      }
                      readOnly={readOnly}
                      onChange={v => patchModelWith('count', Number(v))}
                    />
                  )}
                  {modelKind === 'video' && (
                    <>
                      <SpecChips
                        label="时长"
                        fieldPath="/do/with/duration"
                        options={MODEL_DURATION_OPTIONS}
                        hint="新手建议 5–8 秒"
                        value={
                          typeof modelWith.duration === 'number'
                            ? String(modelWith.duration)
                            : MODEL_SPEC_DEFAULTS.duration
                        }
                        readOnly={readOnly}
                        onChange={v => patchModelWith('duration', Number(v))}
                      />
                      <TextField
                        label="首帧图"
                        title="first_frame"
                        value={
                          typeof modelWith.first_frame === 'string' ? modelWith.first_frame : ''
                        }
                        readOnly={readOnly}
                        references
                        onCommit={v => patchModelWith('first_frame', v.trim() ? v : undefined)}
                        hint={editorText('留空 = 文生视频；可插入变量引用上游产物。', t)}
                      />
                    </>
                  )}
                </>
              )}
              {/* 模型选择：只选不配——配置在模型界面完成；候选按模态经 selectableModels 筛选 */}
              <FieldRow
                label={editorFieldLabel('选择模型（留空用绑定能力）', t)}
                fieldPath="/do/with/model"
                hint={
                  <>
                    <div>
                      {editorText(
                        '模型配置在模型界面完成，此处只选择使用哪个模型；同名模型跨服务商时下拉条目已带 provider。',
                        t,
                      )}
                    </div>
                    {modelKind === 'video' && (
                      <div>
                        {editorText('视频生成能力以模型界面的「视频生成」绑定为准。', t)}
                      </div>
                    )}
                    {(modelKind === 'video' || modelKind === 'image') && (
                      <div>
                        {editorText(
                          modelKind === 'video'
                            ? '视频生成通常需要 1–5 分钟，超时上限可在高级里调整。'
                            : '同步返回，通常数秒',
                          t,
                        )}
                      </div>
                    )}
                  </>
                }
              >
                {modelCandidates.length > 0 ? (
                  <select
                    className="wfp-ctl"
                    aria-label={editorFieldLabel('选择模型（留空用绑定能力）', t)}
                    value={modelSelectionValue(modelCandidates, modelWith)}
                    disabled={readOnly}
                    onChange={e => setModelSelection(e.target.value, modelWith, patchActionKey)}
                  >
                    <option value="">{editorText('默认（模型界面绑定的生成能力）', t)}</option>
                    {modelCandidates.map(m => (
                      <option key={`${m.provider}::${m.id}`} value={`${m.provider}::${m.id}`}>
                        {m.id} · {m.provider}
                      </option>
                    ))}
                    {modelRetained && (
                      <option value={modelRetained}>
                        {typeof modelWith.model === 'string' ? modelWith.model : ''}
                        {editorText('（不在当前模态的模型列表中，已保留）', t)}
                      </option>
                    )}
                  </select>
                ) : (
                  <span className="wfp-frow__hint">
                    {editorText(
                      models === undefined
                        ? '正在加载模型列表…'
                        : models === null
                          ? '模型列表不可用；留空或保留原值时将使用模型界面绑定的生成能力。'
                          : '生成能力候选为空：去模型界面 → 图像音频模型 → 图片生成 / 视频生成 绑定模型后，这里才会列出候选。',
                      t,
                    )}
                  </span>
                )}
              </FieldRow>
            </>
          )}
          {kind === 'script' && (
            <>
              <SelectField
                label={editorText('运行环境 runtime', t)}
                value={String((d.script as Record<string, unknown>)?.runtime ?? 'python')}
                disabled={readOnly}
                onChange={e =>
                  patchActionKey('script', { ...(d.script as object), runtime: e.target.value })
                }
              >
                <option value="python">Python</option>
                <option value="node">Node.js</option>
                <option value="ahk">{editorText('AutoHotkey（仅 Windows）', t)}</option>
                <option value="pwsh">PowerShell（pwsh）</option>
                {!['python', 'node', 'ahk', 'pwsh'].includes(
                  String((d.script as Record<string, unknown>)?.runtime ?? 'python'),
                ) && (
                  <option value={String((d.script as Record<string, unknown>)?.runtime)}>
                    {String((d.script as Record<string, unknown>)?.runtime)}
                    {editorText('（当前执行器不支持，已保留）', t)}
                  </option>
                )}
              </SelectField>
              <TextField
                label="脚本代码 code"
                value={String((d.script as Record<string, unknown>)?.code ?? '')}
                readOnly={readOnly}
                mono
                multiline
                references
                required
                title="code"
                onCommit={v => patchActionKey('script', { ...(d.script as object), code: v })}
              />
            </>
          )}
          {kind === 'if' && (
            <ConditionEditor
              fieldPath="/do/if/condition"
              value={(d.if as { condition?: Condition })?.condition}
              readOnly={readOnly}
              onChange={c => patchActionKey('if', { ...(d.if as object), condition: c })}
            />
          )}
          {kind === 'assert' && (
            <>
              <ConditionEditor
                fieldPath="/do/assert/condition"
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
          {kind === 'loop' && (
            <LoopEditor
              d={d}
              readOnly={readOnly}
              onPatch={patchActionKey}
              conditionCatalog={loopConditionCatalog}
            />
          )}
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
                title="server"
                onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), server: v })}
              />
              <TextField
                label="MCP 工具 tool"
                value={String((d.mcp as Record<string, unknown>)?.tool ?? '')}
                readOnly={readOnly}
                mono
                required
                title="tool"
                onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), tool: v })}
              />
            </>
          )}
          {kind === 'sleep' && (
            <NumberFieldDraft
              label="时长（秒）"
              integer
              min={0}
              value={String(d.sleep ?? 1)}
              readOnly={readOnly}
              validate={v => numberError(v, { min: 0 })}
              onCommit={v => {
                return patchActionKey('sleep', Number(v))
              }}
            />
          )}
          {(kind === 'seq' || kind === 'break' || kind === 'continue') && (
            <div className="wfp-frow__hint">
              {kind === 'seq'
                ? editorText('顺序容器：子步骤在画布子层编辑（双击节点进入）', t)
                : editorText('无参数', t)}
            </div>
          )}
          {kind === 'custom' && (
            <div className="wfp-frow__hint wfp-frow__hint--warn">
              {editorText('旧格式节点（不兼容 V2），画布只读。请通过 AI 通道重建为 V2 工作流。', t)}
            </div>
          )}

          </SectionCard>

          {/* ③ 高级（S1 固定顺序；T2b 合并原 model 模态高级）：
              model 模态专属字段（text 采样三件套 / image 宽高返回格式 / video 清晰度等待上限）
              + 步骤标识 / 超时 / 失败处理 / JSON 逃生舱，默认折叠 */}
          <details className="wfp-adv">
            <summary className="wfp-adv__summary">
              {editorText('高级设置', t)}
              {oeMode !== 'abort' || step.timeout_secs != null
                ? editorText('（已自定义失败处理或超时）', t)
                : ''}
              <span className="wfp-adv__chev">
                <IconChevronRight size={11} />
              </span>
            </summary>
            <div className="wfp-adv__body">
              {kind === 'model' && (
                <>
                {modelKind === 'text' && (
                  <>
                    <TextField
                      label="系统提示词"
                      value={typeof modelWith.system === 'string' ? modelWith.system : ''}
                      readOnly={readOnly}
                      multiline
                      onCommit={v => patchModelWith('system', v || undefined)}
                    />
                    <NumberFieldDraft
                      label="随机程度"
                      help="temperature"
                      value={modelWith.temperature != null ? String(modelWith.temperature) : ''}
                      readOnly={readOnly}
                      validate={v => numberError(v, { optional: true })}
                      onCommit={v => {
                        if (!v.trim()) return patchModelWith('temperature', undefined)
                        return patchModelWith('temperature', Number(v))
                      }}
                    />
                    <NumberFieldDraft
                      label="最大输出长度"
                      help="max_tokens"
                      integer
                      min={1}
                      max={4294967295}
                      value={modelWith.max_tokens != null ? String(modelWith.max_tokens) : ''}
                      readOnly={readOnly}
                      validate={v =>
                        numberError(v, { integer: true, min: 1, max: 4294967295, optional: true })
                      }
                      onCommit={v => {
                        if (!v.trim()) return patchModelWith('max_tokens', undefined)
                        return patchModelWith('max_tokens', Number(v))
                      }}
                    />
                  </>
                )}
                {modelKind === 'image' && (
                  <>
                    <NumberFieldDraft
                      label="图片宽度 width"
                      help="width"
                      integer
                      min={1}
                      value={modelParamsValue('width')}
                      readOnly={readOnly}
                      validate={v => imageDimensionError(v)}
                      onCommit={v => patchModelParamsKey('width', v.trim() ? Number(v) : undefined)}
                      hint="尺寸上限由所选模型决定，超出时协议层会返回明确错误。"
                    />
                    <NumberFieldDraft
                      label="图片高度 height"
                      help="height"
                      integer
                      min={1}
                      value={modelParamsValue('height')}
                      readOnly={readOnly}
                      validate={v => imageDimensionError(v)}
                      onCommit={v => patchModelParamsKey('height', v.trim() ? Number(v) : undefined)}
                    />
                    <SelectField
                      label={editorFieldLabel('返回格式', t)}
                      fieldPath="/do/with/params"
                      value={modelParamsValue('response_format')}
                      disabled={readOnly}
                      onChange={e =>
                        patchModelParamsKey('response_format', e.target.value || undefined)
                      }
                    >
                      <option value="">{editorText('默认（url）', t)}</option>
                      <option value="url">url</option>
                      <option value="base64">base64</option>
                    </SelectField>
                  </>
                )}
                {modelKind === 'video' && (
                  <>
                    <TextField
                      label="画面清晰度"
                      title="resolution"
                      placeholder="如 1080P"
                      value={modelParamsValue('resolution')}
                      readOnly={readOnly}
                      onCommit={v => patchModelParamsKey('resolution', v.trim() ? v.trim() : undefined)}
                      hint="留空使用生成端默认清晰度。"
                    />
                    <NumberFieldDraft
                      label="最长等待时间 timeout_secs"
                      help="timeout_secs"
                      integer
                      min={30}
                      max={600}
                      value={modelWith.timeout_secs != null ? String(modelWith.timeout_secs) : ''}
                      readOnly={readOnly}
                      validate={v =>
                        numberError(v, { integer: true, min: 30, max: 600, optional: true })
                      }
                      onCommit={v => {
                        if (!v.trim()) return patchModelWith('timeout_secs', undefined)
                        return patchModelWith('timeout_secs', Number(v))
                      }}
                    />
                  </>
                )}
                <JsonEscapeField
                  label="参数透传（JSON，逃生舱）"
                  value={modelWith.params}
                  readOnly={readOnly}
                  onCommit={v => patchModelWith('params', v)}
                  hint="上面的可视化选项会写进这里，手改需自负。"
                />
                </>
              )}

              <TextField
                label={idReferenced ? '步骤标识 ID（有历史记录，修改需确认）' : '步骤标识 ID'}
                value={step.id}
                readOnly={readOnly}
                mono
                required
                onCommit={v => onPatch({ id: v })}
              />
              <NumberFieldDraft
                label="超时秒数（空为执行器默认）"
                integer
                min={1}
                value={step.timeout_secs != null ? String(step.timeout_secs) : ''}
                readOnly={readOnly}
                validate={v => numberError(v, { integer: true, min: 1, optional: true })}
                onCommit={v => {
                  return onPatch({ timeout_secs: v.trim() ? Number(v) : undefined })
                }}
                hint={
                  kind === 'script'
                    ? '脚本默认超时为 120 秒，可在上方修改。'
                    : !['seq', 'if', 'mcp', 'script'].includes(kind)
                      ? `${editorText('此类步骤暂不使用上述超时字段。', t)}${
                          kind === 'tool'
                            ? editorText('工具的超时由具体工具决定。', t)
                            : editorText('已保留历史配置，执行时使用对应动作的行为。', t)
                        }`
                      : undefined
                }
              />
              <SelectField
                label={editorText('失败后怎么办', t)}
                help="on_error"
                value={oeMode}
                disabled={readOnly}
                onChange={e => {
                  const m = e.target.value
                  if (m === 'retry') onPatch({ on_error: { retry: { max: 3, backoff_ms: 500 } } })
                  else if (m === 'allow_codes')
                    onPatch({ on_error: { allow_codes: { codes: [0] } } })
                  else onPatch({ on_error: m as OnErrorValue })
                }}
                hint={
                  kind === 'tool' && oeMode === 'abort'
                    ? editorText('工具执行器仍可能先自动重试，再终止工作流。', t)
                    : kind !== 'tool' && (oeMode === 'retry' || oeMode === 'allow_codes')
                      ? editorText('已保留历史配置；当前步骤执行器不支持此失败处理设置。', t)
                      : undefined
                }
              >
                <option value="abort">{editorText('终止工作流（默认）', t)}</option>
                <option value="skip">{editorText('跳过失败步骤，继续执行', t)}</option>
                <option value="retry" disabled={kind !== 'tool'}>
                  {editorText('重试（工具步骤）', t)}
                </option>
                <option value="allow_codes" disabled={kind !== 'tool'}>
                  {editorText('允许指定退出码（工具步骤）', t)}
                </option>
              </SelectField>
              {oeMode === 'retry' && retryCfg && (
                <>
                  <NumberFieldDraft
                    label="重试次数"
                    integer
                    min={0}
                    max={4294967295}
                    value={String(retryCfg.max)}
                    readOnly={readOnly}
                    validate={v => numberError(v, { integer: true, min: 0, max: 4294967295 })}
                    onCommit={v => {
                      const n = Number(v)
                      return onPatch({ on_error: { retry: { ...retryCfg, max: n } } })
                    }}
                  />
                  <NumberFieldDraft
                    label="重试间隔（毫秒）"
                    integer
                    min={0}
                    value={String(retryCfg.backoff_ms ?? 500)}
                    readOnly={readOnly}
                    validate={v => numberError(v, { integer: true, min: 0 })}
                    onCommit={v => {
                      const n = Number(v)
                      return onPatch({ on_error: { retry: { ...retryCfg, backoff_ms: n } } })
                    }}
                    hint={
                      retryCfg?.backoff_ms === 0
                        ? '当前工具执行器会将 0 毫秒按默认 500 毫秒处理。'
                        : undefined
                    }
                  />
                </>
              )}
              {oeMode === 'allow_codes' && allowCfg && (
                <TextField
                  label="允许的退出码（逗号分隔）"
                  value={(allowCfg.codes ?? []).join(',')}
                  readOnly={readOnly}
                  validate={v =>
                    parseExitCodes(v)
                      ? null
                      : editorText('请输入完整整数，用英文逗号分隔；末尾不要加逗号', t)
                  }
                  onCommit={v => {
                    const codes = parseExitCodes(v)
                    if (codes) return onPatch({ on_error: { allow_codes: { ...allowCfg, codes } } })
                  }}
                />
              )}
              {/* JSON 逃生舱统一在高级分区（S1/S3：主区零 JSON 裸框）：chat / call / mcp */}
              {kind === 'chat' && (
                <JsonEscapeField
                  label="其他对话选项（JSON）"
                  value={d.with}
                  readOnly={readOnly}
                  onCommit={v => patchActionKey('with', v)}
                />
              )}
              {kind === 'call' && (
                <JsonEscapeField
                  label="with（参数 JSON）"
                  value={d.with}
                  readOnly={readOnly}
                  onCommit={v => patchActionKey('with', v)}
                />
              )}
              {kind === 'mcp' && (
                <JsonEscapeField
                  label="with（参数 JSON）"
                  value={(d.mcp as Record<string, unknown>)?.with}
                  readOnly={readOnly}
                  onCommit={v => patchActionKey('mcp', { ...(d.mcp as object), with: v })}
                />
              )}
            </div>
          </details>

          {/* 输出变量 */}
          {(['tool', 'script', 'chat', 'model', 'mcp'].includes(kind) || step.capture) && (
            <SectionCard title={t('workflowCanvas.variable.outputSection')}>
              <CaptureField
                value={step.capture ?? ''}
                readOnly={readOnly}
                onCommit={v => onPatch({ capture: v || undefined })}
              />
              {!!captureConsumers?.length && (
                <div className="wfp-frow__hint">
                  {t('workflowCanvas.variable.consumers')}
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
              {!['tool', 'script', 'chat', 'model', 'mcp'].includes(kind) && (
                <div className="wfp-frow__hint">
                  {editorText('已保留历史变量设置；此类步骤不直接产生输出。', t)}
                </div>
              )}
            </SectionCard>
          )}

          {lastOutput && lastOutput.length > 0 && (
            <SectionCard title={editorText('最近输出', t)}>
              <pre className="wfc-output-preview">{lastOutput.join('\n')}</pre>
            </SectionCard>
          )}
          {traceContext && (
            <details open={traceOpen} onToggle={event => setTraceOpen(event.currentTarget.open)}>
              <summary>{t('workflowEditor.problem.history')}</summary>
              {traceOpen && <ExecutionTraceViewer stepId={step.id} />}
            </details>
          )}
        </PanelFrame>
        {/* 面板宽度测量锚点：display:none 不参与布局；其 parent 即画布容器——
            与迁移前 panelRef 挂在 aside 上测得的同一个元素（多 Inspector 并存时
            各自测量自己的宿主，querySelector 会串面板） */}
        <span ref={panelRef} hidden aria-hidden="true" />
      </>
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
  const { t } = useLanguage()
  const { catalog } = useContext(VariableContext)
  const draft = useInspectorDraft('/capture', value, {
    onCommit,
    validate: v => (!v || isVariableName(v) ? null : t('workflowCanvas.variable.invalid')),
  })
  return (
    <FieldRow
      label={t('workflowCanvas.variable.capture')}
      fieldPath="/capture"
      hint={draft.text !== value && value ? t('workflowCanvas.variable.renameHint') : undefined}
      error={draft.error ?? undefined}
    >
      {/* picker 不传可见 label：aria-label 回退键与本行标签同值，
          可见标签由 FieldRow 承担，combobox 可访问名不变 */}
      <div
        onBlur={() => {
          void draft.commit()
        }}
      >
        <VariablePicker
          value={draft.text}
          onChange={draft.setText}
          mode="capture"
          catalog={catalog}
          readOnly={readOnly}
        />
      </div>
    </FieldRow>
  )
}

function ReferenceField({
  value,
  readOnly,
  label,
  onCommit,
  hideLabel,
}: {
  value: string
  readOnly?: boolean
  label: string
  onCommit: (value: string) => unknown
  /** 行内复合布局用：隐藏 VariablePicker 的可见标签（对齐 TextField.hideLabel） */
  hideLabel?: boolean
}) {
  const { t } = useLanguage()
  const variables = useContext(VariableContext)
  const draft = useInspectorDraft(editorFieldId(label), value, { onCommit })
  return (
    <div
      className="wfc-field"
      data-draft-field={editorFieldId(label)}
      onBlur={() => {
        void draft.commit()
      }}
    >
      <VariablePicker
        label={hideLabel ? undefined : editorFieldLabel(label, t)}
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
  conditionCatalog,
}: {
  d: Record<string, unknown>
  readOnly: boolean
  onPatch: (key: string, v: unknown) => void
  conditionCatalog?: VariableCatalog
}) {
  const { t } = useLanguage()
  const variables = useContext(VariableContext)
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
      <SelectField
        label={editorText('循环方式', t)}
        value={mode}
        disabled={readOnly}
        onChange={e => setMode(e.target.value)}
      >
        <option value="for_each">{editorText('遍历列表', t)}</option>
        <option value="repeat">{editorText('固定次数', t)}</option>
        <option value="until">{editorText('直到条件满足', t)}</option>
      </SelectField>
      {mode === 'for_each' && (
        <>
          <FieldRow
            label={editorFieldLabel('遍历列表', t)}
            fieldPath={editorFieldId('遍历列表')}
          >
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
              hideLabel
            />
          </FieldRow>
          <TextField
            label="当前项变量 as"
            value={String((def.for_each as Record<string, unknown>)?.as ?? 'item')}
            readOnly={readOnly}
            mono
            title="as"
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
        <NumberFieldDraft
          label="重复次数"
          integer
          min={0}
          max={4294967295}
          value={String(def.repeat ?? 1)}
          readOnly={readOnly}
          validate={v => numberError(v, { integer: true, min: 0, max: 4294967295 })}
          onCommit={v => {
            return onPatch('loop', { ...def, repeat: Number(v) })
          }}
        />
      )}
      {mode === 'until' && (
        <VariableContext.Provider
          value={{ ...variables, catalog: conditionCatalog ?? variables.catalog }}
        >
          <ConditionEditor
            fieldPath="/do/loop/until"
            value={def.until as Condition}
            readOnly={readOnly}
            onChange={c => onPatch('loop', { ...def, until: c })}
          />
        </VariableContext.Provider>
      )}
      <NumberFieldDraft
        label="最多循环次数 max（默认 100）"
        help="max"
        integer
        min={1}
        max={4294967295}
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
