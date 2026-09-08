import { useEffect, useRef, useState } from 'react'
import { IconX } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import type { ApiHealthEventKind, ApiHealthState, ApiHealthStatus } from '../../core/types'
import '../../styles/api-health.css'

export const initialApiHealthState = (): ApiHealthState => ({
  status: 'unknown',
  stableSince: null,
  lastTransitionAt: Date.now(),
  currentTurnId: 0,
  consecutiveFailures: 0,
  retryCount: 0,
  incidents: [],
  unreadCount: 0,
  pulse: null,
})

/** 事件分类说明（聚合行的可读标签；kind 由后端 Warning code 映射） */
const KIND_LABEL: Record<ApiHealthEventKind, string> = {
  retry: '连接重试',
  timeout: '响应超时',
  disconnect: '连接中断',
  truncated: '传输截断',
  provider: '服务商异常',
  recovered: '连接恢复',
}

/** 相对时间（时间线语义：最近发生在前，用「多久前」而非绝对时间） */
function relTime(ts: number): string {
  const d = Date.now() - ts
  if (d < 60_000) return '刚刚'
  if (d < 3_600_000) return `${Math.floor(d / 60_000)} 分钟前`
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)} 小时前`
  return `${Math.floor(d / 86_400_000)} 天前`
}

/** API 状态信号 = 标准状态点（圆点 + 光晕环），对齐 ext-agent-dot 视觉语言。
 *  颜色 = currentColor，由 .api-health-{status} 注入 var token；动画由 CSS 驱动。 */
export function ApiSignalIcon({ status, size = 13 }: { status: ApiHealthStatus; size?: number }) {
  return (
    <svg
      className={`api-health-icon api-health-${status}`}
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
    >
      <circle
        className="api-health-halo"
        cx="8"
        cy="8"
        r="6.5"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeOpacity="0.35"
      />
      <circle className="api-health-core" cx="8" cy="8" r="3.5" fill="currentColor" />
    </svg>
  )
}

export function ApiHealthBadge({
  state,
  compact = false,
  onRead: _onRead,
}: {
  state: ApiHealthState
  compact?: boolean
  onRead?: () => void
}) {
  const { t } = useLanguage()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const down = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', down)
    document.addEventListener('keydown', key)
    return () => {
      document.removeEventListener('mousedown', down)
      document.removeEventListener('keydown', key)
    }
  }, [open])
  const label = t(`apiHealth.${state.status}`)
  // 异常态展开文字标签（圆点孤立时用户无法识别含义）；正常态保持极简圆点不抢视线
  const showLabel = state.status === 'degraded' || state.status === 'offline'
  // 瞬时事件脉冲（传输截断 / 单次重试）：一次性动效，1.5s 后由状态机清除 pulse
  const pulsing = !!state.pulse
  // 时间线：分类聚合行按最近发生倒序（≤8 类）
  // ?? [] 兼容 HMR 旧 state（records → incidents 迁移期间内存中的旧对象无该字段）
  const incidents = [...(state.incidents ?? [])].sort((a, b) => b.lastAt - a.lastAt).slice(0, 8)
  return (
    <div
      className={`api-health ${compact ? 'api-health-compact' : ''}${pulsing ? ' is-pulsing' : ''}`}
      ref={ref}
    >
      <button
        className={`api-health-badge api-health-${state.status}`}
        aria-label={label}
        title={label}
        onClick={() => setOpen(v => !v)}
      >
        <ApiSignalIcon status={state.status} size={compact ? 11 : 12} />
        {showLabel && <span className="api-health-label">{label}</span>}
      </button>
      {open && (
        <div className="api-health-popover" role="dialog" aria-label={t('apiHealth.title')}>
          <div className="api-health-head">
            <strong>{t('apiHealth.title')}</strong>
            <span className="api-health-head-state">{label}</span>
            <button
              className="api-health-close"
              onClick={() => setOpen(false)}
              aria-label={t('common.close')}
            >
              <IconX size={12} />
            </button>
          </div>
          <div className="api-health-current">
            <ApiSignalIcon status={state.status} size={12} />
            <span>{label}</span>
            {state.status === 'stable' && state.stableSince && (
              <small>{`${t('apiHealth.stableFor')} ${Math.floor((Date.now() - state.stableSince) / 1000)}s`}</small>
            )}
          </div>
          <div className="api-health-records">
            {incidents.length === 0 ? (
              <p className="api-health-empty">{t('apiHealth.noRecords')}</p>
            ) : (
              incidents.map(inc => (
                <div className="api-health-incident" key={inc.kind}>
                  <span className={`api-health-incident-dot is-${inc.impact}`} aria-hidden="true" />
                  <span className="api-health-incident-label">{KIND_LABEL[inc.kind]}</span>
                  {inc.count > 1 && <span className="api-health-incident-count">×{inc.count}</span>}
                  <span className="api-health-incident-time">{relTime(inc.lastAt)}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  )
}
