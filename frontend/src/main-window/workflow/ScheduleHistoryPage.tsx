import { useEffect, useMemo, useState } from 'react'
import type { WorkflowItem } from '../../core/types'
import { Button } from '../../ui/Button'
import { IconClock3, IconExternalLink, IconRefresh, IconTrash2 } from '../../ui/Icons'
import {
  listWorkflows,
  wfScheduleHistoryDelete,
  wfScheduleHistoryList,
  type ScheduleHistoryFilter,
  type ScheduleRunRecord,
} from '../lib/api'
import './schedule-history.css'
import { useLanguage } from '../../locales'

interface ScheduleHistoryPageProps {
  onOpenReplay: (workflowId: string, runId: string) => void
}

type TFunc = (key: string, ...args: string[]) => string

/** 词条取值：字典命中 → 用字典文案；未命中 → 用中文兜底。 */
const tr = (t: TFunc, key: string, fallback: string): string => {
  const v = t(key)
  return v === key ? fallback : v
}

/** 页内文案表：按当前语言生成，键名稳定。 */
function makeTXT(t: TFunc) {
  return {
    heading: tr(t, 'schedule.heading', '定时运行历史'),
    refresh: tr(t, 'schedule.refresh', '刷新'),
    clearHistory: tr(t, 'schedule.clearHistory', '清理历史'),
    clearConfirm: tr(
      t,
      'schedule.clearConfirm',
      '删除当前筛选条件下的全部定时运行历史？此操作不可撤销。',
    ),
    filterWorkflow: tr(t, 'schedule.filterWorkflow', '工作流'),
    allWorkflows: tr(t, 'schedule.allWorkflows', '全部工作流'),
    filterStatus: tr(t, 'schedule.filterStatus', '状态'),
    allStatuses: tr(t, 'schedule.allStatuses', '全部状态'),
    statusRunning: tr(t, 'schedule.statusRunning', '运行中'),
    statusSuccess: tr(t, 'schedule.statusSuccess', '成功'),
    statusError: tr(t, 'schedule.statusError', '失败'),
    statusCancelled: tr(t, 'schedule.statusCancelled', '已取消'),
    statusPaused: tr(t, 'schedule.statusPaused', '已暂停'),
    fromDate: tr(t, 'schedule.fromDate', '开始日期'),
    toDate: tr(t, 'schedule.toDate', '结束日期'),
    loading: tr(t, 'schedule.loading', '加载中...'),
    empty: tr(t, 'schedule.empty', '暂无符合条件的定时运行记录'),
    colWorkflow: tr(t, 'schedule.colWorkflow', '工作流'),
    colStartedAt: tr(t, 'schedule.colStartedAt', '运行时间'),
    colDuration: tr(t, 'schedule.colDuration', '耗时'),
    colStatus: tr(t, 'schedule.colStatus', '状态'),
    colResult: tr(t, 'schedule.colResult', '结果'),
    viewReplay: tr(t, 'schedule.viewReplay', '查看回放'),
    prevPage: tr(t, 'schedule.prevPage', '上一页'),
    nextPage: tr(t, 'schedule.nextPage', '下一页'),
    errNoResponse: tr(t, 'schedule.errNoResponse', '读取定时运行历史失败：后端无响应'),
    running: tr(t, 'schedule.running', '运行中'),
    done: tr(t, 'schedule.done', '执行完成'),
  }
}

function statusText(status: ScheduleRunRecord['status'], TXT: ReturnType<typeof makeTXT>): string {
  if (typeof status === 'string') {
    return (
      {
        Running: TXT.statusRunning,
        Success: TXT.statusSuccess,
        Cancelled: TXT.statusCancelled,
        Paused: TXT.statusPaused,
      }[status] ?? status
    )
  }
  return TXT.statusError
}

function statusClass(status: ScheduleRunRecord['status']): string {
  if (typeof status === 'object') return 'error'
  return status.toLowerCase()
}

