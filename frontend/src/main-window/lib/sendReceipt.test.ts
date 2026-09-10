import { describe, expect, it, vi } from 'vitest'
import { createSendReceiptHub } from './sendReceipt'

/** 受理事件 = 后端真实收下消息的权威时点：命中 send_id 时立即回执，
 *  不等（也不依赖）onSend promise 完成。 */
describe('createSendReceiptHub', () => {
  it('(a) 匹配 send_id 的 message_accepted → 立即 ok=true 回执，不等 onSend promise', async () => {
    const hub = createSendReceiptHub()
    const reply = vi.fn()
    // onSend 仍在执行：promise 尚未 settle（受理后整轮可能跑几分钟）
    let settleOnSend: ((ok: boolean) => void) | undefined
    const onSend = new Promise<boolean>(res => {
      settleOnSend = res
    })
    const fire = hub.begin('req-1', reply)
    void onSend.then(ok => fire(ok, undefined))

    // 受理事件到达 → 立即回执，此时 onSend 远未完成
    hub.handleEvent({ type: 'message_accepted', send_id: 'req-1', source: 'desktop' })
    expect(reply).toHaveBeenCalledTimes(1)
    expect(reply).toHaveBeenCalledWith(true, undefined)
    expect(hub.pendingCount()).toBe(0)

    // 整轮结束（迟到的 promise 回执）→ 幂等忽略，不产生二次回执/二次收起
    settleOnSend?.(true)
    await onSend
    expect(reply).toHaveBeenCalledTimes(1)
  })

  it('(b) 受理前失败（onSend 回 ok:false）→ 立即 ok:false，且不被随后的受理事件改写', () => {
    const hub = createSendReceiptHub()
    const reply = vi.fn()
    const fire = hub.begin('req-2', reply)

    // 受理前失败（后端未就绪 / 未配置 Key / IPC 拒绝）
    fire(false, 'connection lost')
    expect(reply).toHaveBeenCalledTimes(1)
    expect(reply).toHaveBeenCalledWith(false, 'connection lost')

    // 迟到的受理事件不得把失败改写成成功（幂等，单一通道先到先得）
    hub.handleEvent({ type: 'message_accepted', send_id: 'req-2', source: 'desktop' })
    expect(reply).toHaveBeenCalledTimes(1)
    expect(reply).toHaveBeenLastCalledWith(false, 'connection lost')
  })

  it('(c) send_id 不匹配 → 无回执，登记保留', () => {
    const hub = createSendReceiptHub()
    const reply = vi.fn()
    hub.begin('req-3', reply)

    hub.handleEvent({ type: 'message_accepted', send_id: 'other', source: 'desktop' })
    hub.handleEvent({ type: 'message_accepted', source: 'mobile' }) // 无 send_id
    hub.handleEvent({ type: 'tool_call_start' }) // 无关事件
    hub.handleEvent(null)

    expect(reply).not.toHaveBeenCalled()
    expect(hub.pendingCount()).toBe(1)
  })

  it('无 sendId（老调用方）：不登记，事件分派不产生回执', () => {
    const hub = createSendReceiptHub()
    const reply = vi.fn()
    const fire = hub.begin('', reply)
    hub.handleEvent({ type: 'message_accepted', send_id: '', source: 'desktop' })

    expect(hub.pendingCount()).toBe(0)
    expect(reply).not.toHaveBeenCalled()
    // 老调用方回执不被包装：原样返回，调用即生效（可重复）
    fire(true)
    fire(true)
    expect(reply).toHaveBeenCalledTimes(2)
  })

  it('clear（卸载）：丢弃登记，事件分派变 no-op', () => {
    const hub = createSendReceiptHub()
    const reply = vi.fn()
    hub.begin('req-4', reply)
    hub.clear()

    hub.handleEvent({ type: 'message_accepted', send_id: 'req-4', source: 'desktop' })
    expect(reply).not.toHaveBeenCalled()
  })
})
