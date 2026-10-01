/**
 * mobile store 提炼链路钉子（2026-10 审计 D1/D2 修复配套）。
 *
 * 覆盖两条曾断裂的行为契约：
 *  - refine_prompt：forced → 置 refining（触发在 App.tsx onEvent，此处钉状态契约）；
 *    refining 中收到新提示必须被忽略（防叠加/覆盖 pendingRefine 破坏锁释放语义）
 *  - session_info：模型真切换 → pendingRefine 作废（D2）；模型未变 → 不动
 *    pendingRefine；refining 执行锁不因切换被打断
 */
import { describe, it, expect } from 'vitest'
import { chatReducer, initialChatState, type ChatState } from './store'
import type { NuphusEvent } from '../core/types'

function stateWithPendingRefine(model = 'step-5-preview'): ChatState {
  return {
    ...initialChatState,
    model,
    pendingRefine: {
      currentTokens: 307_200,
      refineLimit: 300_000,
      forceLimit: 500_000,
      threshold: 0.3,
      contextWindow: 1_000_000,
      forced: false,
    },
  }
}

const refinePrompt = (forced: boolean, tier = 'large'): NuphusEvent =>
  ({
    type: 'refine_prompt',
    current_tokens: 500_000,
    refine_limit: 300_000,
    force_limit: 500_000,
    threshold: 0.3,
    context_window: 1_000_000,
    tier,
    force_threshold: 0.5,
    force_min: 0.5,
    force_max: 0.8,
    forced,
  }) as NuphusEvent

describe('mobile store refine 链路', () => {
  it('forced 提炼：置 refining 且不产生 pendingRefine（触发契约在 App 层）', () => {
    const next = chatReducer(initialChatState, {
      type: 'event',
      event: refinePrompt(true, 'small'),
    })
    expect(next.refining).toBe(true)
    expect(next.pendingRefine).toBeNull()
  })

  it('non-forced 提示：落入 pendingRefine 携带全部字段', () => {
    const next = chatReducer(initialChatState, {
      type: 'event',
      event: refinePrompt(false),
    })
    expect(next.refining).toBe(false)
    expect(next.pendingRefine).toMatchObject({
      currentTokens: 500_000,
      forceLimit: 500_000,
      contextWindow: 1_000_000,
    })
  })

  it('refining 中收到新提示：忽略（防覆盖 pendingRefine 破坏锁释放）', () => {
    const refining: ChatState = { ...initialChatState, refining: true }
    const next = chatReducer(refining, {
      type: 'event',
      event: refinePrompt(false),
    })
    // 「忽略」按语义断言（event 分支外层总包 activity.detail 派生，无引用相等）：
    // refining 执行锁保持、pendingRefine 不被新提示创建/覆盖
    expect(next.refining).toBe(true)
    expect(next.pendingRefine).toBeNull()
  })

  it('D2：模型真切换 → pendingRefine 作废，refining 执行锁不动', () => {
    const next = chatReducer(stateWithPendingRefine(), {
      type: 'event',
      event: { type: 'session_info', session_id: 'x', model: 'deepseek-v4-flash', timestamp: 0 },
    })
    expect(next.model).toBe('deepseek-v4-flash')
    expect(next.pendingRefine).toBeNull()

    // 切换不打断正在执行的提炼（refining 是执行锁，不是询问态）
    const locked: ChatState = {
      ...stateWithPendingRefine(),
      refining: true,
    }
    const kept = chatReducer(locked, {
      type: 'event',
      event: { type: 'session_info', session_id: 'x', model: 'deepseek-v4-flash', timestamp: 0 },
    })
    expect(kept.refining).toBe(true)
    expect(kept.pendingRefine).toBeNull()
  })

  it('模型未变：pendingRefine 原样保留（不误伤）', () => {
    const before = stateWithPendingRefine('step-5-preview')
    const next = chatReducer(before, {
      type: 'event',
      event: {
        type: 'session_info',
        session_id: 'x',
        model: 'step-5-preview',
        timestamp: 0,
      },
    })
    expect(next.pendingRefine).toEqual(before.pendingRefine)
  })
})
