import type { TimelineEntry } from '../../core/types'
import { MoodFace } from '../../ui/MoodFace'
import type { MoodState } from '../../ui/MoodFace'
import { IconButton } from '../../ui/Button'
import { playUiSound } from '../../ui/sound'
import { useLanguage } from '../../locales'

interface ThinkingIndicatorProps {
  step: string
  isThinking: boolean
  completed?: boolean
  dismissed?: boolean
  phase?: 'understanding' | 'executing' | 'recording' | 'workflow' | 'retrying' | ''
  timeline?: TimelineEntry[]
  mood?: MoodState
  progress?: { iteration: number; max: number; calls: number }
  onExpand?: () => void
  onClose?: () => void
}

const PHASE_LABELS: Record<string, string> = {
  understanding: 'thinking.understanding',
  executing: 'thinking.executing',
  recording: 'thinking.recording',
  workflow: 'thinking.workflow',
  retrying: 'thinking.retrying',
}

const PHASE_COLORS: Record<string, string> = {
  understanding: '#3b82f6',
  executing: '#f59e0b',
  recording: '#22c55e',
  workflow: '#8b5cf6',
  retrying: '#f97316',
}

/** 最近一次工具调用条目（执行中无 agent text 时提示用；无则 null） */
function lastToolCall(timeline?: TimelineEntry[]): TimelineEntry | null {
  if (!timeline) return null
  for (let i = timeline.length - 1; i >= 0; i--) {
    const e = timeline[i]
    if (e.kind === 'tool_call' && e.toolName) return e
  }
  return null
}

/** 路径/URL 类参数的键（值需形如路径才采用） */
const TARGET_PATH_KEYS = [
  'path',
  'file_path',
  'original_path',
  'modified_path',
  'template_path',
  'image_path',
  'from',
  'to',
  'url',
  'source',
  'destination',
  'dir',
  'folder',
]

/** 无路径时回退的意图类参数键（command/query 等） */
const TARGET_TEXT_KEYS = ['command', 'query', 'title', 'id', 'keywords', 'desc', 'prompt']

function looksPathLike(v: string): boolean {
  if (v.length < 2 || v.length > 260 || v.includes('\n')) return false
  if (/^[a-zA-Z]:[\\/]/.test(v)) return true // C:\...
  if (/^https?:\/\//i.test(v)) return true // URL
  if (v.startsWith('/') || v.startsWith('~') || v.startsWith('./') || v.startsWith('../')) return true
  return v.includes('/') || v.includes('\\') // 相对路径/嵌套路径
}

function compact(s: string, max = 120): string {
  const t = s.trim().replace(/\s+/g, ' ')
  return t.length > max ? t.slice(0, max) + '…' : t
}

/** 从工具调用参数中提取「工具名 + 目标」的紧凑标签，如 Read frontend/src/hooks/useSession.ts */
function toolCallLabel(toolName: string, params: unknown): string | null {
  if (!params || typeof params !== 'object' || Array.isArray(params)) return null
  const p = params as Record<string, unknown>
  // 1) 优先取路径/URL 参数（相对路径如 frontend/src/... 也命中）
  for (const key of TARGET_PATH_KEYS) {
    const v = p[key]
    if (typeof v === 'string' && looksPathLike(v)) {
      const t = compact(v)
      return key === 'id' ? `${toolName} id: ${t}` : `${toolName} ${t}`
    }
  }
  // 2) 无路径时取命令/查询类意图参数
  for (const key of TARGET_TEXT_KEYS) {
    const v = p[key]
    if (typeof v === 'string' && v.trim()) {
      const t = compact(v)
      return key === 'id' ? `${toolName} id: ${t}` : `${toolName} ${t}`
    }
  }
  return null
}

export function ThinkingIndicator({
  step,
  isThinking,
  completed,
  dismissed,
  phase,
  timeline,
  mood,
  progress,
  onExpand,
  onClose,
}: ThinkingIndicatorProps) {
  const { t } = useLanguage()
  // 已点击「关闭」：无论是否 completed / isThinking，整条指示器都应隐藏。
  if (dismissed) return null
  if (!completed && !isThinking && !step) return null

  const phaseColor = completed ? '#22c55e' : phase ? PHASE_COLORS[phase] : '#3b82f6'

  const callCount = timeline ? timeline.filter(t => t.kind === 'tool_call').length : 0
  const lastCall = lastToolCall(timeline)
  const toolName = lastCall?.toolName || null
  // 优先展示「工具名 + 目标」标签（如 Read frontend/src/hooks/useSession.ts）；
  // 无目标参数时退回「正在调用 {toolName}」
  const toolLabel = lastCall ? toolCallLabel(lastCall.toolName!, lastCall.params) : null

  // 点开执行详情窗口：播放切换音（展开动作反馈）
  const handleExpand = () => {
    playUiSound('switch')
    onExpand?.()
  }

  return (
    <div
      className={`thinking-indicator ${isThinking ? 'breathing' : completed ? 'completed' : ''}${mood && mood !== 'idle' ? ` mood-${mood}` : ''}`}
      style={{
        boxShadow: `0 0 24px ${phaseColor}08, var(--shadow-elevated)`,
      }}
    >
      <div className="thinking-indicator-inner" onClick={handleExpand}>
        <div className="thinking-mood-box">
          <MoodFace mood={mood || 'idle'} size={36} />
        </div>

        <div className="thinking-body">
          <div className="thinking-body-top">
            {phase && !completed && (
              <span className="thinking-phase" style={{ color: phaseColor }}>
                {t(PHASE_LABELS[phase] || phase)}
              </span>
            )}
            {callCount > 0 && (
              <span className="thinking-call-badge" style={{ color: phaseColor }}>
                {t('thinking.steps', String(callCount))}
              </span>
            )}
          </div>
          <div className="thinking-text-wrap">
            <span className="thinking-text">
              {completed
                ? t('thinking.completed')
                : step || toolLabel || (toolName ? t('thinking.toolCall', toolName) : t('thinking.inProgress'))}
            </span>
          </div>
        </div>

        <div className="thinking-actions" onClick={e => e.stopPropagation()}>
          <IconButton variant="ghost" label={t('thinking.viewDetails')} onClick={handleExpand}>
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="9 18 15 12 9 6" />
            </svg>
          </IconButton>
          {completed && onClose && (
            <IconButton variant="ghost" label={t('thinking.close')} onClick={onClose}>
              <svg
                width="11"
                height="11"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
              >
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </IconButton>
          )}
        </div>
      </div>
    </div>
  )
}