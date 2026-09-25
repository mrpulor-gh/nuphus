import { useContext, useEffect, useId, useMemo, useRef, useState } from 'react'
import {
  wfTraceList,
  wfTraceRead,
  type WorkflowInvocationTrace,
  type WorkflowRunTrace,
} from '../lib/api'
import { WorkflowTraceContext } from './WorkflowTraceContext'
import {
  runtimeChildren,
  runtimeChildCount,
  runtimePreview,
  runtimeRoots,
  type RuntimeField,
} from './runtimeFields'
import type { VariableCatalog } from './variableCatalog'
import { useTraceText, useTraceTime } from './traceMessages'
import './execution-trace.css'

export interface ExecutionTraceViewerProps {
  workflowId?: string
  stepId?: string
  /** Initial filter; omitted shows both normal and debug history. */
  debug?: boolean
  /** Keep this viewer on exactly this run, including while its trace is being created. */
  pinnedRunId?: string
  refreshKey?: string | number
  catalog?: VariableCatalog
  onSelectReference?: (expression: string) => void
  onInvocationSelected?: (run: WorkflowRunTrace, invocation: WorkflowInvocationTrace) => void
  /** A changed/missing/loading selection must not keep previously adoptable test data. */
  onSelectionUnavailable?: () => void
}

function FieldTree({
  field,
  onSelect,
}: {
  field: RuntimeField
  onSelect: (expression: string) => void
}) {
  const text = useTraceText()
  const [expanded, setExpanded] = useState(false)
  const [count, setCount] = useState(50)
  const children = useMemo(
    () => (expanded ? runtimeChildren(field, count) : []),
    [expanded, field, count],
  )
  const childCount = runtimeChildCount(field.value)
  const expandable = childCount > 0
  return (
    <li className="wfc-trace-field">
      <div className="wfc-trace-field-row">
        <button
          type="button"
          className="wfc-trace-expand"
          aria-label={field.expression}
          aria-expanded={expandable ? expanded : undefined}
          disabled={!expandable}
          onClick={() => setExpanded(value => !value)}
        >
          {expandable ? (expanded ? '▾' : '▸') : '·'}
        </button>
        <button
          type="button"
          className="wfc-trace-reference"
          aria-label={`${text('choose')}: ${field.expression}`}
          title={`${text('choose')}: ${field.expression}`}
          onClick={() => onSelect(field.expression)}
        >
          <code>{field.expression}</code>
          <span>{runtimePreview(field.value)}</span>
        </button>
      </div>
      {expanded && (
        <ul>
          {children.map(child => (
            <FieldTree key={child.expression} field={child} onSelect={onSelect} />
          ))}
          {childCount > count && (
            <li>
              <button type="button" onClick={() => setCount(value => value + 50)}>
                {text('more')} ({count}/{childCount})
              </button>
            </li>
          )}
        </ul>
      )}
    </li>
  )
}

/** All payload text is available without truncation; trees are lazy only for navigation. */
function FullValue({ value }: { value: unknown }) {
  const text = useTraceText()
  const [copied, setCopied] = useState<'copied' | 'copyFailed' | null>(null)
  const content = typeof value === 'string' ? value : (JSON.stringify(value, null, 2) ?? '')
  useEffect(() => setCopied(null), [content])
  return (
    <div className="wfc-trace-value">
      <button
        type="button"
        onClick={() => {
          if (!navigator.clipboard) {
            setCopied('copyFailed')
            return
          }
          void navigator.clipboard.writeText(content).then(
            () => setCopied('copied'),
            () => setCopied('copyFailed'),
          )
        }}
      >
        {text('copy')}
      </button>
      {copied && <small role="status">{text(copied)}</small>}
      <pre tabIndex={0}>{content}</pre>
    </div>
  )
}

const tabs = [
  'output',
  'inputs',
  'before',
  'after',
  'attempts',
  'verification',
  'definition',
] as const
type Tab = (typeof tabs)[number]

