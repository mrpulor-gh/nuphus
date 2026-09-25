import { useCallback, useEffect, useMemo, useState } from 'react'
import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { useLanguage } from '../../locales'
import { CompactModal } from '../layout/CompactModal'
import { wfTraceList, type WorkflowInvocationTrace, type WorkflowRunTrace } from '../lib/api'
import { ExecutionTraceViewer } from './ExecutionTraceViewer'
import { debugDependencies, parseTestValues, wfDebugControl, wfDebugRun } from './debugSession'
import { walkSteps } from './dataEdges'
import './workflow-debug.css'

interface Props {
  workflowId: string
  steps: WorkflowStep[]
  inputs: WorkflowInputSpec[]
  selected: WorkflowStep
  blocked: boolean
  runId: string | null
  onRunStarted: (id: string) => void
  onClose: (keepRunning: boolean) => void
}

export function WorkflowDebugPanel({
  workflowId,
  steps,
  inputs,
  selected,
  blocked,
  runId,
  onRunStarted,
  onClose,
}: Props) {
  const { lang } = useLanguage()
  const text = useCallback((zh: string, en: string) => (lang === 'zh' ? zh : en), [lang])
  const [mode, setMode] = useState<'node' | 'through'>('node')
  const [variables, setVariables] = useState('{}')
  const [runtimeInputs, setRuntimeInputs] = useState('{}')
  const [source, setSource] = useState<Record<string, unknown>>({ kind: 'manual' })
  const [history, setHistory] = useState<{
    run: WorkflowRunTrace
    invocation: WorkflowInvocationTrace
  } | null>(null)
  const [useRetry, setUseRetry] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const [active, setActive] = useState<WorkflowRunTrace | null>(null)
  const [historyOpen, setHistoryOpen] = useState(false)
  const dependencies = useMemo(() => debugDependencies(selected), [selected])
  const scope = useMemo(() => {
    const rows: WorkflowStep[] = []
    walkSteps(mode === 'node' ? [selected] : steps, node => rows.push(node))
    return rows
  }, [mode, steps, selected])
  const inFlight = !!runId && (!active || ['running', 'paused'].includes(active.status))
  useEffect(() => {
    if (!runId) return
    let mounted = true
    const poll = async () => {
      try {
        const runs = await wfTraceList(workflowId, true)
        if (mounted) {
          setActive(runs?.find(run => run.run_id === runId) ?? null)
          setRefresh(value => value + 1)
        }
      } catch (reason) {
        if (mounted) setError(String(reason))
      }
    }
    void poll()
    const timer = setInterval(() => void poll(), 1500)
    return () => {
      mounted = false
      clearInterval(timer)
    }
  }, [workflowId, runId])
  const selectHistory = useCallback(
    (run: WorkflowRunTrace, invocation: WorkflowInvocationTrace) => setHistory({ run, invocation }),
    [],
  )
  const useHistory = (phase: 'before' | 'after') => {
    if (!history) return
    const values =
      phase === 'before' ? history.invocation.variables_before : history.invocation.variables_after
    const { inputs: savedInputs, ...pool } = values
    setVariables(JSON.stringify(pool, null, 2))
    setRuntimeInputs(
      JSON.stringify(savedInputs && typeof savedInputs === 'object' ? savedInputs : {}, null, 2),
    )
    setSource({
      kind: 'history',
      run_id: history.run.run_id,
      invocation_id: history.invocation.id,
      revision: history.run.revision,
      phase,
      modified: false,
    })
  }
  const start = async () => {
    setError('')
    setBusy(true)
    try {
      const response = await wfDebugRun({
        workflow_id: workflowId,
        steps,
        inputs,
        selected_step_id: selected.id,
        mode,
        variables: parseTestValues(variables),
        runtime_inputs: parseTestValues(runtimeInputs),
        use_retry_policy: useRetry,
        source,
      })
      if (!response?.run_id)
        throw new Error(
          text('未收到运行 ID，未确认启动成功。', 'No run ID returned; start not confirmed.'),
        )
      setActive(null)
      onRunStarted(response.run_id)
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }
  const control = async (action: 'pause' | 'resume' | 'cancel') => {
    if (!runId) return
    setBusy(true)
    setError('')
    try {
      await wfDebugControl(workflowId, runId, action)
      setRefresh(value => value + 1)
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }
  return (
    <CompactModal
      open
      onClose={() => onClose(inFlight)}
      title={text('节点调试', 'Node debugging')}
      size="xl"
      className="wfc-debug-modal"
      footer={
        <>
          <button className="wfc-btn" onClick={() => onClose(inFlight)}>
            {text('关闭面板（不终止运行）', 'Close panel (keep running)')}
          </button>
          {inFlight ? (
            <>
              <button
                className="wfc-btn"
                disabled={busy}
                onClick={() => void control(active?.status === 'paused' ? 'resume' : 'pause')}
              >
                {active?.status === 'paused'
                  ? text('继续后续步骤', 'Continue remaining steps')
                  : text('暂停', 'Pause')}
              </button>
              <button className="wfc-btn" disabled={busy} onClick={() => void control('cancel')}>
                {text('停止本次调试', 'Stop this debug run')}
              </button>
            </>
          ) : (
            <button
              className="wfc-btn wfc-btn--primary"
              disabled={busy || blocked}
              onClick={() => void start()}
            >
              {busy
                ? text('启动中…', 'Starting…')
                : mode === 'node'
                  ? text('试运行当前节点', 'Test selected node')
                  : text('运行到选中节点（含）', 'Run through selected node')}
            </button>
          )}
        </>
      }
    >
      <div className="wfc-debug">
        <p>
          <strong>{selected.name || selected.id}</strong> <code>{selected.id}</code>
        </p>
        {error && <p role="alert">{error}</p>}
        <fieldset disabled={busy || inFlight}>
          <label>
            {text('执行范围', 'Execution scope')}
            <select
              className="wfc-input"
              value={mode}
              onChange={event => setMode(event.target.value as 'node' | 'through')}
            >
              <option value="node">
                {text('只运行当前节点 / 容器子树', 'Selected node / container subtree only')}
              </option>
              <option value="through">
                {text('从开头运行，选中节点执行后暂停', 'From start; pause after selected node')}
              </option>
            </select>
          </label>
          <p>
            {mode === 'node'
              ? text(
                  '不会自动重跑前序步骤。发送、保存等操作只在下列节点范围内执行。',
                  'Prerequisites are never replayed automatically. Sends, saves and other actions execute only within the scope below.',
                )
              : text(
                  '会重新执行前序步骤，可能再次发送消息或写入文件。分支和循环按实际条件运行；第一次执行完选中节点后暂停，继续将执行余下流程。',
                  'Preceding steps run again and may repeat sends or writes. Branches and loops follow actual conditions. Pause after the first selected invocation; Continue executes the remaining workflow.',
                )}
          </p>
          <details>
            <summary>
              {text('查看节点范围', 'View scope')} ({scope.length})
            </summary>
            <ol>
              {scope.map(node => (
                <li key={node.id}>
                  {node.name || node.id}
                  {node.id === selected.id ? text(' ← 选中节点', ' ← selected') : ''}
                </li>
              ))}
            </ol>
            {mode === 'through' && (
              <p>
                {text(
                  '上面是完整工作流；实际停止位置由运行路径决定，不会跳过分支强行执行目标。',
                  'This is the full workflow; the actual stopping point follows its execution path. The target is not forced into a skipped branch.',
                )}
              </p>
            )}
          </details>
          <label>
            <input
              type="checkbox"
              checked={useRetry}
              onChange={event => setUseRetry(event.target.checked)}
            />
            {text(
              '使用节点的重试策略（默认关闭，避免重复副作用）',
              'Use node retry policies (off by default to avoid repeated side effects)',
            )}
          </label>
          <p>
            {text(
              '这是实际执行，不是模拟。容器、子工作流和 AI 节点会执行内部操作；调试最长 5 分钟（或工作流已有的较短超时），最多 10,000 次执行检查。',
              'This performs real actions, not a simulation. Containers, sub-workflows and AI nodes include their internal operations. Debug runs have a five-minute limit (or the workflow’s shorter timeout) and 10,000 execution checks.',
            )}
          </p>
          {dependencies.length > 0 && (
            <p>
              {text('当前节点引用的变量：', 'Variables referenced by this node: ')}
              <code>{dependencies.join(', ')}</code>
            </p>
          )}
          <p>
            {text(
              '测试数据只用于本次调试，不修改正式输入或历史记录。历史窗口句柄、文件路径等只是示例，可能已失效，请核对。',
              'Test values apply only to this debug run. Saved inputs and history are unchanged. Historical handles and paths may be stale; check before use.',
            )}
          </p>
          <label>
            {text('变量测试值（JSON 对象）', 'Test variables (JSON object)')}
            <textarea
              className="wfc-input wfc-input--mono"
              rows={5}
              value={variables}
              onChange={event => {
                setVariables(event.target.value)
                setSource(previous => ({ ...(previous as object), modified: true }))
              }}
            />
          </label>
          <label>
            {text('外部输入（JSON 对象）', 'Workflow inputs (JSON object)')}
            <textarea
              className="wfc-input wfc-input--mono"
              rows={4}
              value={runtimeInputs}
              onChange={event => {
                setRuntimeInputs(event.target.value)
                setSource(previous => ({ ...(previous as object), modified: true }))
              }}
            />
          </label>
          <p>
            {text('数据来源', 'Data source')}: <code>{JSON.stringify(source)}</code>
          </p>
        </fieldset>
        {!inFlight && (
          <details open={historyOpen} onToggle={event => setHistoryOpen(event.currentTarget.open)}>
            <summary>{text('从已有运行中选择数据', 'Choose existing execution data')}</summary>
            {historyOpen && (
              <>
                <ExecutionTraceViewer
                  workflowId={workflowId}
                  onInvocationSelected={selectHistory}
                />
                <button
                  className="wfc-btn"
                  disabled={!history || busy}
                  onClick={() => useHistory('before')}
                >
                  {text('使用此调用前的变量', 'Use variables before this invocation')}
                </button>
                <button
                  className="wfc-btn"
                  disabled={!history || busy}
                  onClick={() => useHistory('after')}
                >
                  {text('使用此调用后的变量', 'Use variables after this invocation')}
                </button>
              </>
            )}
          </details>
        )}
        {runId && (
          <section>
            <p>
              {text('本次调试', 'Current debug run')}: <code>{runId}</code> ·{' '}
              {active?.status ?? text('启动中', 'Starting')}
            </p>
            <ExecutionTraceViewer workflowId={workflowId} debug refreshKey={refresh} />
          </section>
        )}
      </div>
    </CompactModal>
  )
}
