import { editorText, editorValidationText } from './editorText'
import { useContext, useEffect, useId, useRef, useState } from 'react'
import { InspectorDraftContext, useInspectorDraft } from './inspectorDrafts'
import { useLanguage } from '../../locales'
import './tool-parameter-form.css'

type Schema = Record<string, unknown>
type Parameters = Record<string, unknown>
export interface ToolParameterDraft {
  mode: 'form' | 'json'
  raw: string
  fields: Record<string, string>
  expressions: Record<string, boolean>
  errors: Record<string, string>
}

export interface ToolParameterFormProps {
  schema?: Schema
  value: unknown
  readOnly?: boolean
  onChange: (value: Parameters) => void
  /** Parent may retain this across node switching; invalid text is never serialized into IR. */
  draft?: ToolParameterDraft
  onDraftChange?: (draft: ToolParameterDraft) => void
  variables?: { name: string; label?: string; expression?: string }[]
  /** An explicit user reset discards invalid parameter text as well as serialized values. */
  resetVersion?: number
}

function record(value: unknown): value is Parameters {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function hasOwn(value: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key)
}

const complexKeys = [
  '$ref',
  'allOf',
  'anyOf',
  'oneOf',
  'not',
  'if',
  'then',
  'else',
  'dependentSchemas',
]

/** Unsupported JSON Schema constructs remain editable without pretending to validate them. */
export function parameterProperties(schema?: Schema): Record<string, Schema> | null {
  if (!schema || complexKeys.some(key => key in schema) || !record(schema.properties)) return null
  if (schema.type !== undefined && schema.type !== 'object') return null
  const properties: Record<string, Schema> = {}
  for (const [key, value] of Object.entries(schema.properties)) {
    if (!record(value)) return null
    properties[key] = value
  }
  return properties
}

function fieldType(schema: Schema): string {
  if (complexKeys.some(key => key in schema)) return 'json'
  return typeof schema.type === 'string' ? schema.type : 'json'
}

function expression(value: unknown): boolean {
  return typeof value === 'string' && /^\s*\{\{[\s\S]+\}\}\s*$/.test(value)
}

function fieldError(value: unknown, schema: Schema): string | null {
  if (expression(value)) return null
  const type = fieldType(schema)
  if (type === 'json') return null
  if (value === null) return '此参数类型不接受 null；原值已保留，可在 JSON 中编辑'
  if (type === 'array' && !Array.isArray(value)) return '请输入 JSON 数组'
  if (type === 'object' && !record(value)) return '请输入 JSON 对象'
  if (type === 'string' && typeof value !== 'string') return '请输入文本'
  if (
    (type === 'number' || type === 'integer') &&
    (typeof value !== 'number' || !Number.isFinite(value))
  )
    return '请输入有效数值，或切换为变量/表达式'
  if (type === 'integer' && !Number.isSafeInteger(value)) return '请输入有效整数'
  if (type === 'boolean' && typeof value !== 'boolean') return '请选择是或否，或使用变量/表达式'
  if (typeof value === 'number') {
    if (typeof schema.minimum === 'number' && value < schema.minimum)
      return `不能小于 ${schema.minimum}`
    if (typeof schema.maximum === 'number' && value > schema.maximum)
      return `不能大于 ${schema.maximum}`
    if (typeof schema.exclusiveMinimum === 'number' && value <= schema.exclusiveMinimum)
      return `必须大于 ${schema.exclusiveMinimum}`
    if (typeof schema.exclusiveMaximum === 'number' && value >= schema.exclusiveMaximum)
      return `必须小于 ${schema.exclusiveMaximum}`
  }
  if (
    Array.isArray(schema.enum) &&
    !schema.enum.some(v => JSON.stringify(v) === JSON.stringify(value))
  )
    return '请选择允许的值；原值已保留'
  return null
}

export function validateToolParameters(value: Parameters, schema?: Schema): Record<string, string> {
  const errors: Record<string, string> = {}
  const properties = parameterProperties(schema)
  if (!properties) return errors
  const required = Array.isArray(schema?.required) ? schema.required : []
  for (const [key, property] of Object.entries(properties)) {
    if (!(key in value)) {
      if (required.includes(key)) errors[key] = '请填写必填参数'
      continue
    }
    const error = fieldError(value[key], property)
    if (error) errors[key] = error
  }
  return errors
}

/** Only real schema defaults are candidates; no placeholder zero/false/first enum choice. */
export function declaredParameterDefaults(schema?: Schema): Parameters {
  return Object.fromEntries(
    Object.entries(parameterProperties(schema) ?? {})
      .filter(([, s]) => hasOwn(s, 'default'))
      .map(([key, s]) => [key, s.default]),
  )
}

