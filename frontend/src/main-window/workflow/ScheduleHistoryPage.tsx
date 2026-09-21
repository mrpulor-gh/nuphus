import { useEffect, useMemo, useState } from 'react'
import { Clock3, ExternalLink, RefreshCw, Trash2 } from 'lucide-react'
import type { WorkflowItem } from '../../core/types'
import { Button } from '../../ui/Button'
import {
  listWorkflows,
  wfScheduleHistoryDelete,
  wfScheduleHistoryList,
  type ScheduleHistoryFilter,
  type ScheduleRunRecord,
} from '../lib/api'
import './schedule-history.css'

interface ScheduleHistoryPageProps {
  onOpenReplay: (workflowId: string, runId: string) => void
}

function statusText(status: ScheduleRunRecord['status']): string {
  if (typeof status === 'string') {
    return (
      { Running: '运行中', Success: '成功', Cancelled: '已取消', Paused: '已暂停' }[status] ??
      status
    )
  }
  return '失败'
}

function statusClass(status: ScheduleRunRecord['status']): string {
  if (typeof status === 'object') return 'error'
  return status.toLowerCase()
}

function duration(record: ScheduleRunRecord): string {
  if (!record.finished_at) return '运行中'
  const ms = new Date(record.finished_at).getTime() - new Date(record.started_at).getTime()
  if (!Number.isFinite(ms) || ms < 0) return '-'
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`
}

function resultText(record: ScheduleRunRecord): string {
  if (record.error) return record.error
  const last = [...(record.steps ?? [])].reverse().find(step => step.output_summary)
  return (
    last?.output_summary ??
    (typeof record.status === 'string' && record.status === 'Success' ? '执行完成' : '-')
  )
}

export function ScheduleHistoryPage({ onOpenReplay }: ScheduleHistoryPageProps) {
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
      if (!result) throw new Error('读取定时运行历史失败：后端无响应')
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
    if (total === 0 || !window.confirm('删除当前筛选条件下的全部定时运行历史？此操作不可撤销。'))
      return
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
          <Clock3 size={16} />
          <span>定时运行历史</span>
        </div>
        <Button variant="ghost" size="sm" onClick={() => void load()} disabled={loading}>
          <RefreshCw size={13} />
          刷新
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void clearHistory()}
          disabled={loading || total === 0}
        >
          <Trash2 size={13} />
          清理历史
        </Button>
      </div>
      <div className="schedule-history-filters">
        <label>
          工作流
          <select
            value={workflowId}
            onChange={event => {
              setWorkflowId(event.target.value)
              setPage(0)
            }}
          >
            <option value="">全部工作流</option>
            {workflows.map(item => (
              <option key={item.id} value={item.id}>
                {item.title}
              </option>
            ))}
          </select>
        </label>
        <label>
          状态
          <select
            value={status}
            onChange={event => {
              setStatus(event.target.value as ScheduleHistoryFilter['status'])
              setPage(0)
            }}
          >
            <option value="">全部状态</option>
            <option value="success">成功</option>
            <option value="error">失败</option>
            <option value="running">运行中</option>
            <option value="cancelled">已取消</option>
            <option value="paused">已暂停</option>
          </select>
        </label>
        <label>
          开始日期
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
          结束日期
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
        <div className="schedule-history-empty">加载中...</div>
      ) : runs.length === 0 ? (
        <div className="schedule-history-empty">暂无符合条件的定时运行记录</div>
      ) : (
        <div className="schedule-history-table-wrap">
          <table className="schedule-history-table">
            <thead>
              <tr>
                <th>工作流</th>
                <th>运行时间</th>
                <th>耗时</th>
                <th>状态</th>
                <th>结果</th>
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
                  <td>{duration(run)}</td>
                  <td>
                    <span className={`schedule-history-status is-${statusClass(run.status)}`}>
                      {statusText(run.status)}
                    </span>
                  </td>
                  <td className="schedule-history-result" title={resultText(run)}>
                    {resultText(run)}
                  </td>
                  <td>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => onOpenReplay(run.workflow_id, run.run_id)}
                    >
                      <ExternalLink size={13} />
                      查看回放
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
            上一页
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
            下一页
          </Button>
        </div>
      )}
    </div>
  )
}
