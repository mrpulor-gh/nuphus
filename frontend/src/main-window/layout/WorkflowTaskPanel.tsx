// WorkflowTaskPanel.tsx — Native Workflow Step Tree Panel
// Windows 11 inspired design: acrylic background, rounded corners, tree indentation

import { useState, useRef, useEffect } from 'react'
import { IconSquare } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { CompactModal } from './CompactModal'
import { listWorkflows } from '../lib/api'
import type { WorkflowRunStep } from '../../core/types'

interface WorkflowTaskPanelProps {
  visible: boolean
  steps: WorkflowRunStep[]
  /** 当前/最近一次运行的 workflow id（用于加载步骤参数定义） */
  workflowId?: string | null
  isPaused?: boolean
  onTerminate?: () => void
  onPause?: () => void
  onResume?: () => void
  onClose?: () => void
  /** 重新执行当前工作流 */
  onReRun?: () => void
  /** 紧急停止 — 重置整个会话（force_reset） */
  onForceReset?: () => void
}

// ── 步骤参数查看（只读）──

/** 详情中不展示的字段：V2 顶层标识/容器字段（实际参数在 do 内） */
const HIDDEN_KEYS = new Set([
  'id',
  'name',
  'description',
  'on_error',
  'capture',
  'timeout_secs',
  'do',
])

/** 判断一个数组是否为子步骤数组（容器 children），不作为参数展示 */
function isStepArray(arr: unknown[]): boolean {
  return arr.length > 0 && arr.every(x => !!x && typeof x === 'object' && 'id' in (x as object))
}

/** 拍平 V2 workflow 定义步骤树（do.seq / do.loop.do / do.if.then / do.if.else / do.wait.auto），建立 step_id → 定义 映射 */
function flattenStepDefs(steps: unknown, map: Map<string, Record<string, unknown>>) {
  if (!Array.isArray(steps)) return
  for (const s of steps) {
    if (!s || typeof s !== 'object') continue
    const step = s as Record<string, unknown>
    if (typeof step.id === 'string') map.set(step.id, step)
    const doObj = step.do as Record<string, unknown> | undefined
    if (doObj && typeof doObj === 'object') {
      flattenStepDefs(doObj.seq, map)
      const loop = doObj.loop as Record<string, unknown> | undefined
      if (loop && typeof loop === 'object') flattenStepDefs(loop.do, map)
      const ifDef = doObj.if as Record<string, unknown> | undefined
      if (ifDef && typeof ifDef === 'object') {
        flattenStepDefs(ifDef.then, map)
        flattenStepDefs(ifDef.else, map)
      }
      flattenStepDefs(doObj.auto, map)
    }
  }
}

/** 提取可展示的参数项：V2 参数位于 do 内（tool.with / chat.with 等）；剔除标识/子步骤数组/空值 */
function paramEntriesOf(def: Record<string, unknown>): [string, unknown][] {
  const raw = def.do && typeof def.do === 'object' ? (def.do as Record<string, unknown>) : def
  return Object.entries(raw).filter(([k, v]) => {
    if (HIDDEN_KEYS.has(k)) return false
    if (v === null || v === undefined) return false
    if (Array.isArray(v)) {
      if (v.length === 0) return false
      if (isStepArray(v)) return false
    }
    return true
  })
}

/** 参数值渲染：字符串→多行文本，数字/布尔→mono 单值，对象/数组→JSON */
function ParamValue({ value }: { value: unknown }) {
  if (typeof value === 'string') return <div className="wfst-param-text">{value}</div>
  if (typeof value === 'number' || typeof value === 'boolean') {
    return <code className="wfst-param-mono">{String(value)}</code>
  }
  return <pre className="wfst-param-json">{JSON.stringify(value, null, 2)}</pre>
}

// ── Step kind icons (SVG) ──
const KIND_ICONS: Record<string, React.ReactNode> = {
  tool: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M12 1v4M12 19v4M4.22 4.22l2.83 2.83M16.95 16.95l2.83 2.83M1 12h4M19 12h4M4.22 19.78l2.83-2.83M16.95 7.05l2.83-2.83" />
    </svg>
  ),
  seq: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <line x1="8" y1="6" x2="21" y2="6" />
      <line x1="3" y1="6" x2="5" y2="6" />
      <line x1="8" y1="12" x2="21" y2="12" />
      <line x1="3" y1="12" x2="5" y2="12" />
      <line x1="8" y1="18" x2="21" y2="18" />
      <line x1="3" y1="18" x2="5" y2="18" />
    </svg>
  ),
  loop: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
    </svg>
  ),
  if: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 2L2 12l10 10 10-10L12 2z" />
      <line x1="8" y1="12" x2="16" y2="12" />
      <line x1="12" y1="8" x2="12" y2="16" />
    </svg>
  ),
  call: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <line x1="7" y1="17" x2="17" y2="7" />
      <polyline points="7 7 17 7 17 17" />
    </svg>
  ),
  wait: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <circle cx="12" cy="12" r="10" />
      <polyline points="12 6 12 12 16 14" />
    </svg>
  ),
  chat_agent: (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
    </svg>
  ),
}

