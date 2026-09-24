import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ChevronDown,
  ChevronUp,
  CircleAlert,
  TriangleAlert,
  Crosshair,
  ScrollText,
  ListChecks,
} from 'lucide-react'
import type { RunRecord, StepRunRecord } from '../../core/types'
import type { RunLogEntry } from './runStatus'
import type { Problem } from './validate'
import type { ValidationDiagnostic } from '../lib/api'
import { useLanguage } from '../../locales'
import { mergeEditorProblems, type EditorProblem, type ProblemCategory } from './editorProblems'

interface ProblemsPanelProps {
  problems: Problem[]
  backendReport: {
    errors: string[]
    warnings: string[]
    diagnostics?: ValidationDiagnostic[]
  } | null
  timeline: RunLogEntry[]
  running: boolean
  runHistory: RunRecord[]
  replay?: boolean
  onLocate: (stepId: string, fieldPath?: string) => void
  nameOf: (stepId: string) => string
}

function statusText(status: StepRunRecord['status'] | RunRecord['status']): string {
  return typeof status === 'string' ? status : `Error: ${status.Error}`
}

function formatClock(value: number | string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleTimeString()
}

function durationOf(started: string, finished?: string | null): string {
  if (!finished) return ''
  const duration = new Date(finished).getTime() - new Date(started).getTime()
  if (!Number.isFinite(duration) || duration < 0) return ''
  return duration < 1000 ? `${duration} ms` : `${(duration / 1000).toFixed(2)} s`
}

