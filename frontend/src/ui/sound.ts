/**
 * 弹窗音效 — Web Audio API 合成，零依赖零资源（不引入任何音频文件）。
 *
 * 三档音色（仅用于 agent/后端驱动的打断型弹窗，用户主动触发的 UI 不出声）：
 * - request  上行双音（G5→C6），柔和、邀请输入
 * - confirm  下行双音（A5→E5），注意、轻微紧迫
 * - approval 三连音（C6→E6→G6），审批/审阅提示
 *
 * 浏览器自动播放策略：AudioContext 首次创建后通常 suspended，直到出现一次用户手势；
 * 这里挂一次性全局手势监听，用户任意交互后恢复上下文，此后弹窗音效即时可用。
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
