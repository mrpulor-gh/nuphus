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

interface ProblemsPanelProps {
  problems: Problem[]
  backendReport: { errors: string[]; warnings: string[] } | null
  timeline: RunLogEntry[]
  running: boolean
  runHistory: RunRecord[]
  replay?: boolean
  onLocate: (stepId: string) => void
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
  const [collapsed, setCollapsed] = useState(false)
  const [mainTab, setMainTab] = useState<'problems' | 'logs'>('problems')
  const [problemTab, setProblemTab] = useState<'local' | 'backend'>('local')
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

  const errorCount = useMemo(
    () => problems.filter(p => p.level === 'error').length + (backendReport?.errors.length ?? 0),
    [problems, backendReport],
  )
  const warnCount = useMemo(
    () =>
      problems.filter(p => p.level === 'warning').length + (backendReport?.warnings.length ?? 0),
    [problems, backendReport],
  )
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
            <ListChecks size={13} /> 问题
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
            <ScrollText size={13} /> 运行日志
            {running && <span className="wfc-log-running">运行中</span>}
          </button>
        </div>

        {mainTab === 'problems' ? (
          <div className="wfc-problems-tabs">
            <button
              type="button"
              className={`wfc-chip${problemTab === 'local' ? ' is-active' : ''}`}
              onClick={() => setProblemTab('local')}
            >
              前端校验
            </button>
            <button
              type="button"
              className={`wfc-chip${problemTab === 'backend' ? ' is-active' : ''}`}
              onClick={() => setProblemTab('backend')}
            >
              后端校验
              {backendReport
                ? `（${backendReport.errors.length + backendReport.warnings.length}）`
                : ''}
            </button>
          </div>
        ) : (
          <select
            className="wfc-log-history-select"
            value={historyIndex}
            onChange={event => setHistoryIndex(Number(event.target.value))}
            aria-label="选择运行记录"
          >
            <option value={0}>{timeline.length > 0 ? '本次运行' : '最近一次运行'}</option>
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
          {problemTab === 'local' && problems.length === 0 && (
            <div className="wfc-problems-empty">结构校验通过（保存时将由后端做权威校验）</div>
          )}
          {problemTab === 'local' &&
            problems.map((problem, index) => (
              <div
                className={`wfc-problem wfc-problem--${problem.level}`}
                key={`${problem.rule}-${problem.stepId ?? ''}-${index}`}
              >
                <span className="wfc-problem-icon">
                  {problem.level === 'error' ? (
                    <CircleAlert size={12} />
                  ) : (
                    <TriangleAlert size={12} />
                  )}
                </span>
                <span className="wfc-problem-rule">{problem.rule}</span>
                <span className="wfc-problem-node">
                  {problem.stepId ? nameOf(problem.stepId) : '-'}
                </span>
                <span className="wfc-problem-msg" title={problem.message}>
                  {problem.message}
                </span>
                {problem.stepId && (
                  <button
                    type="button"
                    className="wfc-icon-btn"
                    title="定位到节点"
                    onClick={() => onLocate(problem.stepId!)}
                  >
                    <Crosshair size={12} />
                  </button>
                )}
              </div>
            ))}
          {problemTab === 'backend' &&
            (!backendReport ? (
              <div className="wfc-problems-empty">尚未运行过后端校验（保存或点击“检查”触发）</div>
            ) : (
              <>
                {[
                  ...backendReport.errors.map(message => ({ message, level: 'error' as const })),
                  ...backendReport.warnings.map(message => ({
                    message,
                    level: 'warning' as const,
                  })),
                ].map((item, index) => (
                  <div
                    className={`wfc-problem wfc-problem--${item.level}`}
                    key={`${item.level}-${index}`}
                  >
                    <span className="wfc-problem-icon">
                      {item.level === 'error' ? (
                        <CircleAlert size={12} />
                      ) : (
                        <TriangleAlert size={12} />
                      )}
                    </span>
                    <span className="wfc-problem-rule">L3</span>
                    <span className="wfc-problem-msg" title={item.message}>
                      {item.message}
                    </span>
                  </div>
                ))}
                {backendReport.errors.length === 0 && backendReport.warnings.length === 0 && (
                  <div className="wfc-problems-empty">后端权威校验通过</div>
                )}
              </>
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
                    title="定位到节点"
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
                    title="定位到节点"
                    onClick={() => onLocate(step.step_id)}
                  >
                    <Crosshair size={12} />
                  </button>
                </div>
              ))}
            </>
          ) : (
            <div className="wfc-problems-empty">尚无运行记录</div>
          )}
        </div>
      )}
    </div>
  )
}
