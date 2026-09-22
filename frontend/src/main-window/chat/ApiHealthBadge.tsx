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
  retry: null,
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

/** API 状态信号 —— **状态语义化图标**（不再是「实心圆点 + 光晕」，与 mode 圆点明确区分）。
 *
 *  五态形状语言（统一 16×16、strokeWidth 1.5、linecap/linejoin round）：
 *  · connecting / retry → 断口加载环（3/4 圆弧 + dasharray 流动 + 整体旋转）＝「正在等」
 *  · offline           → 断裂环（上下对称双缺口，无旋转）+ 极缓脉冲 ＝「通路已断」
 *  · degraded          → 同心双弧（外圈满弧淡 + 内圈短弧波动）＝「不稳但在通」
 *  · stable / unknown  → 同心细环 + 实心微芯（静态，仅弹窗内可见；rail 上因 label 为 null 不渲染）
 *
 *  颜色 = currentColor，由 .api-health-{status} 注入 var token；动画由 CSS 驱动（尊重 reduced-motion）。 */
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
      {(status === 'connecting' || status === 'degraded') && (
        <>
          {/* 底衬满环（极淡）：给出「环已闭合」的基准，避免旋转弧看起来在漂移 */}
          <circle
            cx="8"
            cy="8"
            r="6"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeOpacity="0.16"
          />
          {status === 'connecting' ? (
            // 加载环：3/4 弧，dasharray 静态 + 整体旋转（旋转由 CSS 驱动）
            <circle
              className="api-health-arc"
              cx="8"
              cy="8"
              r="6"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeDasharray="28.27 9.42"
            />
          ) : (
            // 波动：内圈短弧（绕中心缓慢摆动，见 CSS api-health-sway）
            <circle
              className="api-health-arc-inner"
              cx="8"
              cy="8"
              r="3"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeDasharray="9.42 9.42"
              strokeDashoffset="4.71"
            />
          )}
        </>
      )}
      {status === 'offline' && (
        // 断裂环：上下对称双缺口（dasharray 用极短 dash + 长 gap 形成两处断口）
        <circle
          className="api-health-break"
          cx="8"
          cy="8"
          r="6"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeDasharray="30 3 30 3"
          strokeDashoffset="15"
          transform="rotate(90 8 8)"
        />
      )}
      {(status === 'stable' || status === 'unknown') && (
        <>
          <circle
            cx="8"
            cy="8"
            r="6"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeOpacity="0.45"
          />
          <circle cx="8" cy="8" r="2.6" fill="currentColor" />
        </>
      )}
    </svg>
  )
}

/**
 * rail 的**紧凑实时状态标签**——同时是「要不要渲染这块连接 UI」的判据。
 *
 * - 出问题时返回最短文本，直述「现在在发生什么」：`retry 1/3`（数字来自事件字段，
 *   不解析文案）/ `offline` / `connecting` / `degraded`；
 * - 正常态（stable / unknown 且无重试进度）返回 `null` → **连状态圆点都不渲染**。
 *   2026-09-22 定稿：常驻圆点无信息量，输入框底栏在健康时保持绝对干净。
 */
export function apiHealthRailLabel(state: ApiHealthState): string | null {
  if (state.status === 'offline') return 'offline'
  if (state.status === 'connecting') return 'connecting'
  // 重试进度优先于 degraded 单词：单次重试时 status 可能尚未判定为 degraded（仍 stable），
  // 但用户此刻最想知道的是「第几次 / 还剩几次」。重试成功收到内容后由状态机清空。
  if (state.retry) return `retry ${state.retry.attempt}/${state.retry.max}`
  if (state.status === 'degraded') return 'degraded'
  return null
}

/**
 * rail 图标的**视觉态**：与 `apiHealthRailLabel` 同源，但把「重试进度」映射为在途加载环。
 *
 * - `retry` 非空（status 可能仍是 stable）→ `connecting`（旋转加载环）——用户此刻在等重试；
 * - 其余直接沿用 status 中的可见态（offline / connecting / degraded）；
 * - 正常态（stable / unknown 且无 retry）由调用方判空不渲染，这里回落到 stable（弹窗内可复用）。
 */
export function apiHealthSignalStatus(state: ApiHealthState): ApiHealthStatus {
  if (state.status === 'offline') return 'offline'
  if (state.status === 'connecting') return 'connecting'
  if (state.retry) return 'connecting'
  if (state.status === 'degraded') return 'degraded'
  return state.status
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
  const railLabel = apiHealthRailLabel(state)
  // 正常态：整块连接 UI（含状态圆点）不渲染 —— 底栏健康时保持干净。
  // 注意必须在全部 hooks 之后返回，保证 hooks 调用顺序稳定。
  if (!railLabel) return null
  // 图标视觉态：把「重试进度」映射为在途加载环（status 可能仍是 stable）——与 railLabel 同源
  const signal = apiHealthSignalStatus(state)
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
        className={`api-health-badge api-health-${signal}`}
        aria-label={label}
        title={label}
        onClick={() => setOpen(v => !v)}
      >
        <ApiSignalIcon status={signal} size={compact ? 11 : 12} />
        {railLabel && <span className="api-health-label">{railLabel}</span>}
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
            <ApiSignalIcon status={signal} size={12} />
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