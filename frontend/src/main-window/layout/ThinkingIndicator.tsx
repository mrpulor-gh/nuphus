import type { TimelineEntry } from '../../core/types'
import { MoodFace } from '../../ui/MoodFace'
import type { MoodState } from '../../ui/MoodFace'
import { IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'

interface ThinkingIndicatorProps {
  step: string
  isThinking: boolean
  completed?: boolean
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

export function ThinkingIndicator({
  step,
  isThinking,
  completed,
  phase,
  timeline,
  mood,
  progress,
  onExpand,
  onClose,
}: ThinkingIndicatorProps) {
  const { t } = useLanguage()
  if (!completed && !isThinking && !step) return null

  const phaseColor = completed ? '#22c55e' : phase ? PHASE_COLORS[phase] : '#3b82f6'

  const callCount = timeline ? timeline.filter(t => t.kind === 'tool_call').length : 0

  return (
    <div
      className={`thinking-indicator ${isThinking ? 'breathing' : completed ? 'completed' : ''}${mood && mood !== 'idle' ? ` mood-${mood}` : ''}`}
      style={{
        boxShadow: `0 0 24px ${phaseColor}08, var(--shadow-elevated)`,
      }}
    >
      <div className="thinking-indicator-inner" onClick={() => onExpand?.()}>
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
              {completed ? t('thinking.completed') : step || t('thinking.inProgress')}
            </span>
          </div>
        </div>

        <div className="thinking-actions" onClick={e => e.stopPropagation()}>
          <IconButton
            variant="ghost"
            label={t('thinking.viewDetails')}
            onClick={() => onExpand?.()}
          >
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