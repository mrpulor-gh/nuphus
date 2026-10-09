// ExecutionTraceFloating.tsx — Central execution trace panel
// Desktop app style: center floating, not web popup, with material depth

import { useState, useRef, useEffect, useLayoutEffect, useCallback, useMemo, Fragment } from 'react'
import {
  IconX,
  IconTerminal,
  IconSearch,
  IconCheck,
  IconPlay,
  IconCircle,
  IconChevronUp,
  IconChevronDown,
  IconStar,
  IconRefresh,
} from '../../ui/Icons'
import { NuphusAvatar } from '../../ui/NuphusAvatar'
import MarkdownContent from '../chat/MarkdownContent'
import type { TimelineEntry, TurnMeta } from '../../core/types'
import { resolveTurnCalls, resolveTurnDuration } from '../../core/types'
import { desktopActionResult } from '../lib/desktopActionResult'
import { DesktopActionStatus } from './DesktopActionStatus'
import { useStickyScroll } from '../../hooks/useStickyScroll'

interface ExecutionTraceProps {
  timeline: TimelineEntry[]
  /** 气泡执行回溯覆盖：非空时显示该轮历史执行过程（替代全局 timeline） */
  traceOverride?: TimelineEntry[] | null
  stepIndex: number
  progress: { iteration: number; max: number; calls: number }
  isProcessing: boolean
  completed: boolean
  expandedCalls: Set<string>
  onToggleExpand: (id: string) => void
  goal?: string
  totalDurationMs?: number
  totalCalls?: number
  /** 执行中实时步数（timeline 派生）——totalCalls 是完成态值，执行中恒 0 */
  liveCalls?: number
  /** 本轮元数据（耗时 / token / 步数）——后端权威，与输入栏 ctx / 消息底部同源 */
  turnMeta?: TurnMeta | null
  onRate?: (name: string, rating: number, comment: string, saveAsStrategy: boolean) => void
  onRegenerate?: () => void
  // 新增：控制显示/隐藏
  visible?: boolean
  onClose?: () => void
  // 当前运行模式：leader | workflow
  mode?: string
}