// ── Status icon ──
function StatusBadge({ status }: { status: string }) {
  const dot = {
    pending: (
      <svg
        width="10"
        height="10"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      >
        <circle cx="12" cy="12" r="8" />
      </svg>
    ),
    running: (
      <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor">
        <circle cx="12" cy="12" r="8" />
      </svg>
    ),
    completed: (
      <svg
        width="10"
        height="10"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    ),
    failed: (
      <svg
        width="10"
        height="10"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    ),
    paused: (
      <svg width="10" height="10" viewBox="0 0 24 24" fill="currentColor">
        <rect x="6" y="4" width="4" height="16" rx="1" />
        <rect x="14" y="4" width="4" height="16" rx="1" />
      </svg>
    ),
  }[status] || (
    <svg
      width="10"
      height="10"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
    >
      <circle cx="12" cy="12" r="8" />
    </svg>
  )

  return <span className={`wfst-status wfst-status-${status}`}>{dot}</span>
}

// ── New-entry marker (brief highlight) ──
function useNewEntryIds(steps: WorkflowRunStep[]): Set<string> {
  const [newIds, setNewIds] = useState(new Set<string>())
  const prevLen = useRef(0)
  useEffect(() => {
    if (steps.length > prevLen.current) {
      const added = new Set(steps.slice(prevLen.current).map(s => s.id))
      setNewIds(added)
      const timer = setTimeout(() => setNewIds(new Set()), 1800)
      prevLen.current = steps.length
      return () => clearTimeout(timer)
    }
    prevLen.current = steps.length
  }, [steps])
  return newIds
}

// ── Step kind tag (compact colored label) ──
const KIND_COLORS: Record<string, string> = {
  tool: '#3b82f6',
  seq: '#8b5cf6',
  loop: '#f59e0b',
  if: '#10b981',
  call: '#ec4899',
  wait: '#06b6d4',
  chat_agent: '#a78bfa',
}

function KindTag({ kind }: { kind: string }) {
  const color = KIND_COLORS[kind] || '#6b7280'
  return (
    <span className="wfst-kind" style={{ color }}>
      {KIND_ICONS[kind] || '?'}
    </span>
  )
}