export function ExecutionTraceViewer(props: ExecutionTraceViewerProps) {
  const context = useContext(WorkflowTraceContext)
  const workflowId = props.workflowId ?? context?.workflowId
  const refreshKey = props.refreshKey ?? context?.refreshKey
  const text = useTraceText()
  const formatTime = useTraceTime()
  const id = useId()
  const onInvocationSelected = useRef(props.onInvocationSelected)
  onInvocationSelected.current = props.onInvocationSelected
  const onSelectionUnavailable = useRef(props.onSelectionUnavailable)
  onSelectionUnavailable.current = props.onSelectionUnavailable
  const [filter, setFilter] = useState<'all' | 'true' | 'false'>(
    props.debug === undefined ? 'all' : (String(props.debug) as 'true' | 'false'),
  )
  const [refresh, setRefresh] = useState(0)
  const [runId, setRunId] = useState('')
  const [invocationId, setInvocationId] = useState<number | null>(null)
  const [tab, setTab] = useState<Tab>(props.onSelectReference ? 'after' : 'output')
  const scopeKey = `${workflowId ?? ''}:${filter}`
  const [listing, setListing] = useState<{
    key: string
    runs: WorkflowRunTrace[]
    loading: boolean
    error?: string
  }>({ key: '', runs: [], loading: false })
  const runs = listing.key === scopeKey ? listing.runs : []
  const selectedRun =
    props.pinnedRunId !== undefined
      ? runs.find(run => run.run_id === props.pinnedRunId)
      : runId
        ? runs.find(run => run.run_id === runId)
        : runs[0]
  const invocations =
    selectedRun?.invocations.filter(
      invocation =>
        !props.stepId ||
        (invocation.step_id === props.stepId && invocation.workflow_id === workflowId),
    ) ?? []
  const selectedSummary =
    invocations.find(invocation => invocation.id === invocationId) ??
    invocations[invocations.length - 1]
  const detailKey =
    selectedRun && selectedSummary ? `${scopeKey}:${selectedRun.run_id}:${selectedSummary.id}` : ''
  const [detail, setDetail] = useState<{
    key: string
    trace?: WorkflowInvocationTrace
    error?: string
  }>({ key: '' })
  const trace = detailKey && detail.key === detailKey ? detail.trace : undefined

  useEffect(() => {
    if (!workflowId) return
    let active = true
    setListing(previous => ({
      key: scopeKey,
      runs: previous.key === scopeKey ? previous.runs : [],
      loading: true,
    }))
    const partitions = filter === 'all' ? [false, true] : [filter === 'true']
    void Promise.allSettled(partitions.map(debug => wfTraceList(workflowId, debug))).then(
      results => {
        if (!active) return
        const collected: WorkflowRunTrace[] = []
        const failures: string[] = []
        results.forEach((result, index) => {
          if (result.status === 'fulfilled') {
            // The requested partition is authoritative, including older trace records.
            collected.push(
              ...(result.value ?? []).map(run => ({ ...run, debug: partitions[index] })),
            )
          } else {
            failures.push(
              `${partitions[index] ? text('debug') : text('normal')}: ${String(result.reason)}`,
            )
          }
        })
        collected.sort((a, b) => Date.parse(b.started_at) - Date.parse(a.started_at))
        setListing({
          key: scopeKey,
          runs: collected,
          loading: false,
          error: failures.join('\n') || undefined,
        })
      },
    )
    return () => {
      active = false
    }
    // text only translates an error; language changes need not reload stored evidence.
  }, [workflowId, scopeKey, filter, refresh, refreshKey])

  const selectedRunId = selectedRun?.run_id
  const selectedDebug = selectedRun?.debug ?? false
  const selectedInvocationId = selectedSummary?.id
  const invocationStatus = selectedSummary?.status
  useEffect(() => {
    if (!workflowId || !selectedRunId || selectedInvocationId === undefined) return
    let active = true
    setDetail({ key: detailKey })
    void wfTraceRead(workflowId, selectedRunId, selectedDebug, selectedInvocationId).then(
      result => {
        if (active)
          setDetail(
            result ? { key: detailKey, trace: result } : { key: detailKey, error: 'unavailable' },
          )
      },
      error => {
        if (active) setDetail({ key: detailKey, error: String(error) })
      },
    )
    return () => {
      active = false
    }
  }, [
    workflowId,
    selectedRunId,
    selectedInvocationId,
    detailKey,
    invocationStatus,
    selectedDebug,
    refresh,
    refreshKey,
  ])

  useEffect(() => {
    if (selectedRun && trace) onInvocationSelected.current?.(selectedRun, trace)
    else onSelectionUnavailable.current?.()
  }, [selectedRun, trace])

  const variables = trace && (tab === 'before' ? trace.variables_before : trace.variables_after)
  const roots = useMemo(
    () =>
      variables && props.catalog && trace?.workflow_id === workflowId
        ? runtimeRoots(variables, props.catalog)
        : [],
    [variables, props.catalog, trace?.workflow_id, workflowId],
  )
  const payload =
    trace &&
    {
      output: trace.output,
      inputs: trace.inputs,
      before: trace.variables_before,
      after: trace.variables_after,
      attempts: trace.attempts,
      verification: trace.verification,
      definition: trace.definition,
    }[tab]
  const status = (value: string) =>
    value === 'error' || value === 'failed'
      ? text('failedStatus')
      : [
            'success',
            'running',
            'interrupted',
            'timeout',
            'skipped',
            'paused',
            'cancelled',
            'completed_with_skips',
          ].includes(value)
        ? text(value as 'success')
        : value

  return (
    <section className="wfc-trace" aria-label={text('title')}>
      <div className="wfc-trace-toolbar">
        {props.pinnedRunId === undefined && (
          <label>
            {text('mode')}
            <select
              value={filter}
              onChange={event => {
                setFilter(event.target.value as typeof filter)
                setRunId('')
                setInvocationId(null)
              }}
            >
              <option value="all">{text('all')}</option>
              <option value="false">{text('normal')}</option>
              <option value="true">{text('debug')}</option>
            </select>
          </label>
        )}
        <button
          type="button"
          disabled={listing.loading || !workflowId}
          onClick={() => setRefresh(value => value + 1)}
        >
          {text('refresh')}
        </button>
      </div>
      {listing.key === scopeKey && listing.error && (
        <p role="alert">
          {text('failed')}: {listing.error}
        </p>
      )}
      {listing.loading && <p role="status">{text('loading')}</p>}
      {!listing.loading && !listing.error && !runs.length && props.pinnedRunId === undefined && (
        <p>{text('empty')}</p>
      )}
      {!listing.loading && !selectedRun && props.pinnedRunId !== undefined && (
        <p role="status">{text('pendingRun')}</p>
      )}
      {!listing.loading &&
        !selectedRun &&
        props.pinnedRunId === undefined &&
        runId &&
        !!runs.length && (
          <p role="status">
            {text('missingSelection')}{' '}
            <button
              type="button"
              onClick={() => {
                setRunId('')
                setInvocationId(null)
              }}
            >
              {text('chooseLatest')}
            </button>
          </p>
        )}
      {selectedRun && (
        <>
          {props.pinnedRunId === undefined && (
            <label className="wfc-trace-selector">
              {text('run')}
              <select
                value={selectedRun.run_id}
                onChange={event => {
                  setRunId(event.target.value)
                  setInvocationId(null)
                }}
              >
                {runs.map(run => (
                  <option key={`${run.debug}:${run.run_id}`} value={run.run_id}>
                    {formatTime(run.started_at)} · {text(run.debug ? 'debug' : 'normal')} ·{' '}
                    {status(run.status)}
                  </option>
                ))}
              </select>
            </label>
          )}
          <p className="wfc-trace-provenance">
            {text(selectedRun.debug ? 'debug' : 'normal')} · {formatTime(selectedRun.started_at)} ·{' '}
            {status(selectedRun.status)}
          </p>
          <details className="wfc-trace-provenance">
            <summary>{text('technicalDetails')}</summary>
            {text('run')}: <code>{selectedRun.run_id}</code>
            <br />
            {text('revision')}: <code>{selectedRun.revision}</code>
            <br />
            {text('startedAt')}: <code>{selectedRun.started_at}</code>
          </details>
          {selectedRun.storage_error && (
            <p role="alert">
              {text('failed')}: {selectedRun.storage_error}
            </p>
          )}
          {selectedRun.error && (
            <p role="alert">
              {text('error')}: {selectedRun.error}
            </p>
          )}
          {!invocations.length && <p>{text('emptyInvocations')}</p>}
          {!!invocations.length && (
            <label className="wfc-trace-selector">
              {text('invocation')}
              <select
                value={selectedSummary?.id ?? ''}
                onChange={event => setInvocationId(Number(event.target.value))}
              >
                {invocations.map(invocation => (
                  <option key={invocation.id} value={invocation.id}>
                    #{invocation.id} · {invocation.step_name || invocation.step_id} ·{' '}
                    {status(invocation.status)}
                    {invocation.parent_id != null
                      ? ` · ${text('parent')} #${invocation.parent_id}`
                      : ''}
                  </option>
                ))}
              </select>
            </label>
          )}
        </>
      )}
      {detailKey &&
        !trace &&
        (detail.key === detailKey && detail.error ? (
          <p role="alert">
            {text('failed')}: {detail.error === 'unavailable' ? text('unavailable') : detail.error}
          </p>
        ) : (
          <p role="status">{text('loading')}</p>
        ))}
      {trace && (
        <>
          <p className="wfc-trace-provenance">
            {trace.step_name || trace.step_id} · {formatTime(trace.started_at)} ·{' '}
            {status(trace.status)}
            {' · '}
            {text('duration')}:{' '}
            {trace.finished_at
              ? `${Math.max(0, Date.parse(trace.finished_at) - Date.parse(trace.started_at))} ms`
              : text('pendingDuration')}
          </p>
          <details className="wfc-trace-provenance">
            <summary>{text('invocationDetails')}</summary>
            {text('invocation')}: #{trace.id} ·{' '}
            <code>
              {trace.workflow_id}/{trace.step_id}
            </code>
            <br />
            {text('startedAt')}: <code>{trace.started_at}</code>
          </details>
          {trace.error !== null && (
            <div role="alert">
              <strong>{text('error')}</strong>
              <FullValue value={trace.error} />
            </div>
          )}
          <div className="wfc-trace-tabs" role="tablist" aria-label={text('title')}>
            {tabs.map(name => (
              <button
                key={name}
                type="button"
                id={`${id}-${name}`}
                role="tab"
                aria-selected={tab === name}
                aria-controls={`${id}-panel`}
                onClick={() => setTab(name)}
              >
                {text(name)}
              </button>
            ))}
          </div>
          <div role="tabpanel" id={`${id}-panel`} aria-labelledby={`${id}-${tab}`}>
            {tab === 'verification' && trace.verification === null ? (
              <p>{text('unverified')}</p>
            ) : tab === 'output' && trace.output === null ? (
              <p>{text('noOutput')}</p>
            ) : (
              <FullValue value={payload} />
            )}
            {(tab === 'before' || tab === 'after') && props.onSelectReference && (
              <div className="wfc-trace-fields">
                <strong>{text('fields')}</strong>
                <p>{text('historical')}</p>
                {roots.length ? (
                  <ul>
                    {roots.map(field => (
                      <FieldTree
                        key={`${detailKey}:${tab}:${field.expression}`}
                        field={field}
                        onSelect={props.onSelectReference!}
                      />
                    ))}
                  </ul>
                ) : (
                  <p>{text('noFields')}</p>
                )}
              </div>
            )}
          </div>
          <button
            type="button"
            onClick={() => {
              const url = URL.createObjectURL(
                new Blob([JSON.stringify({ run: selectedRun, invocation: trace }, null, 2)], {
                  type: 'application/json',
                }),
              )
              const link = document.createElement('a')
              link.href = url
              link.download = `workflow-${selectedRun?.run_id}-invocation-${trace.id}.json`
              link.click()
              setTimeout(() => URL.revokeObjectURL(url), 1000)
            }}
          >
            {text('download')}
          </button>
        </>
      )}
    </section>
  )
}
