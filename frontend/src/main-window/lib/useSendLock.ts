// useSendLock.ts — 「发送 / 导出」重活期间的界面锁
//
// 背景：点「发送给 Leader」后要先离屏捕获整棵 DOM（html-to-image 克隆 + 编码）再落盘，
// 重活跑在主线程上，期间界面看起来是冻死的；若此时仍可点击，用户会连点重试，第二次
// 捕获叠上来，等待翻倍甚至重复投递。故整轮用一把锁：遮罩挡住指针与键盘，并起总时长兜底。
//
// 锁的必要条件是有解除路径：无解除路径的锁比不锁更糟。因此解锁只有 cancel() 一个出口，
// 成功 / 失败 / 回执超时 / 兜底到点 / 组件卸载全部汇入，且 cancel() 幂等。

import { type MutableRefObject, useCallback, useEffect, useRef, useState } from 'react'

/** 兜底时长：捕获本身无法中断（3s 字体等待 + 克隆编码之后仍可能挂住），
 *  回执也可能永不到达，到点必须无条件解锁。 */
export const SEND_TOTAL_TIMEOUT = 30_000

export interface SendLock {
  /** 遮罩是否显示（React state，驱动渲染）；与 busyRef 同源维护，不会出现错配 */
  sending: boolean
  /** 整轮占用标记：遮罩之外的第二道防线（遮罩期间重复点击直接忽略）。
   *  ref 读取不触发渲染，可直接在 async 流程与事件监听里判断。 */
  busyRef: MutableRefObject<boolean>
  /** 上锁：置标记 + 显示遮罩 + 起兜底计时；重复调用不会叠加计时器 */
  begin: (onTimeout: () => void) => void
  /** 解锁（幂等）：收掉兜底计时 + 复位标记 + 收起遮罩 */
  cancel: () => void
}

export function useSendLock(timeoutMs: number = SEND_TOTAL_TIMEOUT): SendLock {
  const [sending, setSending] = useState(false)
  const busyRef = useRef(false)
  const timerRef = useRef<number | null>(null)

  const cancel = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    busyRef.current = false
    setSending(false)
  }, [])

  const begin = useCallback(
    (onTimeout: () => void) => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current)
      busyRef.current = true
      setSending(true)
      timerRef.current = window.setTimeout(() => {
        timerRef.current = null
        // 兜底到点：先解锁再交给调用方提示，提示不会被自己的遮罩挡住
        cancel()
        onTimeout()
      }, timeoutMs)
    },
    [cancel, timeoutMs],
  )

  // 卸载清理：收掉计时器并复位标记（不 setState，避免卸载后再置状态）
  useEffect(
    () => () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current)
        timerRef.current = null
      }
      busyRef.current = false
    },
    [],
  )

  return { sending, busyRef, begin, cancel }
}
