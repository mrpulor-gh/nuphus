/**
 * TurnMetaBar —— 一轮执行的元数据条（耗时 / 上下文增量 / 步数），chip 视觉。
 *
 * ## 为什么是「唯一组件」
 *
 * 三处接入点（assistant 消息底部 / 执行面板 ctx 弹窗 / 移动端模型信息卡）必须
 * **共用同一数据源同一个渲染**。若各自实现，必然各自计时、各自格式化，最终像
 * 原缺陷那样三处显示三个不同的耗时。这里收口：
 *   - 数据：上游传 `meta: TurnMeta`（后端权威）
 *   - 判空：`isTurnMetaEmpty` → 不渲染空壳条（与 `trace_items` 同策略）
 *   - 标签：`turn.*` 走 i18n，zh/en 双侧齐备
 *
 * ## 令牌项 = 上下文增量（不是累计消耗）
 *
 * `tokens` slot 显示 `meta.contextDeltaTokens`（轮末占用 − 轮初占用，后端权威）。
 * 曾经这里显示「本轮所有 LLM 调用的 token 合计」——同一段上下文在同轮多次调用里
 * 被反复计入，累加和随调用次数线性膨胀（130K × 40 次 = 5.2M，实测出现过 11.2M），
 * 对用户是没有信息量的数字。增量回答「这一轮让上下文长了多少」。
 *
 * ## 耗时权威性（缺陷根因）
 *
 * 执行中前端**不持有起点**：起点由后端 `started_at_ms` 下发，调用方以
 * `Date.now() - started_at_ms` 实时推算后经 `liveDurationMs` 传入。刷新页面后
 * 起点仍是后端绝对值，计时从真实起点继续走，不归零。完成后以 `meta.durationMs`
 * 为准（后端权威值）。
 *
 * ## slot 化
 *
 * `slots` 决定显示哪几项，顺序即传入顺序；缺数据的 slot 自动跳过。
 * 消息底部给全三 slot；紧凑场景（如移动端）可只给 `['duration']`。
 */

import { useLanguage } from '../locales'
import type { TurnMeta } from '../core/types'
import { isTurnMetaEmpty, resolveTurnCalls, resolveTurnDuration } from '../core/types'

/** 元数据条可展示槽位 */
export type TurnMetaSlot = 'duration' | 'tokens' | 'steps'

export interface TurnMetaBarProps {
  /**
   * 后端权威元数据。耗时规则由 `resolveTurnDuration` 统一：
   *   · 有 `durationMs`（turn 已结束）→ 直接显示
   *   · 否则有 `startedAtMs`（进行中）→ `Date.now() - startedAtMs` 实时推算
   * 故本组件**不接受**调用方传入的实时值——那会让同一指标出现第二个来源。
   * 走秒由调用方的 tick 触发重渲染。
   */
  meta: TurnMeta | null | undefined
  /** 展示哪些槽位（默认全三 slot：耗时 / 令牌 / 步数） */
  slots?: TurnMetaSlot[]
  /** 附加 class（消息内 / 弹窗内视觉微调） */
  className?: string
}

/** 毫秒 → 紧凑可读时长（与执行面板 / 输入栏同款格式，避免三处各写一套） */
export function formatTurnDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0ms'
  if (ms < 1000) return `${Math.round(ms)}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  const m = Math.floor(ms / 60000)
  const s = Math.floor((ms % 60000) / 1000)
  return `${m}m ${s}s`
}

/** token 数 → 紧凑可读（1.2k / 3.4M），与输入栏 fmt 同款 */
function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M'
  if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k'
  return String(n)
}

export function TurnMetaBar({
  meta,
  slots = ['duration', 'tokens', 'steps'],
  className,
}: TurnMetaBarProps) {
  const { t } = useLanguage()

  // 判空：duration / token / steps 全空**且无起点**才不渲染（无起点连耗时都推不出）。
  // startedAtMs 必须单独计入判据——「仅带起点」的轮次有耗时可展示，不能当空壳条。
  const hasStart = meta?.startedAtMs != null && meta.startedAtMs > 0
  if (isTurnMetaEmpty(meta) && !hasStart) return null

  const m = meta ?? {}

  // 耗时 / 步数唯一出口，无兜底、无 live 分支：
  //   执行中 → Date.now() - startedAtMs（turn 开始即起跑）
  //   已结束 → durationMs（后端权威）
  // 走秒由调用方（ChatPanel/ChatInputBar）的 tick 触发重渲染，此处不持有时间值。
  const durationMs = resolveTurnDuration(meta)
  // 上下文增量（后端权威）：本轮让上下文长了多少，**不是**多次调用的累计消耗
  const ctxDelta = m.contextDeltaTokens ?? 0
  const steps = resolveTurnCalls(meta)

  // 图标一律不配：三个 chip 都是纯文字（时间/上下文/步数）。曾给令牌项加过一个
  // CPU 图标——既把 chip 拉宽挤同行其它项，又与「纯只读信息、不是按钮」的定位不符。
  const chips: { key: TurnMetaSlot; label: string; value: string }[] = []
  for (const slot of slots) {
    if (slot === 'duration' && durationMs > 0) {
      chips.push({ key: slot, label: t('turn.duration'), value: formatTurnDuration(durationMs) })
    } else if (slot === 'tokens' && ctxDelta > 0) {
      chips.push({
        key: slot,
        label: t('turn.ctxDelta'),
        value: `+${formatTokens(ctxDelta)}`,
      })
    } else if (slot === 'steps' && steps > 0) {
      chips.push({ key: slot, label: t('turn.steps'), value: String(steps) })
    }
  }

  // slot 全被数据缺失跳过（如只有起点、无 token 无步数）→ 同样不渲染
  if (chips.length === 0) return null

  return (
    <div className={`turn-meta-bar${className ? ` ${className}` : ''}`}>
      {chips.map(c => (
        <span key={c.key} className={`turn-meta-chip turn-meta-chip--${c.key}`}>
          <span className="turn-meta-chip-label">{c.label}</span>
          <span className="turn-meta-chip-value">{c.value}</span>
        </span>
      ))}
    </div>
  )
}
