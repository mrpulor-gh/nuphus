import { useLanguage } from '../../locales'

interface StatusBarProps {
  mainTokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  execTokenUsage?: { inputTokens: number; outputTokens: number; cacheHitTokens: number } | null
  totalDurationMs?: number
  totalCalls?: number
  mood?: string
  contextLimit?: number
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  const m = Math.floor(ms / 60000)
  const s = Math.floor((ms % 60000) / 1000)
  return `${m}m ${s}s`
}

export const MOOD_COLORS: Record<string, string> = {
  idle: '#555566',
  thinking: '#3b82f6',
  working: '#f59e0b',
  success: '#22c55e',
  error: '#ef4444',
  waiting: '#8b5cf6',
  reading: '#3b82f6',
  writing: '#06b6d4',
  searching: '#f59e0b',
  coding: '#f97316',
  analyzing: '#8b5cf6',
}

function ContextBar({ used, limit }: { used: number; limit: number }) {
  const { t } = useLanguage()
  const pct = limit > 0 ? Math.min(used / limit, 1) : 0
  const color = pct > 0.8 ? '#ef4444' : pct > 0.6 ? '#f59e0b' : '#22c55e'
  const remaining = Math.max(limit - used, 0)
  return (
    <span
      className="status-ctx-wrap"
      title={t(
        'status.contextTooltip',
        formatTokens(used),
        formatTokens(limit),
        (pct * 100).toFixed(0),
      )}
    >
      <span className="status-ctx-bar" style={{ background: `${color}33` }}>
        <span className="status-ctx-fill" style={{ width: `${pct * 100}%`, background: color }} />
      </span>
      <span className="status-ctx-label" style={{ color }}>
        {formatTokens(used)}{' '}
        <span className="status-ctx-remaining">
          {t('status.remaining', formatTokens(remaining))}
        </span>
      </span>
    </span>
  )
}

export function StatusBar({
  mainTokenUsage,
  execTokenUsage,
  totalDurationMs,
  totalCalls,
  mood,
  contextLimit,
}: StatusBarProps) {
  const { t } = useLanguage()
  const execTokens = (execTokenUsage?.inputTokens || 0) + (execTokenUsage?.outputTokens || 0)
  const ctxUsed = mainTokenUsage?.inputTokens || 0

  return (
    <div className="status-bar">
      <div className="status-left">
        <span className="status-item">
          {contextLimit ? (
            <ContextBar used={ctxUsed} limit={contextLimit} />
          ) : (
            <span title={t('status.contextFallback', formatTokens(ctxUsed))}>
              {formatTokens(ctxUsed)} / ctx
            </span>
          )}
        </span>
        {(() => {
          const usage = mainTokenUsage
          const hit = usage?.cacheHitTokens || 0
          const total = usage?.inputTokens || 0
          if (hit > 0 && total > 0) {
            const rate = (hit / total) * 100
            const color = rate > 60 ? '#22c55e' : rate > 30 ? '#f59e0b' : '#ef4444'
            return (
              <>
                <span className="status-divider" />
                <span
                  className="status-item"
                  style={{ color }}
                  title={t(
                    'status.cacheTooltip',
                    formatTokens(hit),
                    formatTokens(total),
                    rate.toFixed(0),
                  )}
                >
                  {rate.toFixed(0)}%
                </span>
              </>
            )
          }
          return null
        })()}
      </div>
      <div className="status-center">
        <span className="status-divider" />
        <span
          className="status-item"
          title={t(
            'status.tokenTooltip',
            formatTokens(execTokens),
            formatTokens(execTokenUsage?.inputTokens || 0),
            formatTokens(execTokenUsage?.outputTokens || 0),
          )}
        >
          {formatTokens(execTokens)} tok
        </span>
        <span className="status-divider" />
        <span className="status-item">{t('status.steps', String(totalCalls || 0))}</span>
        <span className="status-divider" />
        <span className="status-item">{formatDuration(totalDurationMs || 0)}</span>
      </div>
    </div>
  )
}
