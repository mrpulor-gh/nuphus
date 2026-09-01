/**
 * UI 音效体系 — Web Audio API 合成，零依赖零资源（不引入任何音频文件）。
 *
 * 两类用途，音色刻意区分：
 * 1. 弹窗音效（agent/后端驱动的打断型弹窗）：request / confirm / approval
 * 2. 交互反馈（用户主动操作，短促克制）：send / session / done / switch
 *
 * 音高走向即语义：
 * - send     C5→G5 短促上行（523→784Hz）＝ 指令已发出，轻快肯定
 * - session  A5 极轻单音（880Hz）＝ 导航定位，不打扰
 * - done     E5→B5 上行双音 + 泛音（659→988Hz）＝ 任务完成，上扬收稳
 * - switch   G5→C6 清脆双响（784→1046Hz）＝ 状态切换
 *
 * 浏览器自动播放策略：AudioContext 首次创建后通常 suspended，直到出现一次用户手势；
 * 这里挂一次性全局手势监听，用户任意交互后恢复上下文，此后音效即时可用。
 */

let audioCtx: AudioContext | null = null

function getCtx(): AudioContext | null {
  try {
    if (!audioCtx) {
      const AC =
        window.AudioContext ||
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
      if (!AC) return null
      audioCtx = new AC()
      const resume = () => {
        if (audioCtx && audioCtx.state === 'suspended') void audioCtx.resume()
      }
      window.addEventListener('pointerdown', resume)
      window.addEventListener('keydown', resume)
    }
    if (audioCtx.state === 'suspended') void audioCtx.resume()
    return audioCtx
  } catch {
    return null
  }
}

/**
 * 预热：首次用户手势时创建并恢复 AudioContext（在 pointerdown 同步栈中创建，
 * state 直接为 running；避免首次音效在 click 时才创建、异步 resume 丢音）。
 */
export function ensureAudioCtx(): void {
  getCtx()
}

/** 单音：正弦波 + 快速起音 + 指数衰减（避免电子音生硬感） */
function tone(freq: number, delay: number, duration: number, peak = 0.06): void {
  const ctx = getCtx()
  if (!ctx) return
  const osc = ctx.createOscillator()
  const gain = ctx.createGain()
  osc.type = 'sine'
  osc.frequency.value = freq
  const t0 = ctx.currentTime + delay
  gain.gain.setValueAtTime(0, t0)
  gain.gain.linearRampToValueAtTime(peak, t0 + 0.02)
  gain.gain.exponentialRampToValueAtTime(0.0001, t0 + duration)
  osc.connect(gain)
  gain.connect(ctx.destination)
  osc.start(t0)
  osc.stop(t0 + duration + 0.06)
}

export type PopupSound = 'request' | 'confirm' | 'approval'

export function playPopupSound(kind: PopupSound): void {
  switch (kind) {
    case 'request':
      tone(783.99, 0, 0.2)
      tone(1046.5, 0.1, 0.24)
      break
    case 'confirm':
      tone(880, 0, 0.16)
      tone(659.25, 0.09, 0.22, 0.05)
      break
    case 'approval':
      tone(1046.5, 0, 0.15)
      tone(1318.5, 0.08, 0.15)
      tone(1567.98, 0.16, 0.26)
      break
  }
}

export type UiSound = 'send' | 'session' | 'done' | 'switch' | 'deny' | 'error' | 'retry'

export function playUiSound(kind: UiSound): void {
  switch (kind) {
    case 'send':
      // 发送：C5→G5 短促上行，指令已发出
      tone(523.25, 0, 0.08, 0.16)
      tone(783.99, 0.045, 0.1, 0.1)
      break
    case 'session':
      // 会话选中：A5 轻单音，导航定位不打扰
      tone(880, 0, 0.06, 0.09)
      break
    case 'done':
      // 执行完成：E5→B5 上行双音 + 高八度泛音，任务完成上扬收稳
      tone(659.25, 0, 0.12, 0.14)
      tone(987.77, 0.09, 0.18, 0.12)
      tone(1975.53, 0.09, 0.14, 0.04)
      break
    case 'switch':
      // 切换：G5→C6 清脆双响，状态切换
      tone(783.99, 0, 0.07, 0.12)
      tone(1046.5, 0.06, 0.09, 0.09)
      break
    case 'deny':
      // 拒绝/取消：C5→G4 低沉下行，否定操作
      tone(523.25, 0, 0.14, 0.12)
      tone(392, 0.07, 0.16, 0.09)
      break
    case 'error':
      // 执行错误：A4→E4→C4 三音下行（440→330→262Hz），节奏拉长低沉警示，
      // 与 deny 双音区分——错误是「执行中断」而非「操作否定」
      tone(440, 0, 0.16, 0.11)
      tone(329.63, 0.13, 0.18, 0.1)
      tone(261.63, 0.28, 0.24, 0.09)
      break
    case 'retry':
      // 重试提醒：G4→C4 两声短促低音「咚咚」，中性提示「还在重试、请稍候」，
      // 区别于 error 三音下行——重试不是失败，不打断不恐慌
      tone(392, 0, 0.1, 0.12)
      tone(261.63, 0.16, 0.16, 0.12)
      break
  }
}