function display(value: unknown, type: string): string {
  if (value === undefined) return ''
  if (type === 'string' || expression(value))
    return typeof value === 'string' ? value : JSON.stringify(value)
  return JSON.stringify(value, null, 2)
}

function initialDraft(value: unknown, schema?: Schema): ToolParameterDraft {
  return {
    mode: parameterProperties(schema) && record(value ?? {}) ? 'form' : 'json',
    raw: JSON.stringify(value ?? {}, null, 2),
    fields: {},
    expressions: {},
    errors: {},
  }
}

/** In the canvas, text/validation belong to its draft coordinator, including hidden nodes. */
function ManagedToolParameterForm(props: ToolParameterFormProps) {
  const { t } = useLanguage()
  const baseline = JSON.stringify({
    parameters: props.value ?? {},
    draft: initialDraft(props.value, props.schema),
  })
  const buffer = useInspectorDraft('/do/with', baseline, {
    onCommit: text => props.onChange(JSON.parse(text).parameters),
    validate: text => {
      const payload = JSON.parse(text) as { parameters: Parameters; draft: ToolParameterDraft }
      return (
        Object.values(payload.draft.errors)[0] ??
        Object.values(validateToolParameters(payload.parameters, props.schema))[0] ??
        null
      )
    },
  })
  const previousReset = useRef(props.resetVersion)
  useEffect(() => {
    if (previousReset.current !== props.resetVersion) {
      previousReset.current = props.resetVersion
      buffer.reset(baseline)
    }
  }, [props.resetVersion, baseline, buffer])
  const payload = JSON.parse(buffer.text) as { parameters: Parameters; draft: ToolParameterDraft }
  const latest = useRef(payload)
  latest.current = payload
  return (
    <div
      data-draft-field="/do/with"
      onBlur={() => {
        void buffer.commit()
      }}
    >
      <ParameterEditor
        {...props}
        value={payload.parameters}
        draft={payload.draft}
        onDraftChange={next => {
          latest.current = { ...latest.current, draft: next }
          buffer.setText(JSON.stringify(latest.current))
          props.onDraftChange?.(next)
        }}
        onChange={next => {
          latest.current = { ...latest.current, parameters: next }
          buffer.setText(JSON.stringify(latest.current))
        }}
      />
      {buffer.error && (
        <span className="wfc-field-error">{editorValidationText(buffer.error, t)}</span>
      )}
    </div>
  )
}

export function ToolParameterForm(props: ToolParameterFormProps) {
  const context = useContext(InspectorDraftContext)
  return context ? <ManagedToolParameterForm {...props} /> : <ParameterEditor {...props} />
}

