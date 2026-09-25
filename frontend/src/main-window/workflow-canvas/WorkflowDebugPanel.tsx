import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { WorkflowInputSpec, WorkflowStep } from '../../core/types'
import { useLanguage } from '../../locales'
import { CompactModal } from '../layout/CompactModal'
import {
  wfTraceList,
  wfTraceRead,
  wfValidate,
  type WorkflowInvocationTrace,
  type WorkflowRunTrace,
} from '../lib/api'
import { ExecutionTraceViewer } from './ExecutionTraceViewer'
import {
  debugDependencies,
  debugPreflight,
  parseTestValues,
  wfDebugControl,
  wfDebugRun,
} from './debugSession'
import { walkSteps } from './dataEdges'
import { mergeEditorProblems, type EditorProblem } from './editorProblems'
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
  onLocateIssue?: (stepId: string, fieldPath?: string) => void
}

interface DebugError {
  message: string
  detail?: string
  field?: 'variables' | 'inputs'
  stepId?: string
  fieldPath?: string
}

function durationOf(started?: string, finished?: string | null): string | null {
  if (!started || !finished) return null
  const ms = Date.parse(finished) - Date.parse(started)
  return Number.isFinite(ms) && ms >= 0 ? `${ms} ms` : null
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
  onLocateIssue,
}: Props) {
  const { lang, t } = useLanguage()
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
  const [error, setError] = useState<DebugError | null>(null)
  const [attempted, setAttempted] = useState(false)
  const [hiddenRunId, setHiddenRunId] = useState<string | null>(null)
  const [startedScope, setStartedScope] = useState<{
    runId: string
    mode: 'node' | 'through'
  } | null>(null)
  const [evidence, setEvidence] = useState<{
    runId: string
    trace: WorkflowInvocationTrace
  } | null>(null)
  const [evidenceError, setEvidenceError] = useState('')
  const variablesRef = useRef<HTMLTextAreaElement>(null)
  const inputsRef = useRef<HTMLTextAreaElement>(null)
  const detailsRef = useRef<HTMLElement>(null)
  const [refresh, setRefresh] = useState(0)
  const [active, setActive] = useState<WorkflowRunTrace | null>(null)
  const [historyOpen, setHistoryOpen] = useState(false)
  const dependencies = useMemo(() => debugDependencies(selected), [selected])
  const scope = useMemo(() => {
    const rows: WorkflowStep[] = []
    walkSteps(mode === 'node' ? [selected] : steps, node => rows.push(node))
    return rows
  }, [mode, steps, selected])
  const currentRunId = runId !== hiddenRunId ? runId : null
  const current = active?.run_id === currentRunId ? active : null
  const currentEvidence = evidence?.runId === currentRunId ? evidence.trace : null
  const inFlight = !!currentRunId && (!current || ['running', 'paused'].includes(current.status))
  const terminal = !!current && !inFlight
  useEffect(() => {
    if (busy || !error?.field) return
    ;(error.field === 'variables' ? variablesRef : inputsRef).current?.focus()
  }, [error, busy])
  useEffect(() => {
    if (!currentRunId) return
    let mounted = true
    let polling = false
    const poll = async () => {
      if (polling) return
      polling = true
      try {
        const runs = await wfTraceList(workflowId, true)
        if (mounted) {
          const run = runs?.find(run => run.run_id === currentRunId) ?? null
          setActive(run)
          setRefresh(value => value + 1)
          setEvidenceError('')
          const invocations = run?.invocations ?? []
          const invocation =
            run?.status === 'success'
              ? ([...invocations].reverse().find(item => item.step_id === selected.id) ??
                invocations[invocations.length - 1])
              : invocations[invocations.length - 1]
          if (invocation) {
            const trace = await wfTraceRead(workflowId, currentRunId, true, invocation.id)
            if (mounted && trace) setEvidence({ runId: currentRunId, trace })
          }
        }
      } catch (reason) {
        if (mounted) setEvidenceError(String(reason))
      } finally {
        polling = false
      }
    }
    void poll()
    const timer = setInterval(() => void poll(), 1500)
    return () => {
      mounted = false
      clearInterval(timer)
    }
  }, [workflowId, currentRunId, selected.id])
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
    setError(null)
    setAttempted(true)
    setHiddenRunId(runId)
    setActive(null)
    setEvidence(null)
    setEvidenceError('')
    let testVariables: Record<string, unknown>
    let testInputs: Record<string, unknown>
    try {
      testVariables = parseTestValues(variables)
    } catch (reason) {
      setError({
        message: text(
          '变量测试值格式不正确，请填写 JSON 对象，例如 {"wn": 123}。',
          'Test variables must be a JSON object, for example {"wn": 123}.',
        ),
        detail: String(reason),
        field: 'variables',
      })
      return
    }
    try {
      testInputs = parseTestValues(runtimeInputs)
    } catch (reason) {
      setError({
        message: text(
          '外部输入格式不正确，请填写 JSON 对象，例如 {"name": "测试"}。',
          'Workflow inputs must be a JSON object, for example {"name": "Test"}.',
        ),
        detail: String(reason),
        field: 'inputs',
      })
      return
    }
    const showIssue = (issue: EditorProblem) => {
      const key = `workflowEditor.diagnostic.${issue.code}`
      setError({
        message: t(key) === key ? t('workflowEditor.diagnostic.validation') : t(key),
        detail: issue.details.join('\n'),
        stepId: issue.stepId ?? (issue.fieldPath?.startsWith('/inputs') ? '' : undefined),
        fieldPath: issue.fieldPath,
      })
    }
    const debugSteps = mode === 'node' ? [selected] : steps
    const issue = debugPreflight(debugSteps)
    if (issue) {
      showIssue(issue)
      return
    }
    setBusy(true)
    try {
      const report = await wfValidate({
        id: workflowId,
        name: selected.name || workflowId,
        status: 'Draft',
        steps: debugSteps,
        inputs,
      })
      if (!report)
        throw new Error(
          text(
            '未收到参数检查结果，未启动执行。',
            'No validation response; execution was not started.',
          ),
        )
      const validationIssue = mergeEditorProblems([], report).find(item => item.level === 'error')
      if (validationIssue) {
        showIssue(validationIssue)
        return
      }
      if (!report.passed)
        throw new Error(
          text(
            '参数检查未通过，未启动执行。',
            'Validation did not pass; execution was not started.',
          ),
        )
      const response = await wfDebugRun({
        workflow_id: workflowId,
        steps,
        inputs,
        selected_step_id: selected.id,
        mode,
        variables: testVariables,
        runtime_inputs: testInputs,
        use_retry_policy: useRetry,
        source,
      })
      if (!response?.run_id)
        throw new Error(
          text('未收到运行 ID，未确认启动成功。', 'No run ID returned; start not confirmed.'),
        )
      setActive(null)
      setStartedScope({ runId: response.run_id, mode })
      onRunStarted(response.run_id)
    } catch (reason) {
      setError({
        message: text(
          '本次调试未能启动。请检查节点参数、测试数据及是否有其他任务正在运行，再手动重试。',
          'This debug run could not start. Check node settings, test values and any other running task, then retry manually.',
        ),
        detail: String(reason),
      })
    } finally {
      setBusy(false)
    }
  }
  const control = async (action: 'pause' | 'resume' | 'cancel') => {
    if (!currentRunId) return
    setBusy(true)
    setError(null)
    try {
      await wfDebugControl(workflowId, currentRunId, action)
      setRefresh(value => value + 1)
    } catch (reason) {
      setError({
        message: text(
          '操作未确认成功，请查看本次运行状态后再操作。',
          'The control action was not confirmed. Check the current run status before trying again.',
        ),
        detail: String(reason),
      })
    } finally {
      setBusy(false)
    }
  }
  const lifecycle: Record<string, string> = {
    running: text('执行中', 'Running'),
    paused: text('已暂停', 'Paused'),
    success: text('执行完成', 'Completed'),
    error: text('执行失败', 'Failed'),
    cancelled: text('已停止', 'Stopped'),
    interrupted: text('已中断', 'Interrupted'),
    completed_with_skips: text('执行结束，存在跳过步骤', 'Completed with skipped steps'),
  }
  const status = current
    ? (lifecycle[current.status] ?? current.status)
    : busy
      ? text('启动中…', 'Starting…')
      : error && !currentRunId
        ? text('未启动，请修正后重试', 'Not started; review and retry')
        : currentRunId
          ? text('正在等待运行记录', 'Waiting for run evidence')
          : text('待运行', 'Ready')
  const elapsed = durationOf(current?.started_at, current?.finished_at)
  const output = currentEvidence?.error || current?.error || currentEvidence?.output
  const currentNode = current?.invocations?.[current.invocations.length - 1]
  const recordedMode =
    current?.source && typeof current.source === 'object' && 'mode' in current.source
      ? current.source.mode
      : null
  const summaryMode =
    recordedMode === 'node' || recordedMode === 'through'
      ? recordedMode
      : startedScope?.runId === currentRunId
        ? startedScope.mode
        : mode
  return (
    <CompactModal
      open
      onClose={() => {
        if (!busy) onClose(inFlight)
      }}
      title={text('节点调试', 'Node debugging')}
      size="xl"
      className="wfc-debug-modal"
      footer={
        <>
          <button className="wfc-btn" disabled={busy} onClick={() => onClose(inFlight)}>
            {inFlight
              ? text('关闭面板（不终止运行）', 'Close panel (keep running)')
              : text('关闭面板', 'Close panel')}
          </button>
          {inFlight ? (
            <>
              <button
                className="wfc-btn"
                disabled={busy}
                onClick={() => void control(current?.status === 'paused' ? 'resume' : 'pause')}
              >
                {current?.status === 'paused'
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
                  ? terminal || attempted
                    ? text('重新试运行', 'Retry selected node')
                    : text('试运行当前节点', 'Test selected node')
                  : terminal || attempted
                    ? text('重新从开头运行', 'Restart from beginning')
                    : text('运行到选中节点（含）', 'Run through selected node')}
            </button>
          )}
        </>
      }
    >
      <div className="wfc-debug">
        <section
          className="wfc-debug-summary"
          aria-label={text('本次调试概览', 'Current debug summary')}
        >
          <div className="wfc-debug-summary-row">
            <strong>{selected.name || selected.id}</strong>
            <span role="status" aria-live="polite">
              {status}
            </span>
          </div>
          <p>
            {text('执行范围：', 'Scope: ')}
            {summaryMode === 'node'
              ? text('仅当前节点 / 容器子树', 'Selected node / container subtree')
              : text('从开头运行至选中节点（含）', 'From start through selected node')}
          </p>
          {inFlight && currentNode && (
            <p>
              {text('当前步骤：', 'Current step: ')}
              {currentNode.step_name || currentNode.step_id}
            </p>
          )}
          {elapsed && (
            <p>
              {text('耗时：', 'Duration: ')}
              {elapsed}
            </p>
          )}
          {output && (
            <p className="wfc-debug-output">
              {text(
                currentEvidence?.error || current?.error ? '错误：' : '输出：',
                currentEvidence?.error || current?.error ? 'Error: ' : 'Output: ',
              )}
              {output.slice(0, 240)}
              {output.length > 240 ? '…' : ''}
            </p>
          )}
          {terminal && !output && (
            <p>
              {text('未记录输出，可查看完整详情。', 'No output recorded. Check execution details.')}
            </p>
          )}
          {currentRunId && (
            <button
              className="wfc-btn"
              onClick={() => {
                detailsRef.current?.scrollIntoView({ block: 'start', behavior: 'smooth' })
                detailsRef.current?.focus({ preventScroll: true })
              }}
            >
              {text('查看完整详情', 'View execution details')}
            </button>
          )}
        </section>
        {error && (
          <div className="wfc-debug-error" role="alert">
            <p>{error.message}</p>
            {error.stepId && (
              <p>
                {text('节点：', 'Node: ')}
                {scope.find(node => node.id === error.stepId)?.name || error.stepId}
              </p>
            )}
            {error.stepId !== undefined && onLocateIssue && (
              <button
                className="wfc-btn"
                onClick={() => onLocateIssue(error.stepId!, error.fieldPath)}
              >
                {text('返回节点修正', 'Edit node settings')}
              </button>
            )}
            {error.detail && (
              <details>
                <summary>{text('技术详情', 'Technical details')}</summary>
                <pre>{error.detail}</pre>
              </details>
            )}
          </div>
        )}
        {evidenceError && (
          <div className="wfc-debug-error" role="alert">
            <p>
              {text(
                '无法读取本次运行记录，正在重试读取；不会重跑节点。',
                'Could not read this run’s evidence. Retrying the read only; the node will not run again.',
              )}
            </p>
            <details>
              <summary>{text('技术详情', 'Technical details')}</summary>
              <pre>{evidenceError}</pre>
            </details>
          </div>
        )}
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
              ref={variablesRef}
              aria-invalid={error?.field === 'variables'}
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
              ref={inputsRef}
              aria-invalid={error?.field === 'inputs'}
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
          <details
            open={historyOpen}
            onToggle={event => {
              setHistoryOpen(event.currentTarget.open)
              if (!event.currentTarget.open) setHistory(null)
            }}
          >
            <summary>{text('从已有运行中选择数据', 'Choose existing execution data')}</summary>
            {historyOpen && (
              <>
                <ExecutionTraceViewer
                  workflowId={workflowId}
                  onInvocationSelected={selectHistory}
                  onSelectionUnavailable={() => setHistory(null)}
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
        {currentRunId && (
          <section
            ref={detailsRef}
            tabIndex={-1}
            className="wfc-debug-details"
            aria-label={text('本次调试完整详情', 'Current debug execution details')}
          >
            <h3>{text('本次调试', 'Current debug run')}</h3>
            <ExecutionTraceViewer
              workflowId={workflowId}
              debug
              pinnedRunId={currentRunId}
              refreshKey={refresh}
            />
          </section>
        )}
      </div>
    </CompactModal>
  )
}