function duration(record: ScheduleRunRecord, TXT: ReturnType<typeof makeTXT>): string {
  if (!record.finished_at) return TXT.running
  const ms = new Date(record.finished_at).getTime() - new Date(record.started_at).getTime()
  if (!Number.isFinite(ms) || ms < 0) return '-'
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`
}

function resultText(record: ScheduleRunRecord, TXT: ReturnType<typeof makeTXT>): string {
  if (record.error) return record.error
  const last = [...(record.steps ?? [])].reverse().find(step => step.output_summary)
  return (
    last?.output_summary ??
    (typeof record.status === 'string' && record.status === 'Success' ? TXT.done : '-')
  )
}

export function ScheduleHistoryPage({ onOpenReplay }: ScheduleHistoryPageProps) {
  const { t } = useLanguage()
  const TXT = makeTXT(t)
  const [workflows, setWorkflows] = useState<WorkflowItem[]>([])
  const [runs, setRuns] = useState<ScheduleRunRecord[]>([])
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(0)
  const [workflowId, setWorkflowId] = useState('')
  const [status, setStatus] = useState<ScheduleHistoryFilter['status'] | ''>('')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const filter = useMemo<ScheduleHistoryFilter>(
    () => ({
      ...(workflowId ? { workflow_id: workflowId } : {}),
      ...(status ? { status } : {}),
      ...(from ? { from: new Date(`${from}T00:00:00`).toISOString() } : {}),
      ...(to ? { to: new Date(`${to}T23:59:59.999`).toISOString() } : {}),
      page,
      page_size: 50,
    }),
    [workflowId, status, from, to, page],
  )

  const load = async () => {
    setLoading(true)
    setError(null)
    try {
      const result = await wfScheduleHistoryList(filter)
      if (!result) throw new Error(TXT.errNoResponse)
      setRuns(result.runs)
      setTotal(result.total)
    } catch (reason) {
      setError(String(reason))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    void listWorkflows()
      .then(items => setWorkflows(items ?? []))
      .catch(() => setWorkflows([]))
  }, [])

  useEffect(() => {
    void load()
  }, [filter])

  const clearHistory = async () => {
    if (total === 0 || !window.confirm(TXT.clearConfirm)) return
    setError(null)
    try {
      await wfScheduleHistoryDelete(filter)
      setPage(0)
      await load()
    } catch (reason) {
      setError(String(reason))
    }
  }

  return (
    <div className="schedule-history-page">
      <div className="schedule-history-toolbar">
        <div className="schedule-history-heading">
          <IconClock3 size={16} />
          <span>{TXT.heading}</span>
        </div>
        <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
          <IconRefresh size={13} />
          {TXT.refresh}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void clearHistory()}
          disabled={loading || total === 0}
        >
          <IconTrash2 size={13} />
          {TXT.clearHistory}
        </Button>
      </div>
      <div className="schedule-history-filters">
        <label>
          {TXT.filterWorkflow}
          <select
            value={workflowId}
            onChange={event => {
              setWorkflowId(event.target.value)
              setPage(0)
            }}
          >
            <option value="">{TXT.allWorkflows}</option>
            {workflows.map(item => (
              <option key={item.id} value={item.id}>
                {item.title}
              </option>
            ))}
          </select>
        </label>
        <label>
          {TXT.filterStatus}
          <select
            value={status}
            onChange={event => {
              setStatus(event.target.value as ScheduleHistoryFilter['status'])
              setPage(0)
            }}
          >
            <option value="">{TXT.allStatuses}</option>
            <option value="success">{TXT.statusSuccess}</option>
            <option value="error">{TXT.statusError}</option>
            <option value="running">{TXT.statusRunning}</option>
            <option value="cancelled">{TXT.statusCancelled}</option>
            <option value="paused">{TXT.statusPaused}</option>
          </select>
        </label>
        <label>
          {TXT.fromDate}
          <input
            type="date"
            value={from}
            onChange={event => {
              setFrom(event.target.value)
              setPage(0)
            }}
          />
        </label>
        <label>
          {TXT.toDate}
          <input
            type="date"
            value={to}
            onChange={event => {
              setTo(event.target.value)
              setPage(0)
            }}
          />
        </label>
      </div>
      {error && <div className="schedule-history-error">{error}</div>}
      {loading ? (
        <div className="schedule-history-empty">{TXT.loading}</div>
      ) : runs.length === 0 ? (
        <div className="schedule-history-empty">{TXT.empty}</div>
      ) : (
        <div className="schedule-history-table-wrap">
          <table className="schedule-history-table">
            <thead>
              <tr>
                <th>{TXT.colWorkflow}</th>
                <th>{TXT.colStartedAt}</th>
                <th>{TXT.colDuration}</th>
                <th>{TXT.colStatus}</th>
                <th>{TXT.colResult}</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {runs.map(run => (
                <tr key={run.run_id}>
                  <td>
                    <strong>{run.workflow_title}</strong>
                    <small>{run.workflow_id}</small>
                  </td>
                  <td>{new Date(run.started_at).toLocaleString()}</td>
                  <td>{duration(run, TXT)}</td>
                  <td>
                    <span className={`schedule-history-status is-${statusClass(run.status)}`}>
                      {statusText(run.status, TXT)}
                    </span>
                  </td>
                  <td className="schedule-history-result" title={resultText(run, TXT)}>
                    {resultText(run, TXT)}
                  </td>
                  <td>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => onOpenReplay(run.workflow_id, run.run_id)}
                    >
                      <IconExternalLink size={13} />
                      {TXT.viewReplay}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {total > 50 && (
        <div className="schedule-history-pagination">
          <Button
            variant="ghost"
            size="sm"
            disabled={page === 0}
            onClick={() => setPage(value => value - 1)}
          >
            {TXT.prevPage}
          </Button>
          <span>
            {page + 1} / {Math.ceil(total / 50)}
          </span>
          <Button
            variant="ghost"
            size="sm"
            disabled={(page + 1) * 50 >= total}
            onClick={() => setPage(value => value + 1)}
          >
            {TXT.nextPage}
          </Button>
        </div>
      )}
    </div>
  )
}