export function ProblemsPanel({
  problems,
  backendReport,
  timeline,
  running,
  runHistory,
  replay = false,
  onLocate,
  nameOf,
}: ProblemsPanelProps) {
  const { t } = useLanguage()
  const [collapsed, setCollapsed] = useState(false)
  const [mainTab, setMainTab] = useState<'problems' | 'logs'>('problems')
  const [problemTab, setProblemTab] = useState<ProblemCategory | 'all'>('all')
  const [historyIndex, setHistoryIndex] = useState(0)
  const wasRunning = useRef(false)

  useEffect(() => {
    if (!wasRunning.current && running) {
      setCollapsed(false)
      setMainTab('logs')
      setHistoryIndex(0)
    }
    wasRunning.current = running
  }, [running])

  useEffect(() => {
    if (!replay) return
    setCollapsed(false)
    setMainTab('logs')
    setHistoryIndex(0)
  }, [replay])

  const issues = useMemo(() => {
    const merged = mergeEditorProblems(problems, backendReport)
    const latest = runHistory[0]
    for (const step of latest?.steps ?? []) {
      if (typeof step.status !== 'object') continue
      merged.push({
        code: 'execution',
        category: 'runtime',
        level: 'error',
        stepId: step.step_id,
        details: [step.status.Error],
        sources: [latest.run_id],
      } satisfies EditorProblem)
    }
    return merged
  }, [problems, backendReport, runHistory])
  const errorCount = issues.filter(issue => issue.level === 'error').length
  const warnCount = issues.filter(issue => issue.level === 'warning').length

  const historyOffset = timeline.length > 0 ? 0 : 1
  const selectedRun =
    historyIndex === 0 ? runHistory[0] : runHistory[historyIndex - 1 + historyOffset]
  const showLive = timeline.length > 0 && historyIndex === 0

  return (
    <div className={`wfc-problems${collapsed ? ' is-collapsed' : ''}`}>
      <div className="wfc-problems-bar">
        <button
          type="button"
          className="wfc-problems-toggle"
          onClick={() => setCollapsed(value => !value)}
        >
          {collapsed ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
        </button>
        <div className="wfc-bottom-main-tabs" role="tablist">
          <button
            type="button"
            className={`wfc-bottom-main-tab${mainTab === 'problems' ? ' is-active' : ''}`}
            onClick={() => {
              setMainTab('problems')
              setCollapsed(false)
            }}
          >
            <ListChecks size={13} /> {t('workflowEditor.problem.title')}
            {errorCount > 0 && (
              <span className="wfc-problems-count wfc-problems-count--error">{errorCount}</span>
            )}
            {warnCount > 0 && (
              <span className="wfc-problems-count wfc-problems-count--warning">{warnCount}</span>
            )}
          </button>
          <button
            type="button"
            className={`wfc-bottom-main-tab${mainTab === 'logs' ? ' is-active' : ''}`}
            onClick={() => {
              setMainTab('logs')
              setCollapsed(false)
            }}
          >
            <ScrollText size={13} /> {t('workflowEditor.problem.logs')}
            {running && (
              <span className="wfc-log-running">{t('workflowEditor.problem.running')}</span>
            )}
          </button>
        </div>

        {mainTab === 'problems' ? (
          <select
            className="wfc-input"
            style={{ width: 'auto' }}
            aria-label={t('workflowEditor.problem.all')}
            value={problemTab}
            onChange={event => setProblemTab(event.target.value as ProblemCategory | 'all')}
          >
            {(['all', 'missing', 'variable', 'invalid', 'runtime', 'structure'] as const).map(
              category => (
                <option key={category} value={category}>
                  {t(`workflowEditor.problem.${category}`)}
                </option>
              ),
            )}
          </select>
        ) : (
          <select
            className="wfc-log-history-select"
            value={historyIndex}
            onChange={event => setHistoryIndex(Number(event.target.value))}
            aria-label={t('workflowEditor.problem.history')}
          >
            <option value={0}>
              {t(
                timeline.length > 0
                  ? 'workflowEditor.problem.current'
                  : 'workflowEditor.problem.recent',
              )}
            </option>
            {runHistory.slice(historyOffset).map((run, index) => (
              <option key={run.run_id} value={index + 1}>
                {formatClock(run.started_at)} · {statusText(run.status)}
              </option>
            ))}
          </select>
        )}
      </div>

      {!collapsed && mainTab === 'problems' && (
        <div className="wfc-problems-list">
          {issues.length === 0 && (
            <div className="wfc-problems-empty">{t('workflowEditor.problem.clear')}</div>
          )}
          {issues
            .filter(issue => problemTab === 'all' || issue.category === problemTab)
            .map((issue, index) => (
              <div
                className={`wfc-problem wfc-problem--${issue.level}`}
                key={`${issue.code}-${issue.stepId}-${issue.fieldPath}-${index}`}
              >
                <span className="wfc-problem-icon">
                  {issue.level === 'error' ? (
                    <CircleAlert size={12} />
                  ) : (
                    <TriangleAlert size={12} />
                  )}
                </span>
                <span className="wfc-problem-node">
                  {issue.stepId ? nameOf(issue.stepId) : '-'}
                </span>
                <div className="wfc-problem-content">
                  <div>
                    {t(`workflowEditor.problem.${issue.category}`)}
                    {issue.fieldPath && <code> · {issue.fieldPath}</code>}
                  </div>
                  <div>
                    {t(`workflowEditor.diagnostic.${issue.code}`) ===
                    `workflowEditor.diagnostic.${issue.code}`
                      ? t('workflowEditor.diagnostic.validation')
                      : t(`workflowEditor.diagnostic.${issue.code}`)}
                  </div>
                  <details>
                    <summary>{t('workflowEditor.problem.details')}</summary>
                    <small>{issue.sources.join(' · ')}</small>
                    {issue.details.map((detail, i) => (
                      <pre key={i}>{detail}</pre>
                    ))}
                  </details>
                </div>
                {issue.stepId && (
                  <button
                    type="button"
                    className="wfc-icon-btn"
                    title={t('workflowEditor.problem.locate')}
                    onClick={() => onLocate(issue.stepId!, issue.fieldPath)}
                  >
                    <Crosshair size={12} />
                  </button>
                )}
              </div>
            ))}
        </div>
      )}

      {!collapsed && mainTab === 'logs' && (
        <div className="wfc-run-log" role="log" aria-live="polite">
          {showLive ? (
            timeline.map(entry => (
              <div className={`wfc-run-log-row is-${entry.level}`} key={entry.id}>
                <span className="wfc-run-log-time">{formatClock(entry.at)}</span>
                <span className="wfc-run-log-event">{entry.event.replace(/_/g, ' ')}</span>
                <span
                  className="wfc-run-log-message"
                  style={{ paddingLeft: `${Math.min(entry.depth ?? 0, 4) * 12}px` }}
                >
                  {entry.message}
                </span>
                {entry.stepId && (
                  <button
                    type="button"
                    className="wfc-icon-btn"
                    title={t('workflowEditor.problem.locate')}
                    onClick={() => onLocate(entry.stepId!)}
                  >
                    <Crosshair size={12} />
                  </button>
                )}
              </div>
            ))
          ) : selectedRun ? (
            <>
              <div className="wfc-run-log-summary">
                <span>{formatClock(selectedRun.started_at)}</span>
                <span>{statusText(selectedRun.status)}</span>
                <span>{durationOf(selectedRun.started_at, selectedRun.finished_at)}</span>
              </div>
              {(selectedRun.steps ?? []).map((step, index) => (
                <div
                  className={`wfc-run-log-row ${typeof step.status === 'object' ? 'is-error' : step.status === 'Success' ? 'is-success' : 'is-warning'}`}
                  key={`${step.step_id}-${index}`}
                >
                  <span className="wfc-run-log-time">{formatClock(step.started_at)}</span>
                  <span className="wfc-run-log-event">{statusText(step.status)}</span>
                  <span className="wfc-run-log-message">
                    {nameOf(step.step_id)}
                    {step.output_summary ? ` · ${step.output_summary}` : ''}
                  </span>
                  <span className="wfc-run-log-duration">
                    {durationOf(step.started_at, step.finished_at)}
                  </span>
                  <button
                    type="button"
                    className="wfc-icon-btn"
                    title={t('workflowEditor.problem.locate')}
                    onClick={() => onLocate(step.step_id)}
                  >
                    <Crosshair size={12} />
                  </button>
                </div>
              ))}
            </>
          ) : (
            <div className="wfc-problems-empty">{t('workflowEditor.problem.noRuns')}</div>
          )}
        </div>
      )}
    </div>
  )
}
