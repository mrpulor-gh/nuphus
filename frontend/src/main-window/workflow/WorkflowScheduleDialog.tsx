import { useEffect, useMemo, useState } from 'react'
import type {
  ScheduleConfig,
  WorkflowInputKind,
  WorkflowInputSpec,
  WorkflowItem,
} from '../../core/types'
import { Button } from '../../ui/Button'
import { IconClock3, IconTrash2 } from '../../ui/Icons'
import { CompactModal } from '../layout/CompactModal'
import {
  wfScheduleGet,
  wfSchedulePreview,
  wfScheduleRemove,
  wfScheduleSet,
  type WfScheduleDetails,
} from '../lib/api'
import './workflow-schedule.css'

export type ScheduleMode =
  'minutes' | 'hourly' | 'daily' | 'weekdays' | 'weekly' | 'monthly' | 'custom'

export interface SchedulePattern {
  mode: ScheduleMode
  interval: number
  hour: number
  minute: number
  weekday: number
  monthDay: number
  custom: string
}

const DEFAULT_PATTERN: SchedulePattern = {
  mode: 'daily',
  interval: 5,
  hour: 9,
  minute: 0,
  weekday: 1,
  monthDay: 1,
  custom: '0 9 * * *',
}

const TIMEZONES = ['Asia/Shanghai', 'UTC', 'Asia/Tokyo', 'Europe/London', 'America/New_York']
const EMPTY_INPUTS: WorkflowInputSpec[] = []

function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'Asia/Shanghai'
  } catch {
    return 'Asia/Shanghai'
  }
}

export function cronFromPattern(pattern: SchedulePattern): string {
  const minute = Math.max(0, Math.min(59, Math.trunc(pattern.minute)))
  const hour = Math.max(0, Math.min(23, Math.trunc(pattern.hour)))
  switch (pattern.mode) {
    case 'minutes':
      return `*/${Math.max(1, Math.min(60, Math.trunc(pattern.interval)))} * * * *`
    case 'hourly':
      return `${minute} * * * *`
    case 'daily':
      return `${minute} ${hour} * * *`
    case 'weekdays':
      return `${minute} ${hour} * * 1-5`
    case 'weekly':
      return `${minute} ${hour} * * ${Math.max(0, Math.min(6, Math.trunc(pattern.weekday)))}`
    case 'monthly':
      return `${minute} ${hour} ${Math.max(1, Math.min(28, Math.trunc(pattern.monthDay)))} * *`
    case 'custom':
      return pattern.custom.trim()
  }
}

export function patternFromCron(cron: string): SchedulePattern {
  let match = cron.match(/^\*\/(5|10|15|30) \* \* \* \*$/)
  if (match)
    return { ...DEFAULT_PATTERN, mode: 'minutes', interval: Number(match[1]), custom: cron }
  match = cron.match(/^(\d{1,2}) \* \* \* \*$/)
  if (match) return { ...DEFAULT_PATTERN, mode: 'hourly', minute: Number(match[1]), custom: cron }
  match = cron.match(/^(\d{1,2}) (\d{1,2}) \* \* (\*|1-5|[0-6])$/)
  if (match) {
    const mode = match[3] === '*' ? 'daily' : match[3] === '1-5' ? 'weekdays' : 'weekly'
    return {
      ...DEFAULT_PATTERN,
      mode,
      minute: Number(match[1]),
      hour: Number(match[2]),
      weekday: mode === 'weekly' ? Number(match[3]) : 1,
      custom: cron,
    }
  }
  match = cron.match(/^(\d{1,2}) (\d{1,2}) (\d{1,2}) \* \*$/)
  if (match) {
    return {
      ...DEFAULT_PATTERN,
      mode: 'monthly',
      minute: Number(match[1]),
      hour: Number(match[2]),
      monthDay: Number(match[3]),
      custom: cron,
    }
  }
  return { ...DEFAULT_PATTERN, mode: 'custom', custom: cron }
}

