// voice-sounds — 语音输入三音提示体系
//
// Web Audio 实时合成，零音频资源文件（WebView2 原生支持 AudioContext）。
// 设计原则：音高走向即语义——
//   ready 「叮」上行纯音 + 五度泛音层（A4→E5，440→659Hz）＝ 可以说话了
//   stop  「咚」下行纯音（E5→C4，659→262Hz）＝ 聆听结束，收尾识别
//   error 低沉三角波双跳（220→175Hz）＝ 出错了，与提示音明确区分
// 正弦/三角波 + 快速包络防爆音，音量克制。全部绑相位转换触发（用户手势
// 链路上），满足 autoplay 策略；合成失败静默降级，绝不影响录音功能。

let ctx: AudioContext | null = null

interface ToneLayer {
  type: OscillatorType
  freqFrom: number
  freqTo: number
  /** 相对音量（主层为 1） */
  level: number
  /** 延迟发声（ms），用于叠音 */
  delayMs?: number
}

function play(layers: ToneLayer[], durationMs: number, master = 0.14): void {
  try {
    ctx ??= new AudioContext()
    if (ctx.state === 'suspended') void ctx.resume()
    const now = ctx.currentTime
    const dur = durationMs / 1000
    for (const layer of layers) {
      const t0 = now + (layer.delayMs ?? 0) / 1000
      const osc = ctx.createOscillator()
      const gain = ctx.createGain()
      osc.type = layer.type
      osc.frequency.setValueAtTime(layer.freqFrom, t0)
      osc.frequency.exponentialRampToValueAtTime(layer.freqTo, t0 + dur)
      gain.gain.setValueAtTime(0.0001, t0)
      gain.gain.exponentialRampToValueAtTime(master * layer.level, t0 + 0.012)
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + dur)
      osc.connect(gain)
      gain.connect(ctx.destination)
      osc.start(t0)
      osc.stop(t0 + dur + 0.05)
      osc.onended = () => {
        osc.disconnect()
        gain.disconnect()
      }
    }
  } catch {
    /* 音频子系统不可用时静默降级 */
  }
}

/** 就绪提示音「叮」：activating → listening，麦克风已开、可以说话 */
export function playVoiceReady(): void {
  play(
    [
      { type: 'sine', freqFrom: 440, freqTo: 659, level: 1 },
      { type: 'sine', freqFrom: 880, freqTo: 1318, level: 0.25, delayMs: 30 },
    ],
    120,
  )
}

/** 收尾提示音「咚」：listening → finishing，停止聆听、尾部识别中 */
export function playVoiceStop(): void {
  play(
    [
      { type: 'sine', freqFrom: 659, freqTo: 262, level: 1 },
      { type: 'sine', freqFrom: 330, freqTo: 131, level: 0.2, delayMs: 20 },
    ],
    150,
  )
}

/** 错误警示音：低沉三角波双跳，与提示音明确区分 */
export function playVoiceError(): void {
  play([{ type: 'triangle', freqFrom: 220, freqTo: 175, level: 1 }], 180, 0.1)
}
