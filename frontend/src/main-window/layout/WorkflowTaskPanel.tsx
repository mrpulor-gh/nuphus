// WorkflowTaskPanel.tsx — Native Workflow Step Tree Panel
// Windows 11 inspired design: acrylic background, rounded corners, tree indentation

import { useState, useRef, useEffect } from 'react'
import {
  Sun,
  List,
  RotateCcw,
  DiamondPlus,
  CornerUpRight,
  Clock,
  MessageSquare,
  Circle,
  LoaderCircle,
  Check,
  X,
  Pause,
  Play,
  LayoutGrid,
  Zap,
} from 'lucide-react'
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

// ── Step kind icons — lucide components sized to 12px ──
const KIND_ICONS: Record<string, React.ReactNode> = {
  tool: <Sun size={12} />,
  seq: <List size={12} />,
  loop: <RotateCcw size={12} />,
  if: <DiamondPlus size={12} />,
  call: <CornerUpRight size={12} />,
  wait: <Clock size={12} />,
  chat_agent: <MessageSquare size={12} />,
}

// ── Status icon ──
function StatusBadge({ status }: { status: string }) {
  const dot = {
    pending: <Circle size={10} />,
    running: <LoaderCircle size={10} />,
    completed: <Check size={10} strokeWidth={3} />,
    failed: <X size={10} strokeWidth={3} />,
    paused: <Pause size={10} />,
  }[status] || <Circle size={10} />

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
          <LayoutGrid className="wfst-header-icon" size={14} />
          <span className="wfst-title">工作流</span>
        </div>
        {onClose && (
          // 「收起」而非「关闭」：只隐藏 UI，不清空运行数据——收起后由右下角胶囊
          // 或 Ctrl+Shift+W 展开（旧文案叫"关闭"，容易让人以为数据会被丢弃）。
          <button
            className="wfst-close-btn"
            onClick={onClose}
            aria-label="收起"
            title="收起 (Ctrl+Shift+W)"
          >
            <X size={12} />
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
              <Play size={12} fill="currentColor" />
              继续
            </button>
          ) : allDone ? (
            <button className="wfst-btn wfst-btn-secondary" onClick={onReRun}>
              <RotateCcw size={12} />
              重新执行
            </button>
          ) : (
            <button className="wfst-btn wfst-btn-secondary" onClick={onPause}>
              <Pause size={12} fill="currentColor" />
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
              <Zap size={12} fill="currentColor" />
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
        className="compact-modal--fit"
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