function patternFromConfig(config: ScheduleConfig): SchedulePattern {
  if (config.interval_minutes !== undefined) {
    return { ...DEFAULT_PATTERN, mode: 'minutes', interval: config.interval_minutes }
  }
  return patternFromCron(config.cron)
}

function inputKind(spec: WorkflowInputSpec): WorkflowInputKind {
  return spec.type ?? 'string'
}

function editValue(value: unknown, kind: WorkflowInputKind): string | boolean {
  if (kind === 'boolean') return value === true
  if (kind === 'json') return JSON.stringify(value, null, 2)
  return value === undefined ? '' : String(value)
}

export function resolveScheduleInputs(
  specs: WorkflowInputSpec[],
  fixed: Record<string, boolean>,
  values: Record<string, string | boolean>,
  preserved: Set<string>,
): {
  inputs: Record<string, unknown>
  preserveSensitive: string[]
  errors: Record<string, string>
} {
  const inputs: Record<string, unknown> = {}
  const preserveSensitive: string[] = []
  const errors: Record<string, string> = {}
  for (const spec of specs) {
    if (!fixed[spec.name]) {
      if (spec.required && spec.default === undefined) errors[spec.name] = '必填输入必须固定一个值'
      continue
    }
    if (spec.sensitive && preserved.has(spec.name) && values[spec.name] === '') {
      preserveSensitive.push(spec.name)
      continue
    }
    const kind = inputKind(spec)
    const raw = values[spec.name]
    if (kind === 'boolean') {
      inputs[spec.name] = raw === true
    } else if (kind === 'number') {
      const number = Number(raw)
      if (String(raw).trim() === '' || !Number.isFinite(number))
        errors[spec.name] = '请输入有效数字'
      else inputs[spec.name] = number
    } else if (kind === 'json') {
      try {
        inputs[spec.name] = JSON.parse(String(raw))
      } catch {
        errors[spec.name] = 'JSON 格式无效'
      }
    } else {
      inputs[spec.name] = String(raw ?? '')
    }
  }
  return { inputs, preserveSensitive, errors }
}

interface WorkflowScheduleDialogProps {
  open: boolean
  workflow: Pick<WorkflowItem, 'id' | 'title' | 'inputs' | 'schedule'>
  readOnly?: boolean
  layer?: 'default' | 'settings'
  onClose: () => void
  onChanged: (config: ScheduleConfig | null) => void
}

