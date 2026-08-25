/**
 * 执行弹窗（轻风格）：查看单条消息的完整执行过程。
 * 条目按实际发生顺序排列：思考 → agent 流式文本 → 工具调用（含状态/时长）。
 * 不做 Tab 分离——过程就是时间线，文本也是过程的一部分。
 * 流式输出自动下拉到底；轻量视觉：小字号、hairline 分隔、无重阴影。
 * Header 实时展示会话 token 统计：Cache（缓存命中）+ tok（执行消耗 = input + output，
 * 与桌面端 StatusBar.execTokens 同源）。
 */

import { useEffect, useRef } from 'react'
import { Check, Gauge, Loader2, X, Zap } from 'lucide-react'
import type { TraceItem } from '../store'
import { t } from '../i18n'

interface Props {
  traceItems: TraceItem[]
  /** 会话累计上下文用量（store.tokenUsage，token_usage 事件实时更新） */
  tokenUsage?: { inputTokens: number; outputTokens?: number; cacheHitTokens?: number }
  onClose: () => void
}

/** token 数值紧凑格式化：1.2k / 3.4M */
function formatTokens(n: number | undefined): string | null {
  if (n === undefined || n === null) return null
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

/** 提取工具调用的关键参数（路径/命令），供手机端执行过程展示。
 *  system_shell → command + cwd；其他工具优先显示常用字段，兜底完整 JSON。 */
function formatToolParams(params: unknown): string | null {
  if (params === undefined || params === null) return null
  if (typeof params === 'string') return params
  if (typeof params !== 'object') return String(params)
  const p = params as Record<string, unknown>

  // 常见路径/命令字段（按重要性排序）
  const pick = ['command', 'cwd', 'path', 'file', 'file_path', 'url', 'query', 'repo']
  const lines: string[] = []
  for (const key of pick) {
    const v = p[key]
    if (v === undefined || v === null) continue
    if (typeof v === 'string' && v.length > 0) {
      lines.push(`${key}: ${v}`)
    } else if (typeof v === 'number') {
      lines.push(`${key}: ${v}`)
    }
  }
  if (lines.length > 0) return lines.join('  ')
  // 无常用字段 → 精简 JSON（截断防撑爆弹窗）
  try {
    const s = JSON.stringify(p)
    return s.length > 160 ? s.slice(0, 157) + '…' : s
  } catch {
    return null
  }
}

export default function TraceModal({ traceItems, tokenUsage, onClose }: Props) {
  const listRef = useRef<HTMLDivElement>(null)

  // 弹窗打开时锁定背景滚动：body + .mobile-app 双锁（iOS touchmove 会穿透
  // 子滚动容器 .mobile-messages，仅锁 body 不可靠——.mobile-app 才是会话滚动根）
  useEffect(() => {
    const app = document.querySelector('.mobile-app') as HTMLElement | null
    const prevAppOverflow = app?.style.overflow ?? ''
    const prevBodyOverflow = document.body.style.overflow
    if (app) app.style.overflow = 'hidden'
    document.body.style.overflow = 'hidden'
    return () => {
      if (app) app.style.overflow = prevAppOverflow
      document.body.style.overflow = prevBodyOverflow
    }
  }, [])

  // 流式输出自动下拉：依赖 traceItems 引用变化（每次 delta 新数组）
  useEffect(() => {
    const el = listRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [traceItems])

  // ── 缓存命中率（百分比展示，公式与三档配色对齐桌面 StatusBar/ChatInputBar）──
  // 基数为 inputTokens（与桌面端 cacheTotal 同源，cacheHitTokens 是其中已缓存部分）。
  const cacheHit = tokenUsage?.cacheHitTokens ?? 0
  const cacheTotal = tokenUsage?.inputTokens ?? 0
  const cacheRate = cacheTotal > 0 ? Math.round((cacheHit / cacheTotal) * 100) : null
  const cacheColor =
    cacheRate !== null
      ? cacheRate > 60
        ? '#22c55e'
        : cacheRate > 30
          ? '#f59e0b'
          : '#ef4444'
      : undefined

  // ── 执行消耗（对齐桌面 StatusBar.execTokens = input + output）：弹窗 tok 数字与 tooltip ──
  // 后端 token_usage 事件的 outputTokens 可能为 undefined（后端不可用/旧版本），按 0 处理；
  // 此时 tok 显示 = inputTokens，与现状一致（不显示 +0）。
  const inputTokens = tokenUsage?.inputTokens ?? 0
  const outputTokens = tokenUsage?.outputTokens ?? 0
  const execTokens = inputTokens + outputTokens

  return (
    <div className="mobile-modal-overlay" onClick={onClose}>
      <div className="mobile-modal" onClick={e => e.stopPropagation()}>
        <div className="mobile-modal-header">
          <span className="mobile-modal-title">执行过程</span>
          {(tokenUsage?.cacheHitTokens !== undefined || tokenUsage?.inputTokens !== undefined) && (
            <span className="mobile-modal-header-meta">
              {cacheRate !== null && (
                <span
                  className="mobile-modal-stat"
                  style={{ color: cacheColor }}
                  title={t(
                    'status.cacheTooltip',
                    formatTokens(cacheHit),
                    formatTokens(cacheTotal),
                    cacheRate,
                  )}
                >
                  <Zap size={11} aria-hidden="true" />
                  Cache {cacheRate}%
                </span>
              )}
              {execTokens > 0 && (
                <span
                  className="mobile-modal-stat"
                  title={t(
                    'status.tokenTooltip',
                    formatTokens(execTokens),
                    formatTokens(inputTokens),
                    formatTokens(outputTokens),
                  )}
                >
                  <Gauge size={11} aria-hidden="true" />
                  tok {formatTokens(execTokens)}
                </span>
              )}
            </span>
          )}
          <button type="button" className="mobile-modal-close" onClick={onClose} aria-label="关闭">
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <div ref={listRef} className="mobile-modal-trace">
          {traceItems.length === 0 && <div className="mobile-modal-trace-empty">暂无执行过程</div>}
          {traceItems.map((item, i) => {
            if (item.kind === 'thinking') {
              return (
                <div key={i} className="mobile-modal-trace-item is-thinking">
                  <span className="mobile-modal-trace-text">{item.text}</span>
                </div>
              )
            }
            if (item.kind === 'text') {
              return (
                <div key={i} className="mobile-modal-trace-item is-text">
                  <span className="mobile-modal-trace-text">{item.text}</span>
                </div>
              )
            }
            return (
              <div key={i} className="mobile-modal-trace-item is-tool">
                <div className="mobile-modal-trace-tool-main">
                  <span className="mobile-modal-trace-tool-name">{item.name}</span>
                  {formatToolParams(item.params) && (
                    <code className="mobile-modal-trace-tool-params">
                      {formatToolParams(item.params)}
                    </code>
                  )}
                </div>
                {item.status === 'running' ? (
                  <Loader2 size={13} className="mobile-spin" aria-label="执行中" />
                ) : item.status === 'ok' ? (
                  <Check size={13} className="is-ok" aria-label="成功" />
                ) : (
                  <X size={13} className="is-fail" aria-label="失败" />
                )}
                {item.durationMs !== undefined && (
                  <span className="mobile-modal-trace-duration">({item.durationMs}ms)</span>
                )}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}