function ParameterEditor({
  schema,
  value,
  onChange,
  readOnly,
  draft: storedDraft,
  onDraftChange,
  variables = [],
}: ToolParameterFormProps) {
  const { t } = useLanguage()
  const properties = parameterProperties(schema)
  const parameters = record(value) ? value : {}
  const [localDraft, setDraft] = useState<ToolParameterDraft>(
    () => storedDraft ?? initialDraft(value, schema),
  )
  const draft = storedDraft ?? localDraft
  const [expanded, setExpanded] = useState(false)
  const valueKey = JSON.stringify(value)
  const previousValue = useRef(valueKey)
  const emittedValue = useRef<string | undefined>(undefined)
  const id = useId()
  useEffect(() => {
    if (previousValue.current !== valueKey && emittedValue.current !== valueKey) {
      // Undo/external changes supersede local drafts; normal typing echoes must not erase them.
      setDraft(cur => ({
        ...cur,
        raw: JSON.stringify(value ?? {}, null, 2),
        fields: {},
        errors: {},
      }))
    }
    previousValue.current = valueKey
  }, [value, valueKey])
  const update = (next: ToolParameterDraft, nextValue?: Parameters) => {
    setDraft(next)
    onDraftChange?.(next)
    if (nextValue) {
      emittedValue.current = JSON.stringify(nextValue)
      onChange(nextValue)
    }
  }
  const errors = { ...validateToolParameters(parameters, schema), ...draft.errors }
  const setField = (key: string, text: string, property: Schema, isExpression: boolean) => {
    const next = { ...draft, fields: { ...draft.fields, [key]: text }, errors: { ...draft.errors } }
    let parsed: unknown = text
    let error: string | null = null
    if (isExpression) {
      if (!expression(text)) error = editorText('请填写完整表达式，例如 {{count}}', t)
    } else if (fieldType(property) !== 'string') {
      try {
        parsed = JSON.parse(text)
      } catch {
        error = editorText('请输入有效 JSON 值；空白不等于 0 或未设置', t)
      }
    }
    if (!error) error = fieldError(parsed, property)
    if (error) {
      next.errors[key] = error
      update(next)
    } else {
      delete next.errors[key]
      const nextValue = { ...parameters, [key]: parsed }
      next.raw = JSON.stringify(nextValue, null, 2)
      update(next, nextValue)
    }
  }
  const unset = (key: string) => {
    const nextValue = { ...parameters }
    delete nextValue[key]
    const next = {
      ...draft,
      fields: { ...draft.fields },
      errors: { ...draft.errors },
      raw: JSON.stringify(nextValue, null, 2),
    }
    delete next.fields[key]
    delete next.errors[key]
    update(next, nextValue)
  }
  const jsonMode = draft.mode === 'json' || !properties
  const unknownKeys = properties ? Object.keys(parameters).filter(key => !(key in properties)) : []
  return (
    <div className="wfc-parameter-form">
      <div className="wfc-parameter-toolbar">
        <span className="wfc-field-label">{editorText('动作参数', t)}</span>
        {properties && (
          <button
            type="button"
            disabled={
              readOnly ||
              (jsonMode && !!draft.errors.$json) ||
              (!jsonMode && Object.keys(draft.errors).length > 0)
            }
            onClick={() =>
              update({
                ...draft,
                mode: jsonMode ? 'form' : 'json',
                raw: jsonMode ? draft.raw : JSON.stringify(parameters, null, 2),
              })
            }
          >
            {jsonMode ? editorText('表单编辑', t) : editorText('JSON 编辑', t)}
          </button>
        )}
      </div>
      {jsonMode ? (
        <div className="wfc-field" data-field-path="/do/with">
          <span className="wfc-field-label">
            {editorText('参数 JSON', t)}
            <button type="button" onClick={() => setExpanded(v => !v)}>
              {expanded ? editorText('收起编辑器', t) : editorText('展开编辑器', t)}
            </button>
          </span>
          <textarea
            className="wfc-input wfc-input--mono"
            aria-label={editorText('参数 JSON', t)}
            aria-invalid={!!draft.errors.$json}
            value={draft.raw}
            readOnly={readOnly}
            rows={expanded ? 22 : 8}
            onChange={e => {
              const next = { ...draft, raw: e.target.value, fields: {}, errors: {} }
              try {
                const parsed: unknown = JSON.parse(next.raw)
                if (!record(parsed)) throw new Error(editorText('参数必须是 JSON 对象', t))
                update(next, parsed)
              } catch (error) {
                update({
                  ...next,
                  errors: {
                    $json: error instanceof Error ? error.message : editorText('JSON 无效', t),
                  },
                })
              }
            }}
          />
          {draft.errors.$json && (
            <span className="wfc-field-error">{editorValidationText(draft.errors.$json, t)}</span>
          )}
          {!properties && (
            <span className="wfc-parameter-hint">
              {editorText('此工具未提供可生成表单的参数结构，使用 JSON 编辑。', t)}
            </span>
          )}
          {variables.length > 0 && (
            <span className="wfc-parameter-hint">
              {editorText('变量表达式需放在 JSON 字符串中，例如：', t)}
              {JSON.stringify(variables[0].expression ?? `{{${variables[0].name}}}`)}
            </span>
          )}
        </div>
      ) : (
        <>
          {Object.entries(properties).map(([key, property]) => {
            const type = fieldType(property)
            const present = hasOwn(parameters, key)
            const required = Array.isArray(schema?.required) && schema.required.includes(key)
            const allowsExpressionMode = type !== 'string' || Array.isArray(property.enum)
            const isExpression =
              allowsExpressionMode && (draft.expressions[key] ?? expression(parameters[key]))
            const text = draft.fields[key] ?? display(parameters[key], type)
            const label = typeof property.title === 'string' ? property.title : key
            const options =
              Array.isArray(property.enum) && !isExpression
                ? property.enum
                : type === 'boolean' && !isExpression
                  ? [true, false]
                  : null
            const inputId = `${id}-${key}`
            const error = errors[key]
            return (
              <div
                className="wfc-field"
                key={key}
                data-field-path={`/do/with/${key.replace(/~/g, '~0').replace(/\//g, '~1')}`}
              >
                <div className="wfc-parameter-label">
                  <label htmlFor={inputId} className="wfc-field-label">
                    {label}
                    {required && <em className="wfc-required-mark">{editorText('（必填）', t)}</em>}
                    {label !== key && <small> {key}</small>}
                  </label>
                  <button
                    type="button"
                    disabled={readOnly || (!present && !(key in draft.fields))}
                    onClick={() => unset(key)}
                  >
                    {editorText('设为未设置', t)}
                  </button>
                </div>
                {typeof property.description === 'string' && (
                  <span className="wfc-parameter-hint">{property.description}</span>
                )}
                {options ? (
                  <select
                    id={inputId}
                    aria-label={label}
                    className="wfc-input"
                    disabled={readOnly}
                    aria-invalid={!!error}
                    value={present ? JSON.stringify(parameters[key]) : ''}
                    onChange={e =>
                      e.target.value === ''
                        ? unset(key)
                        : setField(key, e.target.value, { ...property, type: 'json' }, false)
                    }
                  >
                    <option value="">{editorText('未设置', t)}</option>
                    {present &&
                      !options.some(v => JSON.stringify(v) === JSON.stringify(parameters[key])) && (
                        <option value={JSON.stringify(parameters[key])}>
                          {editorText('原值：', t)}
                          {JSON.stringify(parameters[key])}
                        </option>
                      )}
                    {options.map(v => (
                      <option key={JSON.stringify(v)} value={JSON.stringify(v)}>
                        {typeof v === 'boolean'
                          ? v
                            ? editorText('是', t)
                            : editorText('否', t)
                          : String(v)}
                      </option>
                    ))}
                  </select>
                ) : (type === 'array' || type === 'object' || type === 'json') && !isExpression ? (
                  <textarea
                    id={inputId}
                    aria-label={label}
                    className="wfc-input wfc-input--mono"
                    value={text}
                    readOnly={readOnly}
                    rows={4}
                    placeholder={editorText('未设置', t)}
                    aria-invalid={!!error}
                    onChange={e => setField(key, e.target.value, property, false)}
                  />
                ) : (
                  <input
                    id={inputId}
                    aria-label={label}
                    className="wfc-input"
                    type="text"
                    value={text}
                    readOnly={readOnly}
                    placeholder={isExpression ? '{{variable}}' : editorText('未设置', t)}
                    aria-invalid={!!error}
                    onChange={e => setField(key, e.target.value, property, isExpression)}
                  />
                )}
                {error && <span className="wfc-field-error">{editorValidationText(error, t)}</span>}
                <div className="wfc-parameter-tools">
                  {allowsExpressionMode && (
                    <button
                      type="button"
                      disabled={readOnly}
                      aria-pressed={isExpression}
                      onClick={() =>
                        update({
                          ...draft,
                          expressions: { ...draft.expressions, [key]: !isExpression },
                        })
                      }
                    >
                      {isExpression
                        ? editorText('使用固定值', t)
                        : editorText('使用变量/表达式', t)}
                    </button>
                  )}
                  {hasOwn(property, 'default') && (
                    <button
                      type="button"
                      disabled={readOnly}
                      onClick={() =>
                        setField(key, display(property.default, type), property, false)
                      }
                    >
                      {editorText('使用默认值：', t)}
                      {JSON.stringify(property.default)}
                    </button>
                  )}
                  {variables.length > 0 && (
                    <select
                      aria-label={t('workflowCanvas.variable.insert', label)}
                      className="wfc-input"
                      disabled={readOnly}
                      value=""
                      onChange={e => {
                        if (!e.target.value) return
                        const next = {
                          ...draft,
                          expressions: { ...draft.expressions, [key]: true },
                          fields: { ...draft.fields, [key]: e.target.value },
                          errors: { ...draft.errors },
                        }
                        delete next.errors[key]
                        const nextValue = { ...parameters, [key]: e.target.value }
                        update({ ...next, raw: JSON.stringify(nextValue, null, 2) }, nextValue)
                      }}
                    >
                      <option value="">{t('workflowCanvas.variable.insertPlaceholder')}</option>
                      {variables.map(v => (
                        <option key={v.name} value={v.expression ?? `{{${v.name}}}`}>
                          {v.label ?? v.name}
                        </option>
                      ))}
                    </select>
                  )}
                </div>
              </div>
            )
          })}
          {unknownKeys.length > 0 && (
            <p className="wfc-parameter-hint">
              {editorText('保留未识别参数：', t)}
              {unknownKeys.join(', ')}
              {editorText('。可在 JSON 编辑中修改。', t)}
            </p>
          )}
          {Object.keys(properties).length === 0 && (
            <p className="wfc-parameter-hint">
              {editorText('此工具未声明参数，可在 JSON 编辑中添加。', t)}
            </p>
          )}
        </>
      )}
    </div>
  )
}
