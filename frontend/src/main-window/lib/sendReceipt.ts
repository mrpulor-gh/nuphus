/**
 * 发送回执单一出口（纯模块，不依赖 React，可直接单测）。
 *
 * 一次「非输入框发起」的发送（画布「发送给 Leader」）有两条回执来源：
 *  1. 后端受理事件 `message_accepted`——消息被真实收下（开启新执行 / 进入追加队列）；
 *  2. `onSend` promise 的最终结果——整轮执行结束才 settle（可能几分钟，也可能中途失败）。
 * 画布的发送遮罩 / 全屏画布必须在 (1) 立即收起，而 (2) 只作兜底。
 *
 * 因此这里按 sendId 登记 pending 回调，并把回执包成幂等函数：两条来源谁先到谁生效，
 * 后到的一律忽略并自注销——保证 `nuphus:send-result` 只发一次，不出现二次收起。
 *
 * 无 sendId（老调用方）不登记，直接返回原回调，行为与引入本模块前完全一致。
 */

export type ReceiptReply = (ok: boolean, message?: string) => void

/** 事件分派只关心 type + send_id（结构类型，便于单测构造最小事件）。
 *  source（"desktop" | "mobile"）随受理事件携带但不参与判定。 */
export interface SendReceiptEvent {
  type: string
  send_id?: string | null
  source?: string
}

export interface SendReceiptHub {
  /**
   * 登记一次发送并返回幂等回执。sendId 非空时进入 pending；为空时原样返回 reply
   * （老调用方路径，不登记也不拦截）。
   */
  begin(sendId: string | null | undefined, reply: ReceiptReply): ReceiptReply
  /** 事件分派：`message_accepted` + 命中 send_id → 立即 ok=true 回执（不等 onSend）。 */
  handleEvent(event: SendReceiptEvent | null | undefined): void
  /** 丢弃全部登记（组件卸载）；不触发任何回执。 */
  clear(): void
  /** 仍在等待回执的 sendId 数（诊断 / 测试用）。 */
  pendingCount(): number
}

export function createSendReceiptHub(): SendReceiptHub {
  const pending = new Map<string, ReceiptReply>()

  const begin = (sendId: string | null | undefined, reply: ReceiptReply): ReceiptReply => {
    if (typeof sendId !== 'string' || !sendId) return reply
    let settled = false
    const fire: ReceiptReply = (ok, message) => {
      // 幂等：首次生效，后续（事件与 promise 谁后到）直接忽略
      if (settled) return
      settled = true
      // 仅当仍指向本次登记时注销（防止同 id 重登记后误删新登记）
      if (pending.get(sendId) === fire) pending.delete(sendId)
      reply(ok, message)
    }
    pending.set(sendId, fire)
    return fire
  }

  const handleEvent = (event: SendReceiptEvent | null | undefined): void => {
    if (!event || event.type !== 'message_accepted') return
    const sendId = event.send_id
    if (typeof sendId !== 'string' || !sendId) return
    const fire = pending.get(sendId)
    if (!fire) return
    // 受理 = 真实发送成功：立刻回执，不等 onSend promise
    fire(true)
  }

  return {
    begin,
    handleEvent,
    clear: () => pending.clear(),
    pendingCount: () => pending.size,
  }
}
