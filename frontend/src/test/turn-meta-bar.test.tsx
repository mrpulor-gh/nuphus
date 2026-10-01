import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { TurnMetaBar, formatTurnDuration } from '../ui/TurnMetaBar'
import { isTurnMetaEmpty, type TurnMeta } from '../core/types'
import { LangProvider } from '../locales'

/**
 * TurnMetaBar —— 一轮执行元数据条（耗时 / 上下文增量 / 步数）的契约测试。
 *
 * 回归背景 1（耗时）：执行窗口耗时曾由 ChatPanel/ChatInputBar 的组件 ref 记起点
 * （`startTimeRef = Date.now()`），页面刷新即丢 → 后端仍在跑，前端却从刷新时刻
 * 重新计时（「耗时统计不可靠」）。修复后耗时统一由 `resolveTurnDuration` 求出：
 *   执行中 → `Date.now() - startedAtMs`（起点来自后端 `started_at_ms`，绝对时间戳）
 *   已结束 → `meta.durationMs`（后端权威）
 * 走秒由调用方的 tick 触发重渲染，本组件**不接受**调用方传入的实时值（避免第二来源）。
 *
 * 回归背景 2（令牌项）：曾显示「本轮所有 LLM 调用的 token 合计」，且后端 turn_meta
 * 跨轮不重置 → 同一段上下文被反复计入，气泡显示 11.2M。现在后端按轮重置、只在完成时
 * 下发 `contextDeltaTokens`（轮末占用 − 轮初占用，两个值都取自 API usage），界面只认增量。
 *
 * 本组件是三处接入点（消息底部 / ctx 弹窗 / 移动端）的唯一渲染出口，故在此固化
 * 「判空不渲染 / 执行中实时值 / 完成后权威值 / 令牌项=上下文增量 / 三项同屏」契约。
 *
 * 覆盖：
 * 1. 全空 meta（null / 空对象）→ 不渲染任何 chip（与后端 TurnMeta::is_empty 同判据）；
 * 2. 仅有起点 → 由起点推算出耗时并渲染，增量 / 步数缺省则不显示；
 * 3. 完成后带 durationMs → 用权威值（优先于 startedAtMs 推算），令牌项显示上下文增量；
 * 4. slots 过滤只渲染指定维度，缺数据的 slot 自动跳过。
 */
function renderBar(ui: React.ReactElement) {
  return render(<LangProvider>{ui}</LangProvider>)
}

describe('TurnMetaBar 契约', () => {
  it('① 空 meta 不渲染（无空壳条）', () => {
    const { container: c1 } = renderBar(<TurnMetaBar meta={null} />)
    expect(c1.querySelector('.turn-meta-bar')).toBeNull()
    const { container: c2 } = renderBar(<TurnMetaBar meta={{}} />)
    expect(c2.querySelector('.turn-meta-bar')).toBeNull()
    expect(isTurnMetaEmpty(null)).toBe(true)
  })

  it('② 仅有起点 → 由起点推算耗时并渲染', () => {
    // 起点取「当前时刻 - 12.5s」，使 resolveTurnDuration 推出 12.5s
    const meta: TurnMeta = { startedAtMs: Date.now() - 12_500 }
    const { container } = renderBar(<TurnMetaBar meta={meta} />)
    const chips = container.querySelectorAll('.turn-meta-chip')
    // 只有耗时一 chip（token / 步数尚为 0，被跳过）
    expect(chips.length).toBe(1)
    expect(container.querySelector('.turn-meta-chip--duration')?.textContent).toContain('12.5s')
    expect(container.querySelector('.turn-meta-chip--tokens')).toBeNull()
    expect(container.querySelector('.turn-meta-chip--steps')).toBeNull()
  })

  it('③ 完成后用后端权威 durationMs；令牌项 = 上下文增量（不是累计消耗）', () => {
    const meta: TurnMeta = {
      startedAtMs: 1_700_000_000_000,
      durationMs: 3_400,
      // 累计消耗：同轮多次调用把同一段上下文反复计入 → 会膨胀到 11.2M 这种废数
      inputTokens: 5_200_000,
      outputTokens: 8_000,
      // 上下文增量：轮末占用 − 轮初占用（后端权威）——界面只认这个
      contextDeltaTokens: 12_400,
      toolCalls: 4,
    }
    const { container } = renderBar(<TurnMetaBar meta={meta} />)
    // durationMs 优先于 startedAtMs 推算（同两者都有时以权威值为准）
    const dur = container.querySelector('.turn-meta-chip--duration')
    expect(dur?.textContent).toContain('3.4s')
    // 令牌 chip 显示增量（+12.4k），**不**显示 input+output 的累计值
    const tok = container.querySelector('.turn-meta-chip--tokens')
    expect(tok?.textContent).toContain('+12.4k')
    expect(tok?.textContent).not.toContain('5.2M')
    // 三项同屏：耗时 / 上下文增量 / 步数都在
    expect(container.querySelectorAll('.turn-meta-chip').length).toBe(3)
    const steps = container.querySelector('.turn-meta-chip--steps')
    expect(steps?.textContent).toContain('4')
    // 不配图标（纯文字 chip）
    expect(container.querySelector('.turn-meta-chip-icon')).toBeNull()
    expect(tok?.querySelector('svg')).toBeNull()
  })

  it('④ slots 过滤 + 缺数据自动跳过', () => {
    const meta: TurnMeta = { durationMs: 800, outputTokens: 42 }
    const { container } = renderBar(<TurnMetaBar meta={meta} slots={['duration']} />)
    expect(container.querySelectorAll('.turn-meta-chip').length).toBe(1)
    expect(container.querySelector('.turn-meta-chip--tokens')).toBeNull()
    // steps 请求了但无数据 → 不渲染该 chip，也不出空壳
    const { container: c2 } = renderBar(<TurnMetaBar meta={meta} slots={['steps']} />)
    expect(c2.querySelector('.turn-meta-bar')).toBeNull()
  })
})

describe('formatTurnDuration', () => {
  it('毫秒 → 紧凑可读（三段式，与执行面板 / 输入栏同款）', () => {
    expect(formatTurnDuration(0)).toBe('0ms')
    expect(formatTurnDuration(450)).toBe('450ms')
    expect(formatTurnDuration(3400)).toBe('3.4s')
    expect(formatTurnDuration(125_000)).toBe('2m 5s')
  })
})