export function WorkflowScheduleDialog({
  open,
  workflow,
  readOnly = false,
  layer = 'default',
  onClose,
  onChanged,
}: WorkflowScheduleDialogProps) {
  const specs = workflow.inputs ?? EMPTY_INPUTS
  const [details, setDetails] = useState<WfScheduleDetails | null>(null)
  const [pattern, setPattern] = useState(DEFAULT_PATTERN)
  const [timezone, setTimezone] = useState(localTimezone())
  const [label, setLabel] = useState('')
  const [enabled, setEnabled] = useState(true)
  const [fixed, setFixed] = useState<Record<string, boolean>>({})
  const [values, setValues] = useState<Record<string, string | boolean>>({})
  const [preserved, setPreserved] = useState<Set<string>>(new Set())
  const [nextRuns, setNextRuns] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    if (!open) return
    let alive = true
    setLoading(true)
    setError(null)
    void wfScheduleGet(workflow.id)
      .then(result => {
        if (!alive) return
        if (!result) throw new Error('读取定时配置失败：后端无响应')
        const config = result.config ?? {
          cron: '0 9 * * *',
          timezone: localTimezone(),
          enabled: true,
          label: '',
        }
        setDetails(result)
        setPattern(patternFromConfig(config))
        setTimezone(config.timezone)
        setLabel(config.label ?? '')
        setEnabled(config.enabled)
        const nextFixed: Record<string, boolean> = {}
        const nextValues: Record<string, string | boolean> = {}
        const savedSensitive = new Set(result.sensitive_inputs)
        for (const spec of specs) {
          const hasVisible = Object.prototype.hasOwnProperty.call(result.inputs, spec.name)
          const hasSensitive = savedSensitive.has(spec.name)
          nextFixed[spec.name] =
            hasVisible || hasSensitive || (!!spec.required && spec.default === undefined)
          nextValues[spec.name] = hasVisible
            ? editValue(result.inputs[spec.name], inputKind(spec))
            : inputKind(spec) === 'boolean'
              ? false
              : ''
        }
        setFixed(nextFixed)
        setValues(nextValues)
        setPreserved(savedSensitive)
      })
      .catch(reason => alive && setError(String(reason)))
      .finally(() => alive && setLoading(false))
    return () => {
      alive = false
    }
  }, [open, workflow.id, specs])

  const cron = cronFromPattern(pattern)
  const intervalError =
    pattern.mode === 'minutes' &&
    (!Number.isInteger(pattern.interval) || pattern.interval < 1 || pattern.interval > 1440)
  const config = useMemo<ScheduleConfig>(
    () => ({
      cron,
      timezone,
      enabled,
      ...(label.trim() ? { label: label.trim() } : {}),
      ...(pattern.mode === 'minutes'
        ? { interval_minutes: Math.max(1, Math.min(1440, Math.trunc(pattern.interval))) }
        : {}),
    }),
    [cron, timezone, enabled, label, pattern.mode, pattern.interval],
  )
  const resolvedInputs = useMemo(
    () => resolveScheduleInputs(specs, fixed, values, preserved),
    [specs, fixed, values, preserved],
  )

  useEffect(() => {
    if (!open || !cron || !timezone || intervalError) return
    const timer = setTimeout(() => {
      void wfSchedulePreview(config)
        .then(runs => {
          if (!runs) throw new Error('预览失败：后端无响应')
          setNextRuns(runs)
          setError(null)
        })
        .catch(reason => {
          setNextRuns([])
          setError(String(reason))
        })
    }, 250)
    return () => clearTimeout(timer)
  }, [open, config, cron, timezone, intervalError])

  const save = async () => {
    if (!details || Object.keys(resolvedInputs.errors).length > 0) return
    setSaving(true)
    setError(null)
    try {
      await wfScheduleSet(
        workflow.id,
        config,
        resolvedInputs.inputs,
        resolvedInputs.preserveSensitive,
      )
      onChanged(config)
      onClose()
    } catch (reason) {
      setError(String(reason))
    } finally {
      setSaving(false)
    }
  }

  const remove = async () => {
    setSaving(true)
    setError(null)
    try {
      await wfScheduleRemove(workflow.id)
      onChanged(null)
      onClose()
    } catch (reason) {
      setError(String(reason))
    } finally {
      setSaving(false)
    }
  }

  return (
    <CompactModal
      open={open}
      onClose={onClose}
      title={`定时运行 · ${workflow.title}`}
      icon={<IconClock3 size={14} />}
      size="xl"
      layer={layer}
      className="wfs-modal"
      footer={
        <>
          <div className="wcf-footer-left">
            {details?.config && (
              <Button
                variant="danger"
                size="sm"
                disabled={readOnly || saving}
                onClick={() => void remove()}
              >
                <IconTrash2 size={12} /> 删除定时
              </Button>
            )}
            <Button variant="ghost" size="sm" onClick={onClose}>
              取消
            </Button>
          </div>
          <div className="wcf-footer-right">
            <Button
              variant="primary"
              size="sm"
              loading={saving}
              disabled={
                readOnly ||
                loading ||
                !details?.eligible ||
                !!error ||
                intervalError ||
                Object.keys(resolvedInputs.errors).length > 0
              }
              onClick={() => void save()}
            >
              保存并应用
            </Button>
          </div>
        </>
      }
    >
      {loading ? (
        <div className="wfs-empty">加载定时配置...</div>
      ) : (
        <div className="wfs-body">
          {readOnly && <div className="wfs-banner">工作流正在运行，当前只能查看定时设置。</div>}
          {details && !details.eligible && (
            <div className="wfs-banner wfs-banner--warning">{details.ineligible_reason}</div>
          )}
          {error && <div className="wfs-banner wfs-banner--error">{error}</div>}

          <section className="wfs-section">
            <div className="wfs-section-title">触发规则</div>
            <div className="wfs-grid">
              <label>
                频率
                <select
                  value={pattern.mode}
                  disabled={readOnly}
                  onChange={event =>
                    setPattern(current => ({
                      ...current,
                      mode: event.target.value as ScheduleMode,
                    }))
                  }
                >
                  <option value="minutes">每隔几分钟</option>
                  <option value="hourly">每小时</option>
                  <option value="daily">每天</option>
                  <option value="weekdays">工作日</option>
                  <option value="weekly">每周</option>
                  <option value="monthly">每月</option>
                  <option value="custom">高级 Cron</option>
                </select>
              </label>
              {pattern.mode === 'minutes' && (
                <label>
                  间隔（分钟）
                  <input
                    type="number"
                    min={1}
                    max={1440}
                    step={1}
                    value={Number.isFinite(pattern.interval) ? pattern.interval : ''}
                    disabled={readOnly}
                    onChange={event =>
                      setPattern(current => ({
                        ...current,
                        interval:
                          event.target.value === '' ? Number.NaN : Number(event.target.value),
                      }))
                    }
                  />
                  {intervalError && (
                    <span className="wfs-field-error">请输入 1–1440 的整数分钟数</span>
                  )}
                </label>
              )}
              {pattern.mode === 'hourly' && (
                <label>
                  分钟
                  <input
                    type="number"
                    min={0}
                    max={59}
                    value={pattern.minute}
                    disabled={readOnly}
                    onChange={event =>
                      setPattern(current => ({ ...current, minute: Number(event.target.value) }))
                    }
                  />
                </label>
              )}
              {['daily', 'weekdays', 'weekly', 'monthly'].includes(pattern.mode) && (
                <label>
                  时间
                  <input
                    type="time"
                    value={`${String(pattern.hour).padStart(2, '0')}:${String(pattern.minute).padStart(2, '0')}`}
                    disabled={readOnly}
                    onChange={event => {
                      const [hour, minute] = event.target.value.split(':').map(Number)
                      setPattern(current => ({ ...current, hour, minute }))
                    }}
                  />
                </label>
              )}
              {pattern.mode === 'weekly' && (
                <label>
                  星期
                  <select
                    value={pattern.weekday}
                    disabled={readOnly}
                    onChange={event =>
                      setPattern(current => ({ ...current, weekday: Number(event.target.value) }))
                    }
                  >
                    <option value={1}>星期一</option>
                    <option value={2}>星期二</option>
                    <option value={3}>星期三</option>
                    <option value={4}>星期四</option>
                    <option value={5}>星期五</option>
                    <option value={6}>星期六</option>
                    <option value={0}>星期日</option>
                  </select>
                </label>
              )}
              {pattern.mode === 'monthly' && (
                <label>
                  日期
                  <input
                    type="number"
                    min={1}
                    max={28}
                    value={pattern.monthDay}
                    disabled={readOnly}
                    onChange={event =>
                      setPattern(current => ({ ...current, monthDay: Number(event.target.value) }))
                    }
                  />
                </label>
              )}
              {pattern.mode === 'custom' && (
                <label className="wfs-wide">
                  Cron（分 时 日 月 周）
                  <input
                    className="wfs-mono"
                    value={pattern.custom}
                    disabled={readOnly}
                    onChange={event =>
                      setPattern(current => ({ ...current, custom: event.target.value }))
                    }
                  />
                </label>
              )}
              <label>
                时区
                <input
                  list="wfs-timezones"
                  value={timezone}
                  disabled={readOnly}
                  onChange={event => setTimezone(event.target.value)}
                />
                <datalist id="wfs-timezones">
                  {Array.from(new Set([localTimezone(), ...TIMEZONES])).map(value => (
                    <option key={value} value={value} />
                  ))}
                </datalist>
              </label>
              <label>
                标签
                <input
                  value={label}
                  disabled={readOnly}
                  placeholder="可选"
                  onChange={event => setLabel(event.target.value)}
                />
              </label>
              <label className="wfs-toggle">
                <input
                  type="checkbox"
                  checked={enabled}
                  disabled={readOnly}
                  onChange={event => setEnabled(event.target.checked)}
                />
                启用定时任务
              </label>
            </div>
            {nextRuns.length > 0 && (
              <div className="wfs-next-runs">
                <span>接下来</span>
                {nextRuns.map(value => (
                  <time key={value}>{new Date(value).toLocaleString()}</time>
                ))}
              </div>
            )}
          </section>

          {specs.length > 0 && (
            <section className="wfs-section">
              <div className="wfs-section-title">运行输入</div>
              {specs.map(spec => {
                const kind = inputKind(spec)
                const isFixed = !!fixed[spec.name]
                const isPreserved = spec.sensitive && preserved.has(spec.name)
                return (
                  <div className="wfs-input-row" key={spec.name}>
                    <label className="wfs-input-fixed">
                      <input
                        type="checkbox"
                        checked={isFixed}
                        disabled={readOnly || (!!spec.required && spec.default === undefined)}
                        onChange={event => {
                          const checked = event.target.checked
                          setFixed(current => ({ ...current, [spec.name]: checked }))
                          if (!checked)
                            setPreserved(current => {
                              const next = new Set(current)
                              next.delete(spec.name)
                              return next
                            })
                        }}
                      />
                      固定此值
                    </label>
                    <div className="wfs-input-main">
                      <div className="wfs-input-label">
                        <span>{spec.name}</span>
                        <span>{kind}</span>
                        {spec.required && <span>必填</span>}
                        {spec.sensitive && <span>敏感</span>}
                      </div>
                      {spec.description && <div className="wfs-input-hint">{spec.description}</div>}
                      {!isFixed ? (
                        <div className="wfs-input-follow">触发时使用工作流当前默认值</div>
                      ) : kind === 'boolean' ? (
                        <label className="wfs-toggle">
                          <input
                            type="checkbox"
                            checked={values[spec.name] === true}
                            disabled={readOnly}
                            onChange={event =>
                              setValues(current => ({
                                ...current,
                                [spec.name]: event.target.checked,
                              }))
                            }
                          />
                          {values[spec.name] === true ? 'true' : 'false'}
                        </label>
                      ) : kind === 'json' ? (
                        <textarea
                          rows={3}
                          value={String(values[spec.name] ?? '')}
                          disabled={readOnly}
                          placeholder={isPreserved ? '已保存敏感值，留空保持' : 'JSON'}
                          onChange={event => {
                            setValues(current => ({ ...current, [spec.name]: event.target.value }))
                            setPreserved(current => {
                              const next = new Set(current)
                              next.delete(spec.name)
                              return next
                            })
                          }}
                        />
                      ) : (
                        <input
                          type={spec.sensitive ? 'password' : kind === 'number' ? 'number' : 'text'}
                          value={String(values[spec.name] ?? '')}
                          disabled={readOnly}
                          placeholder={isPreserved ? '已保存敏感值，留空保持' : ''}
                          autoComplete="off"
                          onChange={event => {
                            setValues(current => ({ ...current, [spec.name]: event.target.value }))
                            setPreserved(current => {
                              const next = new Set(current)
                              next.delete(spec.name)
                              return next
                            })
                          }}
                        />
                      )}
                      {isPreserved && values[spec.name] === '' && (
                        <div className="wfs-input-saved">已保存敏感值，保存时保持不变</div>
                      )}
                      {resolvedInputs.errors[spec.name] && (
                        <div className="wfs-field-error">{resolvedInputs.errors[spec.name]}</div>
                      )}
                    </div>
                  </div>
                )
              })}
            </section>
          )}
        </div>
      )}
    </CompactModal>
  )
}