// ── Main Component ──
export function WorkflowTaskPanel({
  visible,
  steps,
  workflowId,
  isPaused,
  onTerminate,
  onPause,
  onResume,
  onClose,
  onReRun,
  onForceReset,
}: WorkflowTaskPanelProps) {
  const newIds = useNewEntryIds(steps)
  // 手风琴展开：点击步骤查看定义参数（只读）
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [stepDefs, setStepDefs] = useState<Map<string, Record<string, unknown>> | null>(null)
  // 桌面端终止/紧急停止确认（防误触）：'stop'=终止执行 / 'reset'=重置会话；null=未触发
  const [confirmAction, setConfirmAction] = useState<'stop' | 'reset' | null>(null)
  const treeRef = useRef<HTMLDivElement>(null)

  // 面板可见且有 workflow id 时加载步骤定义（本地文件读取，开销小；
  // visible 变化时重取以覆盖工作流被编辑后的场景）
  useEffect(() => {
    if (!visible || !workflowId) return
    let cancelled = false
    listWorkflows()
      .then(list => {
        if (cancelled) return
        const wf = list.find(w => w.id === workflowId)
        if (!wf) return
        const map = new Map<string, Record<string, unknown>>()
        flattenStepDefs(wf.steps, map)
        setStepDefs(map)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [visible, workflowId])

  // 自动滚动到当前正在执行的步骤
  const runningStepId = steps.find(s => s.status === 'running')?.id
  useEffect(() => {
    if (!treeRef.current || !runningStepId) return
    const el = treeRef.current.querySelector(`[data-step-id="${runningStepId}"]`)
    if (el) el.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  }, [runningStepId])

  if (!visible || steps.length === 0) return null

  const done = steps.filter(s => s.status === 'completed').length
  const failed = steps.filter(s => s.status === 'failed').length
  const total = steps.length
  const pct = total > 0 ? (done / total) * 100 : 0
  // 全部步骤已完成或失败（无 running / pending / paused）
  const allDone = steps.every(s => s.status === 'completed' || s.status === 'failed')

  // Indent per depth level (px)
  const INDENT = 20

  return (
    <div className="wfst-panel">
      {/* ── Header ── */}
      <div className="wfst-header">
        <div className="wfst-header-left">
          <svg
            className="wfst-header-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <rect x="3" y="3" width="7" height="7" />
            <rect x="14" y="3" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" />
            <rect x="14" y="14" width="7" height="7" />
          </svg>
          <span className="wfst-title">工作流</span>
        </div>
        {onClose && (
          <button className="wfst-close-btn" onClick={onClose} aria-label="Close">
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            >
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        )}
      </div>

      {/* ── Progress ── */}
      <div className="wfst-progress">
        <div className="wfst-progress-bar">
          <div className="wfst-progress-track">
            {failed > 0 && (
              <div className="wfst-progress-fail" style={{ width: `${(failed / total) * 100}%` }} />
            )}
            <div className="wfst-progress-fill" style={{ width: `${pct}%` }} />
          </div>
          <span className={`wfst-progress-count ${failed > 0 ? 'has-failed' : ''}`}>
            {done}/{total}
            {failed > 0 && <span className="wfst-failed-badge">{failed}</span>}
          </span>
        </div>
      </div>

      {/* ── Step Tree ── */}
      <div className="wfst-tree" ref={treeRef}>
        {steps.map((step, idx) => {
          const depth = step.depth ?? 0
          const isNew = newIds.has(step.id)
          const isRunning = step.status === 'running'
          const stepDef = stepDefs?.get(step.id)
          const paramEntries = stepDef ? paramEntriesOf(stepDef) : []
          const hasParams = paramEntries.length > 0
          const expanded = expandedId === step.id && hasParams

          return (
            <div key={step.id}>
              <div
                data-step-id={step.id}
                className={[
                  'wfst-node',
                  `wfst-node-${step.status}`,
                  isNew ? 'wfst-node-new' : '',
                  depth > 0 ? 'wfst-node-child' : '',
                  hasParams ? 'wfst-node-expandable' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                style={{ paddingLeft: 12 + depth * INDENT }}
                onClick={() => hasParams && setExpandedId(expanded ? null : step.id)}
              >
                {/* ── Tree connector lines ── */}
                {depth > 0 && (
                  <div className="wfst-connector" style={{ left: 14 + (depth - 1) * INDENT }}>
                    <div className="wfst-connector-line" />
                    <div className="wfst-connector-cap" />
                  </div>
                )}

                {/* ── Status indicator ── */}
                <div className="wfst-node-indicator">
                  {isRunning && <span className="wfst-shimmer-ring" />}
                  <StatusBadge status={step.status} />
                </div>

                {/* ── Kind icon ── */}
                {step.kind && step.kind !== 'tool' && <KindTag kind={step.kind} />}

                {/* ── Step name ── */}
                <span
                  className={[
                    'wfst-node-name',
                    step.status === 'completed' ? 'wfst-name-completed' : '',
                    step.status === 'failed' ? 'wfst-name-failed' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                >
                  {step.name}
                </span>

                {/* ── Running indicator ── */}
                {isRunning && <span className="wfst-running-dot" />}

                {/* ── Params chevron ── */}
                {hasParams && (
                  <span className={`wfst-node-chevron ${expanded ? 'open' : ''}`}>▾</span>
                )}
              </div>

              {/* ── Step params detail（只读）── */}
              {expanded && (
                <div className="wfst-node-detail" style={{ paddingLeft: 12 + depth * INDENT + 20 }}>
                  {paramEntries.map(([key, value]) => (
                    <div key={key} className="wfst-param-row">
                      <span className="wfst-param-key">{key}</span>
                      <ParamValue value={value} />
                    </div>
                  ))}
                </div>
              )}
            </div>
          )
        })}
      </div>

      {/* ── Footer Controls (system-native style buttons) ── */}
      {onTerminate && (
        <div className="wfst-footer">
          {isPaused ? (
            <button className="wfst-btn wfst-btn-primary" onClick={onResume}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                <polygon points="5 3 19 12 5 21 5 3" />
              </svg>
              继续
            </button>
          ) : allDone ? (
            <button className="wfst-btn wfst-btn-secondary" onClick={onReRun}>
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <polyline points="23 4 23 10 17 10" />
                <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
              </svg>
              重新执行
            </button>
          ) : (
            <button className="wfst-btn wfst-btn-secondary" onClick={onPause}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                <rect x="6" y="4" width="4" height="16" rx="1" />
                <rect x="14" y="4" width="4" height="16" rx="1" />
              </svg>
              暂停
            </button>
          )}
          <button className="wfst-btn wfst-btn-danger" onClick={() => setConfirmAction('stop')}>
            <IconSquare size={10} />
            终止
          </button>
          {onForceReset && (
            <button
              className="wfst-btn wfst-btn-danger"
              onClick={() => setConfirmAction('reset')}
              title="紧急停止 — 重置整个会话状态"
            >
              <svg
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
              </svg>
              紧急停止
            </button>
          )}
        </div>
      )}

      {/* 终止 / 紧急停止 确认弹窗（桌面端防误触；确认后执行 onTerminate / onForceReset） */}
      <CompactModal
        open={confirmAction !== null}
        onClose={() => setConfirmAction(null)}
        title={confirmAction === 'reset' ? '紧急停止' : '终止执行'}
        size="sm"
        footer={
          <>
            <Button variant="default" onClick={() => setConfirmAction(null)}>
              取消
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const action = confirmAction
                setConfirmAction(null)
                if (action === 'reset') onForceReset?.()
                else onTerminate?.()
              }}
            >
              {confirmAction === 'reset' ? '紧急停止' : '终止'}
            </Button>
          </>
        }
      >
        <p style={{ margin: 0 }}>
          {confirmAction === 'reset'
            ? '确定重置整个会话状态？将清空当前执行与全部上下文。'
            : '确定终止当前执行？未保存的结果将丢失。'}
        </p>
      </CompactModal>
    </div>
  )
}