// ── Helpers ──
function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(1)}s`
}

/** Parse phase tag at start of LLM response, returns { tag, remainingText } */
function parsePhaseTag(text: string): { tag: string | null; remaining: string } {
  const trimmed = text.trimStart()
  const match = trimmed.match(/^\[(Observe|Analyze|Execute|Verify)\]\s*/)
  if (match) {
    return { tag: match[1], remaining: trimmed.slice(match[0].length) }
  }
  return { tag: null, remaining: text }
}

/** Phase tag color mapping */
function phaseTagColor(tag: string): string {
  switch (tag) {
    case 'Observe':
      return '#3b82f6' // blue
    case 'Analyze':
      return '#8b5cf6' // purple
    case 'Execute':
      return '#10b981' // green
    case 'Verify':
      return '#f59e0b' // amber
    default:
      return '#6b7280'
  }
}

/** Infer phase tag from tool name (internal mechanism, doesn't rely on LLM output) */
function toolPhaseTag(toolName: string): string | null {
  const observeTools = [
    'Read',
    'Grep',
    'Glob',
    'ListDir',
    'FilesInfo',
    'desktop_screenshot',
    'desktop_windows_list',
    'desktop_window_info',
    'desktop_ocr',
    'desktop_clipboard_clean',
    'desktop_mouse_position',
    'desktop_screen_size',
    'desktop_find_image',
    'desktop_find_color',
    'browser_screenshot',
    'browser_navigate',
    'memory_search',
    'memory_query',
    'recent_timeline',
    'timeline_stats',
    'session_history',
    'planner_list',
    'planner_parse',
  ]
  const executeTools = [
    'Edit',
    'Write',
    'Append',
    'Delete',
    'Copy',
    'Rename',
    'CreateDir',
    'RemoveDir',
    'task_dispatch',
    'workflow_run',
    'planner_create',
    'planner_edit',
    'planner_request_review',
    'planner_submit_review',
    'desktop_mouse_click',
    'desktop_mouse_hover',
    'desktop_mouse_drag',
    'desktop_mouse_scroll',
    'desktop_keyboard_press',
    'desktop_keyboard_hotkey',
    'desktop_window_activate',
    'desktop_window_move',
    'desktop_window_resize',
    'desktop_clipboard_write',
    'desktop_input',
    'desktop_find_multi_color',
    'browser_click',
    'browser_type',
    'browser_evaluate',
    'process_kill',
    'schedule_cron',
  ]
  const verifyTools = ['system_shell', 'system_info', 'system_env_get', 'system_env_set']
  if (observeTools.includes(toolName)) return 'Observe'
  if (executeTools.includes(toolName)) return 'Execute'
  if (verifyTools.includes(toolName)) return 'Verify'
  return null
}

// ── Terminal mode tool categories (for differentiated output rendering) ──
// Read-only tools: show command line only, no output body
const READ_ONLY_TOOLS = new Set([
  'Read',
  'Glob',
  'Grep',
  'FilesInfo',
  'ListDir',
  'memory_search',
  'memory_recent',
  'memory_session_context',
  'memory_stats',
  'web_search',
  'web_extract',
  'knowledge_search',
  'skill_query',
  'skill_read',
  'system_info',
  'system_env_get',
  'process_list',
  'browser_snapshot',
  'browser_extract',
  'planner_list',
  'planner_parse',
  'desktop_screenshot',
  'desktop_screen_size',
  'desktop_windows_list',
  'desktop_window_info',
  'desktop_window_screenshot',
  'desktop_ocr',
  'desktop_clipboard_clean',
  'desktop_find_image',
  'desktop_find_color',
  'desktop_find_multi_color',
  'desktop_find_text',
])

// Write/edit tools: full output, no line limit
const WRITE_TOOLS = new Set(['Write', 'Edit', 'Append'])

// Exec tools: extended 50-line preview (user needs to see command output)
const EXEC_TOOLS = new Set([
  'system_shell',
  'process_kill',
  'task_dispatch',
  'workflow_run',
  'browser_evaluate',
])

type ToolCategory = 'read' | 'write' | 'exec' | 'default'

function getToolCategory(toolName: string): ToolCategory {
  if (READ_ONLY_TOOLS.has(toolName)) return 'read'
  if (WRITE_TOOLS.has(toolName)) return 'write'
  if (EXEC_TOOLS.has(toolName)) return 'exec'
  return 'default'
}

// ── Diff display ──
// Generates a unified-diff-like text with line numbers, e.g.:
//   Added 1 line, removed 1 line
//          5    plugins: [react()],
//          6    base: './',
//          8 -  port: 5174,
//          8 +  port: 5173,
//          9    strictPort: true,
function computeDiff(oldStr: string, newStr: string): React.ReactNode {
  if (!oldStr && !newStr) return null
  const oldLines = oldStr.split('\n')
  const newLines = newStr.split('\n')
  const m = oldLines.length,
    n = newLines.length
  const dp: number[][] = Array.from({ length: m + 1 }, () => new Array(n + 1).fill(0))
  for (let i = 1; i <= m; i++)
    for (let j = 1; j <= n; j++)
      dp[i][j] =
        oldLines[i - 1] === newLines[j - 1]
          ? dp[i - 1][j - 1] + 1
          : Math.max(dp[i - 1][j], dp[i][j - 1])
  // Walk back to collect LCS alignment
  const seq: { type: 'equal' | 'add' | 'remove'; text: string }[] = []
  let i = m,
    j = n
  while (i > 0 || j > 0) {
    if (i > 0 && j > 0 && oldLines[i - 1] === newLines[j - 1]) {
      seq.push({ type: 'equal', text: oldLines[i - 1] })
      i--
      j--
    } else if (j > 0 && (i === 0 || dp[i][j - 1] >= dp[i - 1][j])) {
      seq.push({ type: 'add', text: newLines[j - 1] })
      j--
    } else {
      seq.push({ type: 'remove', text: oldLines[i - 1] })
      i--
    }
  }
  seq.reverse()

  let addCount = 0,
    removeCount = 0
  for (const s of seq) {
    if (s.type === 'add') addCount++
    else if (s.type === 'remove') removeCount++
  }

  // Header
  const header: string[] = []
  if (addCount > 0) header.push(`Added ${addCount} line${addCount > 1 ? 's' : ''}`)
  if (removeCount > 0) header.push(`removed ${removeCount} line${removeCount > 1 ? 's' : ''}`)

  return (
    <>
      {header.length > 0 && <div className="tc-diff-header">{header.join(', ')}</div>}
      {seq.map((s, idx) => {
        const marker = s.type === 'add' ? '+' : s.type === 'remove' ? '-' : ' '
        return (
          <div key={idx} className={`tc-diff-line ${s.type}`}>
            <span className="tc-diff-marker">{marker}</span>
            <span className="tc-diff-text">{s.text}</span>
          </div>
        )
      })}
    </>
  )
}

// ── Tool Detail (different expanded views per tool) ──
function ToolDetail({ entry }: { entry: TimelineEntry }) {
  const paramsStr = entry.params ? JSON.stringify(entry.params, null, 2) : ''
  const outputStr = entry.output || ''
  const p = entry.params as Record<string, unknown> | undefined

  if (entry.toolName === 'Edit' || entry.toolName === 'patch') {
    const oldStr = (p?.old_string as string) || ''
    const newStr = (p?.new_string as string) || ''
    const diffText = computeDiff(oldStr, newStr)
    const path = (p?.path as string) || ''
    return (
      <div className="tc-expanded">
        {path && <div className="tc-expanded-path">{path}</div>}
        {diffText && <div className="tc-diff">{diffText}</div>}
      </div>
    )
  }

  if (entry.toolName === 'task_dispatch') {
    // 新契约：title（人看）+ brief（给 Exec）+ task_no（可选归属标签）。
    // 旧契约的 description/task_id/total_tasks 仍兼容读取（老上下文与历史记录）。
    const description = (p?.title as string) || (p?.description as string) || ''
    const goalType = (p?.goal_type as string) || ''
    const planPath = (p?.plan_path as string) || ''
    const taskNo = p?.task_no as number | undefined

    // Output is JSON { status, summary, run } — extract summary for cleaner display
    let summary = outputStr
    let execStatus = ''
    try {
      const parsed = JSON.parse(outputStr)
      if (parsed.summary) summary = parsed.summary
      if (parsed.status) execStatus = parsed.status
    } catch {
      /* output is plain text, use as-is */
    }

    return (
      <div className="tc-expanded">
        <div className="tc-task-header">
          <span className="tc-task-icon" />
          <span className="tc-task-desc">{description || 'Subtask'}</span>
          {execStatus && (
            <span className={`tc-task-status ${execStatus === 'success' ? 'ok' : 'fail'}`}>
              {execStatus === 'success' ? '完成' : '失败'}
            </span>
          )}
        </div>
        {(goalType || planPath || taskNo != null) && (
          <div className="tc-task-meta">
            {goalType && <span className="tc-meta-tag">{goalType}</span>}
            {planPath && <span className="tc-meta-tag tc-meta-path">{planPath}</span>}
            {taskNo != null && <span className="tc-meta-progress">计划#{taskNo}</span>}
          </div>
        )}
        {summary && (
          <div className="tc-code-block tc-task-summary">
            <MarkdownContent content={summary} />
          </div>
        )}
      </div>
    )
  }

  if (entry.toolName === 'workflow_run') {
    const wfId = (p?.id as string) || ''
    // Determine status from output
    let wfStatus = ''
    let wfSummary = outputStr
    try {
      const parsed = JSON.parse(outputStr)
      if (parsed.summary) wfSummary = parsed.summary
      if (parsed.status) wfStatus = parsed.status
    } catch {}
    return (
      <div className="tc-expanded">
        <div className="tc-task-header">
          <span className="tc-task-icon tc-task-icon--wf" />
          <span className="tc-task-desc">工作流{wfId ? `: ${wfId}` : ''}</span>
          {wfStatus && (
            <span className={`tc-task-status ${wfStatus === 'success' ? 'ok' : 'fail'}`}>
              {wfStatus === 'success' ? '完成' : '失败'}
            </span>
          )}
        </div>
        {wfSummary && (
          <div className="tc-code-block tc-task-summary">
            <MarkdownContent content={wfSummary} />
          </div>
        )}
      </div>
    )
  }

  if (entry.toolName === 'Read') {
    const path = (p?.path as string) || ''
    const lines = outputStr.split('\n')
    const maxPreview = 20
    const isLong = lines.length > maxPreview
    const display = isLong
      ? lines.slice(0, maxPreview).join('\n') +
        `\n\n… ${lines.length - maxPreview} more lines (${outputStr.length} chars total, click params to see full)`
      : outputStr
    return (
      <div className="tc-expanded">
        {path && <div className="tc-expanded-path">{path}</div>}
        {display && (
          <div className="tc-code-block">
            <MarkdownContent content={display} />
          </div>
        )}
      </div>
    )
  }

  if (entry.toolName === 'Write' || entry.toolName === 'write') {
    const path = (p?.path as string) || ''
    const content = (p?.content as string) || ''
    return (
      <div className="tc-expanded">
        {path && <div className="tc-expanded-path">{path}</div>}
        {content && (
          <div className="tc-code-block">
            <MarkdownContent content={content} />
          </div>
        )}
      </div>
    )
  }

  if (entry.toolName === 'execute_shell' || entry.toolName === 'terminal') {
    const cmd = (p?.command as string) || ''
    return (
      <div className="tc-expanded">
        <div className="tc-shell-command">
          <span className="tc-shell-prompt">$</span>
          <code>{cmd}</code>
        </div>
        {outputStr && (
          <div className="tc-shell-output">
            <MarkdownContent content={outputStr} />
          </div>
        )}
        {entry.isTruncated && entry.outputFullSize && (
          <div className="tc-truncated">
            ... output truncated ({entry.outputFullSize} chars total)
          </div>
        )}
      </div>
    )
  }

  if (entry.toolName === 'Grep' || entry.toolName === 'Glob') {
    const pattern = (p?.pattern as string) || (p?.query as string) || ''
    return (
      <div className="tc-expanded">
        {pattern && (
          <div className="tc-expanded-path">
            <IconSearch
              size={11}
              strokeWidth={2.5}
              style={{ marginRight: 5, opacity: 0.5, flexShrink: 0 }}
            />
            {pattern}
          </div>
        )}
        {outputStr && (
          <div className="tc-code-block">
            <MarkdownContent content={outputStr} />
          </div>
        )}
      </div>
    )
  }

  // Generic expand — use MarkdownContent for code highlighting in output
  return (
    <div className="tc-expanded">
      {paramsStr && (
        <div className="tc-expanded-section">
          <div className="tc-expanded-label">Params</div>
          <pre className="tc-code-block">{paramsStr}</pre>
        </div>
      )}
      {outputStr && (
        <div className="tc-expanded-section">
          <div className="tc-expanded-label">Output</div>
          <div className="tc-code-block">
            <MarkdownContent content={outputStr} />
          </div>
        </div>
      )}
      {entry.isTruncated && entry.outputFullSize && (
        <div className="tc-truncated">
          ... output truncated ({entry.outputFullSize} chars total)
        </div>
      )}
    </div>
  )
}

// ── Status Icon ──
function StatusIcon({ status }: { status?: string }) {
  return (
    <span className={`tc-status-icon ${status || 'pending'}`}>
      {status === 'success' ? (
        <IconCheck size={12} strokeWidth={3} />
      ) : status === 'running' ? (
        <IconPlay size={12} fill="currentColor" strokeWidth={0} />
      ) : status === 'error' ? (
        <IconX size={12} strokeWidth={3} />
      ) : (
        <IconCircle size={10} />
      )}
    </span>
  )
}

// ── 工具步骤状态微标签（UI 卡片模式 tc-option 徽标文案）──
function toolStatusLabel(status?: string): string {
  if (status === 'running') return '运行中'
  if (status === 'success') return '完成'
  if (status === 'error') return '失败'
  return ''
}

// ── Tool icon (different tools suggest different icons) ──
function ToolIcon({ name }: { name: string }) {
  // Use simple text label instead, don't increase complexity
  return null
}

// ── Rating Modal ──
export function RatingModal({
  goal,
  toolCalls,
  totalMs,
  onClose,
  onSubmit,
}: {
  goal: string
  toolCalls: TimelineEntry[]
  totalMs: number
  onClose: () => void
  onSubmit: (name: string, rating: number, comment: string, saveAsStrategy: boolean) => void
}) {
  const [rating, setRating] = useState(0)
  const [hoverRating, setHoverRating] = useState(0)
  const [comment, setComment] = useState('')
  const [name, setName] = useState(goal)

  const chain = toolCalls
    .filter(t => t.kind === 'tool_call')
    .map(t => t.toolName || '')
    .filter(Boolean)
  const successCount = toolCalls.filter(t => t.status === 'success').length
  const totalCount = chain.length

  const handleSubmit = (saveAsStrategy: boolean) => {
    if (rating === 0) return
    onSubmit(name.trim() || goal || '未命名任务', rating, comment, saveAsStrategy)
    onClose()
  }

  return (
    <div className="rating-modal-overlay" onClick={onClose}>
      <div className="rating-modal" onClick={e => e.stopPropagation()}>
        <div className="rating-modal-header">点评本次执行</div>
        <div className="rating-modal-summary">
          <input
            className="rating-modal-goal-input"
            value={name}
            onChange={e => setName(e.target.value)}
            placeholder="未命名任务"
          />
          <div className="rating-modal-meta">
            {totalCount} 步 · {formatMs(totalMs)} · {successCount}/{totalCount} 成功
          </div>
        </div>
        <div className="rating-modal-stars">
          {[1, 2, 3, 4, 5].map(star => (
            <button
              key={star}
              className={`rating-star ${star <= (hoverRating || rating) ? 'active' : ''}`}
              onMouseEnter={() => setHoverRating(star)}
              onMouseLeave={() => setHoverRating(0)}
              onClick={() => setRating(star)}
            >
              ★
            </button>
          ))}
        </div>
        <textarea
          className="rating-modal-comment"
          placeholder="这次执行如何？有什么建议..."
          value={comment}
          onChange={e => setComment(e.target.value)}
          rows={4}
        />
        <div className="rating-modal-actions">
          <button className="rating-btn secondary" onClick={onClose}>
            取消
          </button>
          <button
            className="rating-btn primary"
            disabled={rating === 0}
            onClick={() => handleSubmit(rating >= 4)}
          >
            {rating >= 4 ? '保存为策略' : '保存点评'}
          </button>
        </div>
      </div>
    </div>
  )
}

// ================================================================

/**
 * 历史折叠默认档：只完整渲染最新这么多**步**（1 步 = 1 条 tool_call），更早的收进折叠条。
 *
 * 为什么需要：本面板是全量渲染（没有虚拟列表），而每一步都可能带整段思考文本，
 * 上百步时 DOM 与文本量会把刷新、滚动都拖到掉帧。折叠是最低成本的减法——默认
 * 只看最近若干步，上滚即可逐档续展，功能一点不少。
 *
 * 30 的取法：面板高度约 80vh，一屏大致能看十几步，30 步足够覆盖「回看刚才发生了什么」
 * 这一常见诉求；再往上加就开始磨损收益了。
 */
const RECENT_STEPS_KEPT = 30

/** 每次自动续展的步数（30 → 60 → 90 …）：用户上滚到折叠条即再放出一档 */
const STEPS_PER_EXPAND = 30

/**
 * 折叠条触发带（px）：滚动容器 scrollTop 落进该带内 = 折叠条已贴到视口顶。折叠条是
 * 内容首元素，两种渲染路径的容器 padding（≤10px）都远小于此值，故无需量 rect。
 * 到带内即视为「上滚到折叠条」自动续展一档；补偿后 scrollTop 被推回带外，不会连环触发。
 */
const FOLD_TRIGGER_BAND_PX = 32

/** 上滚判定阈值（px）：与 useStickyScroll 的方向判定同量级，抹掉亚像素抖动 */
const FOLD_SCROLL_DELTA_EPS_PX = 2

/**
 * 历史折叠行：折叠时提示可上滚展开，展开后提供收回。
 *
 * 抽成组件而不是在两处渲染里各写一遍：终端模式与卡片模式各有一条渲染路径，
 * 这份文案与交互必须一致（两处漂移过一次的东西，通常还会漂第二次）。
 * 单位是**步**（tool_call），与头部「N 调用」同一口径。
 */
function HistoryFoldBar({
  stepCount,
  expanded,
  onToggle,
}: {
  /** 折叠态 = 当前被收起的步数；展开态 = 点「收起」会收起的步数（回到默认档） */
  stepCount: number
  expanded: boolean
  onToggle: () => void
}) {
  return (
    <button
      type="button"
      className={`trace-history-fold ${expanded ? 'is-expanded' : ''}`}
      onClick={onToggle}
    >
      {expanded ? `收起更早的 ${stepCount} 步` : `已折叠更早的 ${stepCount} 步 · 上滚展开`}
    </button>
  )
}

export function ExecutionTraceFloating({
  timeline,
  traceOverride,
  stepIndex,
  progress,
  isProcessing,
  completed,
  expandedCalls,
  onToggleExpand,
  goal,
  totalDurationMs,
  totalCalls,
  liveCalls,
  turnMeta,
  onRate,
  onRegenerate,
  visible,
  onClose,
  mode,
}: ExecutionTraceProps) {
  const [isOpen, setIsOpen] = useState(false)
  const [showRating, setShowRating] = useState(false)
  const [hasRated, setHasRated] = useState(false)
  const [terminalMode, setTerminalMode] = useState(false)
  // Terminal mode: thinking default expanded (collapsed = user manually collapsed)
  const [collapsedThinking, setCollapsedThinking] = useState<Set<string>>(new Set())
  // Card/UI mode: thinking default collapsed (expanded = user manually expanded)
  const [expandedThinking, setExpandedThinking] = useState<Set<string>>(new Set())
  // 历史折叠档位：渲染多少个「最新步骤」（见 RECENT_STEPS_KEPT）。上滚到折叠条自动 +1 档，
  // 点折叠条亦可切换（展开态点「收起」回默认档）。新一轮开始自动收回默认档。
  const [visibleStepCount, setVisibleStepCount] = useState(RECENT_STEPS_KEPT)
  /** 展开/收起的滚动锚点：变更前记下容器 scrollHeight，layoutEffect 里按差值修正 scrollTop */
  const foldAnchorHeightRef = useRef<number | null>(null)
  /** 已贴顶过一次的折叠条失去触发资格，直到用户再次上滚才复位（防补偿后程序滚动连环触发） */
  const foldArmedRef = useRef(false)
  /** 折叠触发用的上一次 scrollTop：用来判定「用户确实在上滚」 */
  const foldLastTopRef = useRef<number | null>(null)
  // Track render count per output line, for new-line animation (terminal mode)
  const lineRenderCountRef = useRef<Map<string, number>>(new Map())

  // Controlled mode: visible has highest priority
  const isVisible = visible !== undefined ? visible : isOpen

  // 气泡执行回溯：traceOverride 非空时展示该轮历史执行过程（替代全局 timeline）
  const displayTimeline = traceOverride ?? timeline

  // ── 历史折叠（按**步骤**切：1 步 = 1 条 tool_call）──
  // 默认只渲染最新 RECENT_STEPS_KEPT 步，更早的收进折叠条；上滚到折叠条自动续下一档。
  // 切点必须落在某条 tool_call 的**起点**：这是一条混合流（tool_call / thinking / text），
  // 按条目数切会把一段思考或一段文本劈成两半；按「第 N 条 tool_call 的下标」切则天然
  // 整块——切点之前的整段 thinking 随之整体收入折叠区，不劈半。
  const totalSteps = displayTimeline.reduce((n, e) => (e.kind === 'tool_call' ? n + 1 : n), 0)
  // 档位不得超过实际步数（切轮次变短时夹紧，避免留出空档）
  const visibleSteps = Math.min(visibleStepCount, totalSteps)
  const hiddenSteps = Math.max(0, totalSteps - visibleSteps)
  const foldable = hiddenSteps > 0
  // 切点：隐藏区正好是「最前面 hiddenSteps 条 tool_call」，渲染从第 hiddenSteps + 1 条
  // tool_call 的下标起 —— 可见区即恰好「最新 visibleSteps 步」。
  let cutIndex = 0
  if (foldable) {
    let seen = 0
    for (let i = 0; i < displayTimeline.length; i++) {
      if (displayTimeline[i].kind !== 'tool_call') continue
      seen++
      if (seen === hiddenSteps + 1) {
        cutIndex = i
        break
      }
    }
  }
  const visibleTimeline = foldable ? displayTimeline.slice(cutIndex) : displayTimeline
  // 折叠条步数 = 点「收起」会收起的量（回到默认档），故恒为 totalSteps − 默认档；
  // 折叠态下 visibleSteps 恒等于默认档，故这个数同时就是「当前被收起的步数」。
  const foldBarSteps = Math.max(0, totalSteps - RECENT_STEPS_KEPT)
  const foldExpanded = visibleStepCount > RECENT_STEPS_KEPT
  // 序号续数：隐藏区内的 tool_call 数即「可见区首条的序号前缀」——否则序号从 1 重来，
  // 与头部的「N 调用」计数对不上（展开/折叠一次就穿帮）。
  const leadingCallCount = hiddenSteps

  // Internal state: auto-popup (only in uncontrolled mode)
  const hasRunning = displayTimeline.some(t => t.kind === 'tool_call' && t.status === 'running')
  useEffect(() => {
    if (visible === undefined && hasRunning && !isOpen && isProcessing) {
      setIsOpen(true)
    }
  }, [hasRunning, isOpen, isProcessing, visible])

  // 新一轮 / 切到另一轮时自动收回默认档：否则上一轮展开过的档位会被原样带进下一轮，
  // 等它再长到上百步就又是全量渲染——折叠形同虚设。
  //
  // 判定用两个信号（任一命中即复位）：
  //   ① 头部条目 id 变化 —— 本轮内 timeline 只增不减、任务行只追加（见 useEvents），
  //      头部 id 本轮内稳定，故同一轮内不会误复位；
  //   ② 步数总量**变少** —— 新一轮从 0 起算 / 切看另一轮，总量回落是唯一解释。
  // 两个信号都只认「轮次换了」，不认「本轮又长了」，故续展档位在本轮内不受打扰。
  const headEntryId = displayTimeline[0]?.id ?? ''
  const roundMarkRef = useRef<{ head: string; steps: number } | null>(null)
  useEffect(() => {
    const prev = roundMarkRef.current
    roundMarkRef.current = { head: headEntryId, steps: totalSteps }
    if (prev && prev.head === headEntryId && totalSteps >= prev.steps) return
    setVisibleStepCount(RECENT_STEPS_KEPT)
    // 触发资格与方向记忆一并复位：新一轮的第一次上滚从干净状态起算
    foldArmedRef.current = false
    foldLastTopRef.current = null
  }, [headEntryId, totalSteps])

  // 贴底跟随（useStickyScroll）：跟随态只由用户手势改写 —— 面板无回底按钮，靠「上滚
  // 即解锁、停在底部才回归」保证用户在流式下读历史不被拽回（旧「15s 静默宽限 /
  // isProcessing 读秒」参数已随 hook 一并删除）。取代更早的自有滚动 state
  // （userScrolledRef + 3s debounce 强制滚底）——那套「停手 3s 即闪回底部」无论用户
  // 在读什么都会被打断。
  const { scrollRef, onScroll, followReset, enterPanel, nudgeScrollTop } =
    useStickyScroll(displayTimeline)

  // ── 历史折叠：自动续展 + 滚动锚点补偿 ──
  /**
   * 展开/收起的共同入口：先记下当前 scrollHeight 作为锚点，再改档位。
   * 新内容插在折叠条之后（即挡在原有可见内容上方），若不修正 scrollTop，视野会被整段
   * 推下去；补偿在 layoutEffect 里按高度差做，展开与点击收起共用同一出口。
   */
  const applyStepCount = useCallback(
    (next: (v: number) => number) => {
      foldAnchorHeightRef.current = scrollRef.current?.scrollHeight ?? null
      setVisibleStepCount(next)
    },
    [scrollRef],
  )

  /**
   * 折叠条点击：展开态 → 收回默认档；折叠态 → 再放一档。
   * 另有自动续展路径（上滚到折叠条），见 handleScroll —— 两者共用 applyStepCount。
   */
  const handleFoldToggle = useCallback(() => {
    applyStepCount(v => (v > RECENT_STEPS_KEPT ? RECENT_STEPS_KEPT : v + STEPS_PER_EXPAND))
  }, [applyStepCount])

  /**
   * 滚动容器 onScroll：先交既有贴底跟随/上翻冻结判定（onScroll），再做折叠续展判定。
   * 两种渲染模式共用这一个 handler，避免两条路径漂移。
   *
   * 触发条件（三与）：用户确实在上滚（delta < 0）+ 折叠条已贴到视口顶（scrollTop 落进
   * 触发带）+ 尚有更早内容。展开后锚点补偿把 scrollTop 推回带外，且 foldArmedRef 已解除，
   * 补偿产生的程序滚动不会连环触发——要再展开必须再上滚一次。
   */
  const handleScroll = useCallback(() => {
    // 先问 hook：这一帧算不算「用户操作」。进场自动下拉、贴底回滚、折叠锚点补偿
    // 都是**程序滚动**，一律不参与续展判定——否则自动下拉途中的一次中间态 delta
    // 就会被当成"用户在上滚"，续展 + 补偿反过来把自动下拉顶掉（2026-10-09 实测事故）。
    const el = scrollRef.current
    if (!el) return
    const top = el.scrollTop
    const prev = foldLastTopRef.current
    // 位置**每帧**都记（哪怕是程序滚动造成的）：方向判定必须拿真实相邻两帧作比较，
    // 否则第一次用户上滚会因"没有前��帧"而永远无法武装（2026-10-09 实测事故）。
    foldLastTopRef.current = top
    // 武装只认「用户帧」：进场自动下拉 / 贴底回滚 / 锚点补偿都是程序滚动。
    if (onScroll() && prev != null && top < prev - FOLD_SCROLL_DELTA_EPS_PX) {
      foldArmedRef.current = true
    }
    if (!foldArmedRef.current || !foldable) return
    if (top > FOLD_TRIGGER_BAND_PX) return
    foldArmedRef.current = false
    applyStepCount(v => v + STEPS_PER_EXPAND)
  }, [onScroll, scrollRef, foldable, applyStepCount])

  /**
   * 滚动锚点补偿：档位变更与 DOM 在同一提交里落定，layoutEffect 同步按新增/减少的高度
   * 修正 scrollTop —— 视野停在原处不跳。溢出/收缩都按同一差值修正（浏览器自行夹紧边界）。
   */
  useLayoutEffect(() => {
    const anchor = foldAnchorHeightRef.current
    if (anchor == null) return
    // 无条件消费锚点：即便此刻容器已不在（模式切换同帧卸挂），也不让旧高度跨提交残留
    foldAnchorHeightRef.current = null
    const el = scrollRef.current
    if (!el) return
    const delta = el.scrollHeight - anchor
    // 走 hook 的程序滚动出口：裸改 scrollTop 会被读成"用户在回底部" → 上翻冻结被解
    if (delta !== 0) nudgeScrollTop(delta)
  }, [visibleStepCount, scrollRef, nudgeScrollTop])

  // 进场防误判：面板打开瞬间 enterPanel —— 先立即滚底展示最新执行态，随后只等这次
  // 自动滚底完成（到底 / 用户伸手接手 / 安全网到点）就交还判定权；没有固定时长宽限
  // （旧实现 3s 硬窗会吞掉窗口内用户的滚动，等于抢控制，详见 useStickyScroll 头注）。
  // open 来源：受控 visible（App.tsx 的 showExecTrace）优先，未传时回落内部 isOpen。
  useEffect(() => {
    if (!isVisible) return
    // 重新进场（含"关闭后再打开"）一律收回默认档：面板只是隐藏时组件并不卸载，
    // 档位会原样留着，用户看到的是上次的展开态而不是"最近 30 步"（2026-10-09 大王报障）。
    setVisibleStepCount(RECENT_STEPS_KEPT)
    foldArmedRef.current = false
    foldLastTopRef.current = null
    foldAnchorHeightRef.current = null
    enterPanel()
  }, [isVisible, enterPanel])

  // Auto-scroll to bottom when switching terminal/card mode. 必须走 followReset 出口
  // （而非裸改 scrollTop）：裸改不恢复跟随态、也不装甲程序回响对冲，其 scroll 事件
  // 会被当成用户操作建立在错误的前提上被判定。
  useEffect(() => {
    const timer = setTimeout(() => {
      followReset()
    }, 50)
    return () => clearTimeout(timer)
  }, [terminalMode, followReset])

  // Reset rating state
  useEffect(() => {
    if (isProcessing) {
      setHasRated(false)
      setShowRating(false)
    }
  }, [isProcessing])

  const toolCalls = displayTimeline.filter(t => t.kind === 'tool_call')
  const isHistoryView = traceOverride != null && traceOverride.length > 0
  // ── 步数 / 耗时：唯一出口 resolveTurnCalls / resolveTurnDuration，无兜底 ──
  //   · 实时轮 → turnMeta（后端权威：execution_progress / execution_completed 写入）
  //   · 气泡回溯 → 该轮 traceOverride 的 tool_call 条目数（这一轮的真实记录；
  //     此时全局 turnMeta 属「当前轮」，与所看历史轮无关）
  const displayCalls = isHistoryView ? toolCalls.length : resolveTurnCalls(turnMeta)
  const failCount = toolCalls.filter(t => t.status === 'error').length

  // ── Dynamic title state from last timeline entry ──
  const titleState = useMemo(() => {
    if (completed) {
      return { avatar: failCount > 0 ? ('error' as const) : ('success' as const), text: '执行完成' }
    }
    if (!isProcessing || displayTimeline.length === 0) {
      return { avatar: 'idle' as const, text: '执行追踪' }
    }
    const last = displayTimeline[displayTimeline.length - 1]
    switch (last.kind) {
      case 'thinking':
        return { avatar: 'thinking' as const, text: '思考中…' }
      case 'text':
        return { avatar: 'streaming' as const, text: '输出中…' }
      case 'tool_call':
        return { avatar: 'working' as const, text: `执行 ${last.toolName || '工具'}…` }
      case 'task':
        return { avatar: 'working' as const, text: '派发任务…' }
      default:
        return { avatar: 'working' as const, text: '执行中…' }
    }
  }, [completed, failCount, isProcessing, displayTimeline])
  const progressPct = completed
    ? 100
    : progress.max > 0
      ? Math.min((progress.iteration / progress.max) * 100, 100)
      : toolCalls.filter(t => t.status === 'success').length > 0
        ? 45
        : 0

  const handleRate = useCallback(
    (name: string, rating: number, comment: string, saveAsStrategy: boolean) => {
      setHasRated(true)
      onRate?.(name, rating, comment, saveAsStrategy)
    },
    [onRate],
  )

  const handleClose = useCallback(() => {
    if (onClose) {
      onClose()
    } else {
      setIsOpen(false)
    }
  }, [onClose])

  // ── 实时计时：turn 开始（execution_started）即起跑，与工具调用无关 ──
  // 数值唯一来源 resolveTurnDuration：
  //   执行中 → Date.now() - turnMeta.startedAtMs（后端绝对起点，刷新后经轮询通道补回）
  //   已结束 → turnMeta.durationMs（后端权威总耗时）
  // 这里只做 100ms 重渲染让数字走秒，**不持有任何时间值、不起跑、不兜底**。
  const [, forceTick] = useState(0)
  useEffect(() => {
    if (!isProcessing) return
    const timer = window.setInterval(() => forceTick(n => n + 1), 100)
    return () => window.clearInterval(timer)
  }, [isProcessing])

  const displayDuration = resolveTurnDuration(turnMeta)

  // ── New entry entrance marker ──
  const prevTimelineIdsRef = useRef(new Set<string>())
  const [newEntryIds, setNewEntryIds] = useState(new Set<string>())
  useEffect(() => {
    const currentIds = new Set(displayTimeline.map(e => e.id))
    const added = new Set<string>()
    for (const id of currentIds) {
      if (!prevTimelineIdsRef.current.has(id)) added.add(id)
    }
    if (added.size > 0) {
      setNewEntryIds(added)
      // Auto-remove marker after animation plays
      const timer = setTimeout(() => setNewEntryIds(new Set()), 3000)
      prevTimelineIdsRef.current = currentIds
      return () => clearTimeout(timer)
    }
    prevTimelineIdsRef.current = currentIds
  }, [displayTimeline])

  // 折叠条节点在这里生成一次、两条渲染路径共用：终端模式与卡片模式的文案、交互、
  // 续展逻辑不可能漂移（两处各写一遍的东西，通常还会漂第二次）。
  const foldBarNode = foldable ? (
    <HistoryFoldBar stepCount={foldBarSteps} expanded={foldExpanded} onToggle={handleFoldToggle} />
  ) : null

  if (!isVisible) return null

  return (
    <div className="execution-trace-overlay" onClick={handleClose}>
      <div
        className={`execution-trace-center ${terminalMode ? 'terminal-mode' : ''} ${completed ? 'completed' : ''}`}
        onClick={e => e.stopPropagation()}
      >
        {/* ── Header ── */}
        <div className="execution-trace-header">
          <div className="execution-trace-title">
            <NuphusAvatar state={titleState.avatar} size={20} />
            <span>{titleState.text}</span>
            <span className="exec-topbar-meta">
              {displayCalls} 调用 · {formatMs(displayDuration)}
              {failCount > 0 && <span className="trace-fail-count"> · {failCount} 失败</span>}
            </span>
          </div>
          <div className="execution-trace-controls">
            <button
              className={`trace-terminal-toggle ${terminalMode ? 'active' : ''}`}
              onClick={() => setTerminalMode(v => !v)}
              title={terminalMode ? '切换回卡片模式' : '终端模式'}
            >
              <IconTerminal size={13} />
              <span>{terminalMode ? 'TERM' : 'UI'}</span>
            </button>
            <button
              className="trace-close"
              onClick={handleClose}
              aria-label={completed ? 'Close' : 'Close'}
            >
              <IconX size={14} />
            </button>
          </div>
        </div>

        {/* ── Progress Bar ── */}
        {isProcessing && (
          <div className="trace-progress-bar">
            <div
              className={`trace-progress-fill ${completed ? 'done' : 'running'}`}
              style={{ width: `${progressPct}%` }}
            />
          </div>
        )}

        {/* ── Timeline Body ── */}
        {terminalMode ? (
          <div className="execution-terminal-body" ref={scrollRef} onScroll={handleScroll}>
            {displayTimeline.length === 0 && isProcessing && (
              <div className="execution-trace-placeholder">等待执行...</div>
            )}
            <div className="execution-terminal-lines">
              {foldBarNode}
              {(() => {
                let callIdx = leadingCallCount
                return visibleTimeline.map((entry, i) => {
                  if (entry.kind === 'tool_call') {
                    callIdx++
                    const isExpanded = expandedCalls.has(entry.id)
                    const p = entry.params as Record<string, unknown> | undefined
                    const outputStr = entry.output || ''
                    const duration = entry.durationMs ? formatMs(entry.durationMs) : ''

                    // Build args string — extract key fields for cleaner display
                    const argStr = (() => {
                      if (!p) return ''
                      // task_dispatch: compact label, full text in expanded view
                      if (entry.toolName === 'task_dispatch') {
                        const tid = (p as { task_id?: number }).task_id || 1
                        const ttl = (p as { total_tasks?: number }).total_tasks || 1
                        const desc = ((p as { description?: string }).description || '')
                          .replace(/\n/g, ' ')
                          .slice(0, 80)
                        return `#${tid}/${ttl} ${desc}${desc.length >= 80 ? '…' : ''}`
                      }
                      return (p as { command?: string })?.command
                        ? `${(p as { command: string }).command}`
                        : (p as { path?: string })?.path
                          ? `${(p as { path: string }).path}`
                          : (p as { description?: string })?.description
                            ? `${(p as { description: string }).description}`
                            : (p as { id?: string })?.id
                              ? `id: ${(p as { id: string }).id}`
                              : JSON.stringify(p).slice(0, 160)
                    })()

                    const lines: JSX.Element[] = []

                    // ── Command line ──
                    const agentTag = entry.fromTask ? (
                      <span className="tc-task-badge" style={{ marginRight: 5 }}>
                        TASK
                      </span>
                    ) : (
                      <span className="exec-agent-tag">LEADER</span>
                    )
                    const cat = getToolCategory(entry.toolName || '')
                    lines.push(
                      <div
                        key="cmd"
                        className={`term-line term-cmd term-cat-${cat} ${entry.fromTask ? 'from-task' : ''}`}
                      >
                        <span className="term-prompt">{entry.fromTask ? '▸' : '$'}</span>
                        {agentTag}
                        <span className="term-toolname">{entry.toolName}</span>
                        {argStr && <span className="term-args">{argStr}</span>}
                        {duration && <span className="term-duration">{duration}</span>}
                      </div>,
                    )

                    // ── Output: categorized by tool type (read-only → skip; write → full; exec → 50 lines; default → 10 lines) ──
                    const outLines = entry.outputLines || (outputStr ? outputStr.split('\n') : [])
                    let showLines = 0
                    let hasMore = false
                    if (cat === 'read') {
                      // Read-only tools: no output body, command line is sufficient
                    } else if (cat === 'write') {
                      // Write tools: full diff/content, no line limit
                      if (entry.toolName === 'Edit') {
                        const oldStr = (p?.old_string as string) || ''
                        const newStr = (p?.new_string as string) || ''
                        const diffText = computeDiff(oldStr, newStr)
                        if (diffText) {
                          lines.push(
                            <div key="diff-block" className="term-diff-block">
                              <div className="tc-diff">{diffText}</div>
                            </div>,
                          )
                        }
                      } else if (outputStr.length > 0) {
                        lines.push(
                          <div key="output" className="term-out-block">
                            <MarkdownContent content={outputStr} />
                          </div>,
                        )
                      }
                    } else if (outputStr.length > 0) {
                      const maxLines = cat === 'exec' ? 50 : 10
                      showLines = Math.min(outLines.length, maxLines)
                      hasMore = outLines.length > showLines
                      const shown = hasMore ? outLines.slice(0, showLines) : outLines
                      const previewText = shown.join('\n')
                      lines.push(
                        <div key="output" className="term-out-block">
                          <MarkdownContent content={previewText} />
                        </div>,
                      )
                      if (hasMore && !isExpanded) {
                        lines.push(
                          <div key="more" className="term-line">
                            <span className="term-gutter"> </span>
                            <span className="term-trunc">
                              … {outLines.length - showLines} more lines (click to expand)
                            </span>
                          </div>,
                        )
                      }
                    }
                    if (entry.isTruncated && entry.outputFullSize && !isExpanded) {
                      lines.push(
                        <div key="cut" className="term-line">
                          <span className="term-gutter"> </span>
                          <span className="term-trunc">
                            … output truncated ({entry.outputFullSize} chars, click to expand)
                          </span>
                        </div>,
                      )
                    }

                    // ── Status line ──
                    const isOk = entry.status === 'success'
                    const isErr = entry.status === 'error'
                    const isRunning = entry.status === 'running'
                    const desktopResult = isRunning
                      ? null
                      : desktopActionResult(entry.toolName, entry.output)
                    lines.push(
                      <div
                        key="status"
                        className={`term-status ${desktopResult ? '' : isOk ? 'term-ok' : isErr ? 'term-err' : isRunning ? 'term-running' : ''}`}
                      >
                        {desktopResult ? (
                          <DesktopActionStatus result={desktopResult} />
                        ) : (
                          <>
                            <span className={`term-status-icon ${isRunning ? 'running' : ''}`} />
                            <span
                              className={`term-exit-code ${isRunning ? 'term-running-text' : ''}`}
                            >
                              {isOk
                                ? 'exit 0'
                                : isErr
                                  ? 'exit 1'
                                  : isRunning
                                    ? 'running…'
                                    : 'pending'}
                            </span>
                          </>
                        )}
                      </div>,
                    )

                    // ── Expanded area: full output + parameter details ──
                    if (isExpanded) {
                      if (entry.toolName === 'task_dispatch') {
                        // task_dispatch: show full description formatted + output result
                        const p = entry.params as Record<string, unknown> | undefined
                        const description = (p?.description as string) || ''
                        let summary = outputStr
                        try {
                          const parsed = JSON.parse(outputStr)
                          if (parsed.summary) summary = parsed.summary
                        } catch {}

                        lines.push(
                          <div key="expanded" className="term-expanded">
                            {description && (
                              <div
                                className="tc-code-block tc-task-summary"
                                style={{
                                  whiteSpace: 'pre-wrap',
                                  fontSize: 'var(--fs-caption)',
                                  lineHeight: 1.6,
                                  padding: 10,
                                  marginBottom: 8,
                                }}
                              >
                                {description}
                              </div>
                            )}
                            <div className="term-expanded-label">Output</div>
                            <div
                              className="tc-code-block tc-task-summary"
                              style={{
                                whiteSpace: 'pre-wrap',
                                fontSize: 'var(--fs-caption)',
                                lineHeight: 1.6,
                                padding: 10,
                              }}
                            >
                              <MarkdownContent content={summary} />
                            </div>
                          </div>,
                        )
                      } else {
                        lines.push(
                          <div key="expanded" className="term-expanded">
                            {hasMore && (
                              <>
                                <div className="term-expanded-label">Full output</div>
                                {outLines.slice(showLines).map((line, li) =>
                                  line.trim() ? (
                                    <div key={`eout-${li}`} className="term-out">
                                      <span className="term-out-marker">&gt;</span>
                                      <span className="term-out-text">{line}</span>
                                    </div>
                                  ) : null,
                                )}
                                <div className="term-expanded-sep" />
                              </>
                            )}
                            {entry.isTruncated && entry.outputFullSize && (
                              <>
                                <div className="term-expanded-label">
                                  Truncated ({entry.outputFullSize} chars)
                                </div>
                                <div className="term-expanded-sep" />
                              </>
                            )}
                            {p && Object.keys(p).length > 0 && (
                              <>
                                <div className="term-expanded-label">Params</div>
                                {Object.entries(p).map(([k, v]) => (
                                  <div key={k} className="term-line term-param">
                                    <span className="term-gutter"> </span>
                                    <span className="term-param-key">{k}</span>
                                    <span className="term-param-val">
                                      {typeof v === 'string' ? v : JSON.stringify(v)}
                                    </span>
                                  </div>
                                ))}
                              </>
                            )}
                          </div>,
                        )
                      }
                    }

                    return (
                      <div
                        key={entry.id}
                        className={`term-call ${entry.status === 'running' ? 'running' : ''}`}
                        onClick={() => onToggleExpand(entry.id)}
                      >
                        {lines}
                      </div>
                    )
                  }
                  if (entry.kind === 'task') {
                    return (
                      <div key={entry.id} className="term-line term-task">
                        <StatusIcon status={entry.status} />
                        <span className="term-task-text">{entry.text}</span>
                        {entry.summary && (
                          <span className="term-task-summary">{entry.summary}</span>
                        )}
                      </div>
                    )
                  }
                  if (entry.kind === 'thinking') {
                    const isCollapsed = collapsedThinking.has(entry.id)
                    const charCount = entry.text?.length || 0
                    return (
                      <div key={entry.id}>
                        <div
                          className="exec-terminal-text exec-terminal-thinking-summary"
                          onClick={() =>
                            setCollapsedThinking(prev => {
                              const next = new Set(prev)
                              if (isCollapsed) next.delete(entry.id)
                              else next.add(entry.id)
                              return next
                            })
                          }
                        >
                          <span className="term-thinking-caret">{isCollapsed ? '▸' : '▾'}</span>{' '}
                          Thinking ({charCount} chars)
                        </div>
                        {!isCollapsed && (
                          <div className="exec-terminal-text term-thinking-body">{entry.text}</div>
                        )}
                      </div>
                    )
                  }
                  if (entry.kind === 'text') {
                    const isExpanded = expandedCalls.has(entry.id)
                    const { tag, remaining } = parsePhaseTag(entry.text || '')
                    const displayText = isExpanded ? remaining : remaining.slice(0, 5000)
                    const hasMore = remaining.length > 5000
                    return (
                      <div
                        key={entry.id}
                        className="term-call term-call-agent"
                        onClick={() => onToggleExpand(entry.id)}
                      >
                        <div className="term-agent">
                          <span className="term-gutter"> </span>
                          {tag && (
                            <span className={`term-agent-tag ${tag?.toLowerCase() || ''}`}>
                              [{tag}]
                            </span>
                          )}
                          <span className="term-agent-text">{displayText}</span>
                        </div>
                        {hasMore && !isExpanded && (
                          <div className="term-more-line">
                            <span className="term-gutter"> </span>
                            <span className="term-more-hint">
                              … {remaining.length - 5000} more chars (click to expand)
                            </span>
                          </div>
                        )}
                      </div>
                    )
                  }
                  if (entry.kind === 'reminder') {
                    return (
                      <div key={entry.id} className="term-reminder">
                        <span className="term-reminder-marker">!</span>
                        <span>{entry.text}</span>
                      </div>
                    )
                  }
                  return null
                })
              })()}
            </div>
          </div>
        ) : (
          <div className="execution-trace-body" ref={scrollRef} onScroll={handleScroll}>
            {displayTimeline.length === 0 && isProcessing && (
              <div className="execution-trace-placeholder">等待执行...</div>
            )}

            {foldBarNode}

            {(() => {
              let callIdx = leadingCallCount
              return visibleTimeline.map((entry, i) => {
                if (entry.kind === 'tool_call') {
                  callIdx++
                  const isExpanded = expandedCalls.has(entry.id)
                  const p = entry.params as Record<string, unknown> | undefined
                  // Extract main params for command line summary
                  const path =
                    (p?.command as string) ||
                    (p?.path as string) ||
                    (p?.pattern as string) ||
                    (p?.query as string) ||
                    (p?.url as string) ||
                    (p?.text as string) ||
                    (p?.description as string) ||
                    (p?.name as string) ||
                    (typeof p === 'object' && p ? JSON.stringify(p).slice(0, 120) : '')
                  // Cleanup: remove extra quotes
                  const displayPath = path.replace(/^["']|["']$/g, '')
                  const desktopResult =
                    entry.status === 'running'
                      ? null
                      : desktopActionResult(entry.toolName, entry.output)

                  return (
                    <Fragment key={entry.id}>
                      <div
                        className={`tc-option ${entry.status || 'pending'} ${entry.fromTask ? 'from-task' : ''} ${isExpanded ? 'is-expanded' : ''} ${newEntryIds.has(entry.id) ? 'trace-new' : ''}`}
                        onClick={() => onToggleExpand(entry.id)}
                      >
                        <span className="tc-option-indicator">{callIdx}</span>
                        <div className="tc-option-text">
                          <div className="tc-option-main">
                            {entry.fromTask && <span className="tc-task-badge">task</span>}
                            {entry.toolName}
                          </div>
                          {path && (
                            <div className="tc-option-desc">
                              <code>{path}</code>
                            </div>
                          )}
                        </div>
                        {desktopResult ? (
                          <DesktopActionStatus result={desktopResult} />
                        ) : (
                          <StatusIcon status={entry.status} />
                        )}
                        {!desktopResult && entry.status && (
                          <span className={`tc-status-chip ${entry.status}`}>
                            {toolStatusLabel(entry.status)}
                          </span>
                        )}
                        <span className="tc-duration">
                          {entry.durationMs ? formatMs(entry.durationMs) : ''}
                        </span>
                        <span className="tc-chevron">
                          {isExpanded ? <IconChevronUp size={12} /> : <IconChevronDown size={12} />}
                        </span>
                      </div>
                      {isExpanded && <ToolDetail entry={entry} />}
                      {entry.status === 'running' && (
                        <div className="tc-progress">
                          <div className="tc-progress-fill" />
                        </div>
                      )}
                    </Fragment>
                  )
                }

                if (entry.kind === 'thinking') {
                  const isExpanded = expandedThinking.has(entry.id)
                  const charCount = entry.text?.length || 0
                  return (
                    <div
                      key={entry.id}
                      className={`${newEntryIds.has(entry.id) ? 'trace-new' : ''}`}
                    >
                      <div
                        className="exec-terminal-text exec-terminal-thinking-summary"
                        onClick={() =>
                          setExpandedThinking(prev => {
                            const next = new Set(prev)
                            if (isExpanded) next.delete(entry.id)
                            else next.add(entry.id)
                            return next
                          })
                        }
                      >
                        <span className="term-thinking-caret">{isExpanded ? '▾' : '▸'}</span>{' '}
                        Thinking ({charCount} chars)
                      </div>
                      {isExpanded && (
                        <div className="exec-terminal-text term-thinking-body">{entry.text}</div>
                      )}
                    </div>
                  )
                }
                if (entry.kind === 'text') {
                  const { tag, remaining } = parsePhaseTag(entry.text || '')
                  const color = tag ? phaseTagColor(tag) : null
                  return (
                    <div
                      key={entry.id}
                      className={`agent-msg-node ${newEntryIds.has(entry.id) ? 'trace-new' : ''}`}
                    >
                      <span className="agent-msg-dot" />
                      <div className="agent-msg-text">
                        {tag && color && (
                          <span className={`tc-task-badge tc-badge-${tag.toLowerCase()}`}>
                            {tag}
                          </span>
                        )}
                        <MarkdownContent content={remaining} />
                      </div>
                    </div>
                  )
                }
                if (entry.kind === 'reminder') {
                  return (
                    <div
                      key={entry.id}
                      className={`agent-msg-node reminder ${newEntryIds.has(entry.id) ? 'trace-new' : ''}`}
                    >
                      <span className="agent-msg-dot" />
                      <div className="agent-msg-text">
                        <MarkdownContent content={entry.text || ''} />
                      </div>
                    </div>
                  )
                }
                return null
              })
            })()}

            {/* ── Memories ── (removed: moved to StatusBar hover) */}
          </div>
        )}

        {/* ── Footer Actions ── */}
        <div className="execution-trace-footer">
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {!hasRated && onRate && (
              <>
                <button className="exec-footer-btn primary" onClick={() => setShowRating(true)}>
                  <IconStar size={14} />
                  <span>点评</span>
                </button>
                {onRegenerate && (
                  <button className="exec-footer-btn" onClick={onRegenerate}>
                    <IconRefresh size={14} />
                    <span>重新生成</span>
                  </button>
                )}
              </>
            )}
            {hasRated && <span className="trace-rated-label">★ 已点评</span>}
          </div>
        </div>

        {/* ── Rating Modal ── */}
        {showRating && (
          <RatingModal
            goal={goal || ''}
            toolCalls={displayTimeline}
            totalMs={displayDuration || 0}
            onClose={() => setShowRating(false)}
            onSubmit={handleRate}
          />
        )}
      </div>
    </div>
  )
}
