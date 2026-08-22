// ExecutionTraceFloating.tsx — Central execution trace panel
// Desktop app style: center floating, not web popup, with material depth

import { useState, useRef, useEffect, useCallback, useMemo, Fragment } from 'react'
import { IconX, IconTerminal } from '../../ui/Icons'
import { NuphusAvatar } from '../../ui/NuphusAvatar'
import MarkdownContent from '../chat/MarkdownContent'
import type { TimelineEntry } from '../../core/types'

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
    const description = (p?.description as string) || ''
    const goalType = (p?.goal_type as string) || ''
    const planPath = (p?.plan_path as string) || ''
    const taskId = (p?.task_id as number) || 1
    const totalTasks = (p?.total_tasks as number) || 1

    // Output is JSON { status, task, summary } — extract summary for cleaner display
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
        {(goalType || planPath || totalTasks > 1) && (
          <div className="tc-task-meta">
            {goalType && <span className="tc-meta-tag">{goalType}</span>}
            {planPath && <span className="tc-meta-tag tc-meta-path">{planPath}</span>}
            {totalTasks > 1 && (
              <span className="tc-meta-progress">
                步骤 {taskId}/{totalTasks}
              </span>
            )}
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
            <svg
              width="11"
              height="11"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              style={{ marginRight: 5, opacity: 0.5, flexShrink: 0 }}
            >
              <circle cx="10.5" cy="10.5" r="7.5" />
              <line x1="16" y1="16" x2="22" y2="22" />
            </svg>
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
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
      ) : status === 'running' ? (
        <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
          <polygon points="5 3 19 12 5 21 5 3" />
        </svg>
      ) : status === 'error' ? (
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinecap="round"
        >
          <line x1="18" y1="6" x2="6" y2="18" />
          <line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      ) : (
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
        >
          <circle cx="12" cy="12" r="10" />
        </svg>
      )}
    </span>
  )
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
  const scrollRef = useRef<HTMLDivElement>(null)
  const userScrolledRef = useRef(false)
  const scrollDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // Track each tool_call entry's output length, detect streaming append
  const outputSizesRef = useRef<Map<string, number>>(new Map())
  // Track each thinking entry's text length, detect streaming reasoning
  const thinkingSizesRef = useRef<Map<string, number>>(new Map())
  // Track render count per output line, for new-line animation (terminal mode)
  const lineRenderCountRef = useRef<Map<string, number>>(new Map())

  // Controlled mode: visible has highest priority
  const isVisible = visible !== undefined ? visible : isOpen

  // 气泡执行回溯：traceOverride 非空时展示该轮历史执行过程（替代全局 timeline）
  const displayTimeline = traceOverride ?? timeline

  // Internal state: auto-popup (only in uncontrolled mode)
  const hasRunning = displayTimeline.some(t => t.kind === 'tool_call' && t.status === 'running')
  useEffect(() => {
    if (visible === undefined && hasRunning && !isOpen && isProcessing) {
      setIsOpen(true)
    }
  }, [hasRunning, isOpen, isProcessing, visible])

  // Scroll anti-hijack: auto-resume follow when user scrolls to bottom, lock for 3s when scrolling up
  const resetAutoScroll = useCallback(() => {
    userScrolledRef.current = false
  }, [])

  const handleScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    // Check if scrolled to bottom → auto-resume follow, no manual trigger needed
    const isAtBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 2
    if (isAtBottom && userScrolledRef.current) {
      userScrolledRef.current = false
      if (scrollDebounceRef.current) {
        clearTimeout(scrollDebounceRef.current)
        scrollDebounceRef.current = null
      }
    }
  }, [])

  // Only lock auto-scroll on active wheel/trackpad scrolling, not on hover/layout shifts
  const handleWheel = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    const isAtBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 2
    if (!isAtBottom) {
      if (scrollDebounceRef.current) clearTimeout(scrollDebounceRef.current)
      userScrolledRef.current = true
      scrollDebounceRef.current = setTimeout(resetAutoScroll, 3000)
    }
  }, [resetAutoScroll])

  // Auto-scroll to bottom when new items added or existing content grows (streaming output/thinking)
  const prevTimelineLen = useRef(0)
  useEffect(() => {
    const el = scrollRef.current
    if (!el) return

    const len = displayTimeline.length
    let hasNewContent = len > prevTimelineLen.current

    // Check if existing tool_call's output grew (streaming append)
    if (!hasNewContent) {
      for (const tc of displayTimeline) {
        if (tc.kind === 'tool_call') {
          const prevSize = outputSizesRef.current.get(tc.id) ?? 0
          const currSize = tc.output?.length ?? 0
          if (currSize > prevSize) {
            hasNewContent = true
            break
          }
        }
      }
    }

    // Check if existing thinking entry's text grew (streaming reasoning)
    if (!hasNewContent) {
      for (const tc of displayTimeline) {
        if (tc.kind === 'thinking') {
          const prevSize = thinkingSizesRef.current.get(tc.id) ?? 0
          const currSize = tc.text?.length ?? 0
          if (currSize > prevSize) {
            hasNewContent = true
            break
          }
        }
      }
    }

    // 更新追踪记录
    for (const tc of displayTimeline) {
      if (tc.kind === 'tool_call') {
        outputSizesRef.current.set(tc.id, tc.output?.length ?? 0)
      }
      if (tc.kind === 'thinking') {
        thinkingSizesRef.current.set(tc.id, tc.text?.length ?? 0)
      }
    }
    prevTimelineLen.current = len

    if (hasNewContent && !userScrolledRef.current) {
      const timer = setTimeout(() => {
        el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' as ScrollBehavior })
      }, 100)
      return () => clearTimeout(timer)
    }
  }, [displayTimeline])

  // Cleanup
  useEffect(() => {
    return () => {
      if (scrollDebounceRef.current) clearTimeout(scrollDebounceRef.current)
    }
  }, [])

  // Auto-scroll to bottom when switching terminal/card mode
  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    const timer = setTimeout(() => {
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' as ScrollBehavior })
    }, 50)
    return () => clearTimeout(timer)
  }, [terminalMode])

  // Reset rating state
  useEffect(() => {
    if (isProcessing) {
      setHasRated(false)
      setShowRating(false)
    }
  }, [isProcessing])

  const toolCalls = displayTimeline.filter(t => t.kind === 'tool_call')
  const displayCalls = completed && totalCalls !== undefined ? totalCalls : toolCalls.length
  const displayDuration =
    completed && totalDurationMs !== undefined
      ? totalDurationMs
      : toolCalls.reduce((sum, t) => sum + (t.durationMs || 0), 0)
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

  // ── Live timer: shows elapsed time in header during execution ──
  const [liveDuration, setLiveDuration] = useState(0)
  const startTimeRef = useRef<number | null>(null)
  useEffect(() => {
    if (isProcessing && !completed) {
      if (startTimeRef.current === null) startTimeRef.current = Date.now()
      const timer = setInterval(() => {
        setLiveDuration(Date.now() - startTimeRef.current!)
      }, 100)
      return () => clearInterval(timer)
    } else if (completed && totalDurationMs !== undefined) {
      setLiveDuration(totalDurationMs)
      startTimeRef.current = null
    } else {
      setLiveDuration(0)
      startTimeRef.current = null
    }
  }, [isProcessing, completed, totalDurationMs])

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
              {displayCalls} 调用 ·{' '}
              {formatMs(isProcessing && !completed ? liveDuration : displayDuration)}
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
          <div
            className="execution-terminal-body"
            ref={scrollRef}
            onScroll={handleScroll}
            onWheel={handleWheel}
          >
            {displayTimeline.length === 0 && isProcessing && (
              <div className="execution-trace-placeholder">等待执行...</div>
            )}
            <div className="execution-terminal-lines">
              {(() => {
                let callIdx = 0
                return displayTimeline.map((entry, i) => {
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
                    lines.push(
                      <div
                        key="status"
                        className={`term-status ${isOk ? 'term-ok' : isErr ? 'term-err' : isRunning ? 'term-running' : ''}`}
                      >
                        <span className={`term-status-icon ${isRunning ? 'running' : ''}`} />
                        <span className={`term-exit-code ${isRunning ? 'term-running-text' : ''}`}>
                          {isOk ? 'exit 0' : isErr ? 'exit 1' : isRunning ? 'running…' : 'pending'}
                        </span>
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
                    const progress =
                      entry.taskId != null && entry.totalTasks != null
                        ? `[${entry.taskId}/${entry.totalTasks}] `
                        : ''
                    return (
                      <div key={entry.id} className="term-line term-task">
                        <StatusIcon status={entry.status} />
                        <span className="term-task-text">
                          {progress}
                          {entry.text}
                        </span>
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
          <div
            className="execution-trace-body"
            ref={scrollRef}
            onScroll={handleScroll}
            onWheel={handleWheel}
          >
            {displayTimeline.length === 0 && isProcessing && (
              <div className="execution-trace-placeholder">等待执行...</div>
            )}

            {(() => {
              let callIdx = 0
              return displayTimeline.map((entry, i) => {
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
                        <StatusIcon status={entry.status} />
                        <span className="tc-duration">
                          {entry.durationMs ? formatMs(entry.durationMs) : ''}
                        </span>
                        <span className="tc-chevron">
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
                            {isExpanded ? (
                              <polyline points="18 15 12 9 6 15" />
                            ) : (
                              <polyline points="6 9 12 15 18 9" />
                            )}
                          </svg>
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
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
                  </svg>
                  <span>点评</span>
                </button>
                {onRegenerate && (
                  <button className="exec-footer-btn" onClick={onRegenerate}>
                    <svg
                      width="14"
                      height="14"
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
