// VoiceButton — 语音输入按钮（本地 sherpa-onnx SenseVoice / 云端 transcriptions）
//
// ── 引擎事实（与 src-tauri/src/speech/ 一致）──
//   本地：SenseVoice model.int8.onnx + silero VAD，16kHz，zh，ITN 开，
//   实时流式转录（RTF ~0.03）：开麦后 partial 实时预览、VAD 按停顿逐句 final
//   上屏——不存在「先录后转」两阶段，开麦即「聆听+实时转录」单一状态。
//   云端：同一路径录音到内存，stop 后整段上传，单个 final 返回。
//
// ── 契约（与 speech/commands.rs 一致）──
//   命令 stt_start / stt_stop / stt_cancel / stt_status
//   事件 stt:ready () — 麦克风真正开始采集（模型/VAD 加载与开麦在 worker 内
//         完成，stt_start 返回时并未就绪）；仅从此刻起进入 listening 相位，
//         杜绝「点击即说话导致前半句丢失」
//        stt:partial { text } — ghost 预览（"" 清除）
//        stt:final { text, start_ms, end_ms } — 句段上屏，聆听中可多次到达
//        stt:done { reason } — 会话终止信号，每条退出路径恰好一次（相位唯一定点）
//        stt:error { message, recoverable }
//
// ── 统一状态机（前端相位 ↔ 用户心智 ↔ UI/音效）──
//   idle       空闲        34px 圆胶囊·实心麦克风图标；hover 延展「图标+语音输入」
//   activating 启动中      点击即展开 voice-note 胶囊（accent·静默波形）；可再点取消
//   listening  聆听·转录   stt:ready 到达 · 红点+计时+滚动波形 + 「叮」上行提示音；
//                          左滑胶囊 = 取消手势（拉动 ≥1 松开执行，Esc 同效）
//   finishing  收尾识别    停止后尾部识别 · accent 波形冻结 + 「咚」下行提示音
//   错误                   stt:error → 低沉警示音 + 错误文案 4s
//
// ── 视觉来源（voice-note 胶囊样式移植）──
// 激活态胶囊整体移植「按住录音/左滑取消」原型的视觉语言：红点+计时+滚动波形、
// 行滑移淡出、取消圆盘就位、弹性展开收拢；配色走 token 桥（error/accent/
// void-primary），详见 chat-input.css voice-note 段。交互按本机口径保持
// 「点击开/关」，原型手势裁剪：无片段预览/播放/擦洗（无真实音频产物），
// 松开即收尾转写进输入框。
//
// stt_status 不可用时降级：no_microphone / 探测失败 → 整体隐藏（用户无法自救）；
// model_missing → 保留按钮，点击弹下载确认框（stt_download_model 后台下载，
// stt:download 事件驱动进度，完成后重新探测恢复正常态）。

import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
  type CSSProperties,
  type ReactElement,
} from 'react'
import { createPortal } from 'react-dom'
import { IconX } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { listen } from '../../core/bridge'
import {
  sttCancel,
  sttStart,
  sttStatus,
  sttStop,
  getCapabilities,
  setCapability,
  type SttFinalPayload,
} from '../lib/api'
import {
  useSttModelDownload,
  sttDownloadProgressPct,
  sttDownloadProgressText,
} from '../lib/useSttModelDownload'
import { useLanguage } from '../../locales'
import { playVoiceReady, playVoiceStop, playVoiceError } from './voice-sounds'

/**
 * 统一相位机（详见文件头状态机表）：
 * idle → activating → listening → finishing → idle
 */
type VoicePhase = 'idle' | 'activating' | 'listening' | 'finishing'

/// 本地尾部识别正常 <1s（RTF 0.03，20s 上限语音也仅 ~2s）；
/// 云端模式 stop 后才上传+批量识别，长语音可能数十秒，故放宽到 60s。
/// 该值同时是零事件兜底场景的用户等待上限。
const FINISH_TIMEOUT_MS = 60_000

/// activating 看门狗：正常路径 stt:ready（成功）或 stt:error（失败）结束；
/// 30s 覆盖冷启动模型加载
const ACTIVATE_TIMEOUT_MS = 30_000

/* ── voice-note 视觉常量（样式移植自语音胶囊原型，比例适配输入行胶囊）──────── */
const VN_BARS = 12 // 波形条数（原型 28，按输入胶囊宽度缩放）
const VN_SAMPLE = 70 // 波形采样间隔 ms（原型同值）
const VN_GAIN = 60 // 电平增益 0..100（原型默认值）
const VN_CANCEL = 64 // 左滑取消触发距离 px（原型 90，胶囊更小按比例收紧）
const VN_CLOSE_MS = 360 // 取消后收拢动画时长 ms（原型同值）

/* 粗略语音形状：音节藏于词组、词组间留隙（原型原样保留，作无真实电平事件的驱动） */
function vnStandIn(t: number) {
  const syll = Math.abs(Math.sin(t * 6.1) * Math.sin(t * 1.9 + 0.7))
  const phrase = Math.sin(t * 0.9 + 0.4) > -0.55 ? 1 : 0.1
  return Math.min(1, 0.06 + syll * phrase * (0.5 + Math.random() * 0.5))
}

const vnClamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v))

const vnFmt = (ms: number) => {
  const s = Math.floor(ms / 1000)
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/* 实心麦克风（原型 MicMark：实心胶囊机身 + 弧线，与 lucide 描线版区分） */
function VnMicMark() {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <rect x="8.5" y="2" width="7" height="12.5" rx="3.5" fill="currentColor" stroke="none" />
      <path d="M5 11.5a7 7 0 0 0 14 0" />
      <path d="M12 18.5V22" />
    </svg>
  )
}

interface VoiceButtonProps {
  /** 终句文本提交（父组件拼入输入框） */
  onFinalText: (text: string) => void
  /** 中间结果 ghost 预览；空串表示清除 */
  onPartialText: (text: string) => void
  /** 会话处理中（isProcessing）时禁用 */
  disabled?: boolean
}

/** 暴露给父组件（ChatInputBar）的命令式句柄 */
export interface VoiceButtonHandle {
  /** 是否有活跃会话（activating / listening / finishing） */
  isActive: () => boolean
  /**
   * 发送前冲刷：listening → 转 finishing 并调 stt_stop；随后等待 stt:done
   * （尾部 final 在 done 之前到达，经 onFinalText 拼入输入框）。3s 超时兜底。
   * idle 时立即 resolve。
   */
  stopAndFlush: () => Promise<void>
}

export const VoiceButton = forwardRef<VoiceButtonHandle, VoiceButtonProps>(function VoiceButton(
  { onFinalText, onPartialText, disabled },
  ref,
) {
  const { t } = useLanguage()
  // null = 尚未探测；false = 不可用（配合 modelMissing 决定是否隐藏）
  const [available, setAvailable] = useState<boolean | null>(null)
  // 模型文件缺失（用户可自救：弹窗确认下载）；no_microphone/探测失败仍整体隐藏
  const [modelMissing, setModelMissing] = useState(false)
  const [showDownload, setShowDownload] = useState(false)
  const [phase, setPhase] = useState<VoicePhase>('idle')
  const [errorMsg, setErrorMsg] = useState('')
  /* voice-note 视觉态：左滑拉动量（0..1）、取消收拢遮罩、波形采样序列与计时 */
  const [pull, setPull] = useState(0)
  const [closing, setClosing] = useState(false)
  const [levels, setLevels] = useState<number[]>([])
  const [vnMs, setVnMs] = useState(0)
  const pullRef = useRef(0)
  const dragX = useRef<number | null>(null)
  const rawLen = useRef(0)
  const suppressClick = useRef(false)
  const closeTimer = useRef<ReturnType<typeof setTimeout>>()
  const phaseRef = useRef<VoicePhase>('idle')
  phaseRef.current = phase
  const errorTimer = useRef<ReturnType<typeof setTimeout>>()
  /** 等待会话终止（stt:done / stt:error）的 flush 回调 */
  const doneWaiters = useRef<Array<() => void>>([])
  const notifyDone = () => {
    const ws = doneWaiters.current
    doneWaiters.current = []
    ws.forEach(r => r())
  }

  // ── 可用性探测：无麦克风/探测失败 → 隐藏；模型缺失 → 保留按钮走下载流；
  // 可用 → 登记能力（providers.toml capabilities.stt）──
  const probe = async () => {
    try {
      const s = await sttStatus()
      if (!s) {
        setAvailable(false)
        setModelMissing(false)
        return
      }
      setAvailable(s.available)
      setModelMissing(!s.available && !!s.reason?.startsWith('model_missing'))
      if (s.available) {
        const caps = await getCapabilities().catch(() => null)
        if (!caps?.stt) {
          setCapability('stt', 'sherpa-onnx-local').catch(() => {})
        }
      }
    } catch {
      setAvailable(false)
      setModelMissing(false)
    }
  }

  useEffect(() => {
    probe()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ── 模型下载（事件驱动；done 后重新探测恢复正常态）──
  const dl = useSttModelDownload(() => {
    setShowDownload(false)
    probe()
  })

  // ── 事件订阅（无轮询）──
  useEffect(() => {
    let unl: Array<() => void> = []
    let mounted = true
    ;(async () => {
      const u1 = await listen<{ text: string }>('stt:partial', p => {
        onPartialText(p.text || '')
      })
      const u2 = await listen<SttFinalPayload>('stt:final', p => {
        if (p.text?.trim()) onFinalText(p.text.trim())
        // final 按设计会在聆听过程中到达（VAD 闭段即逐句上屏），只有 finishing
        // （已点停止）才意味着会话结束可复位；聆听中收到 final 必须保持
        // listening，否则 UI 与后端状态错位（stt_busy 悬挂会话的根因）。
        setPhase(prev => (prev === 'finishing' ? 'idle' : prev))
      })
      const u3 = await listen<{ message: string }>('stt:error', p => {
        setPhase('idle')
        onPartialText('')
        playVoiceError()
        setErrorMsg(p.message || t('input.voiceFailed'))
        clearTimeout(errorTimer.current)
        errorTimer.current = setTimeout(() => setErrorMsg(''), 4000)
        notifyDone()
      })
      // 会话终止信号（后端保证每条退出路径恰好发一次）：唯一可靠的相位复位点
      const u4 = await listen<{ reason?: string }>('stt:done', p => {
        setPhase('idle')
        onPartialText('')
        if (p?.reason === 'timeout') {
          setErrorMsg(t('input.voiceAutoStop'))
          clearTimeout(errorTimer.current)
          errorTimer.current = setTimeout(() => setErrorMsg(''), 4000)
        }
        notifyDone()
      })
      // 麦克风真正开始采集 → activating → listening。若用户在就绪前已取消
      // （相位已非 activating），忽略迟到事件，禁止复活为 listening
      const u5 = await listen('stt:ready', () => {
        setPhase(prev => (prev === 'activating' ? 'listening' : prev))
      })
      if (!mounted) {
        u1()
        u2()
        u3()
        u4()
        u5()
        return
      }
      unl = [u1, u2, u3, u4, u5]
    })()
    return () => {
      mounted = false
      unl.forEach(u => u())
      clearTimeout(errorTimer.current)
      // 组件卸载时若仍在会话中，取消会话避免麦克风悬挂
      if (phaseRef.current !== 'idle') {
        sttCancel().catch(() => {})
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ── activating 看门狗：正常路径 stt:ready（成功）或 stt:error（失败）结束。
  // 两者皆未到达（事件契约错位）时兜底复位，避免胶囊卡在启动态 ──
  useEffect(() => {
    if (phase !== 'activating') return
    const timer = setTimeout(() => {
      if (phaseRef.current !== 'activating') return
      setPhase('idle')
      setErrorMsg(t('input.voiceFailed'))
      clearTimeout(errorTimer.current)
      errorTimer.current = setTimeout(() => setErrorMsg(''), 4000)
    }, ACTIVATE_TIMEOUT_MS)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase])

  // ── finishing 看门狗：正常路径由 stt:done（新后端）或 final（旧后端）复位。
  // 旧后端在「语音已在聆听期间全部上屏、停止后零事件」场景不发任何事件，
  // 只能靠本看门狗兜底——此场景识别已成功，禁止误报失败；真实失败走
  // stt:error 通道（新后端 catch_unwind 保证 panic 也发 error）──
  useEffect(() => {
    if (phase !== 'finishing') return
    const timer = setTimeout(() => {
      if (phaseRef.current !== 'finishing') return
      setPhase('idle')
      onPartialText('')
    }, FINISH_TIMEOUT_MS)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase])

  // ── 提示音：绑相位转换而非点击，覆盖点击停止、发送冲刷（stopAndFlush
  // 同样走 listening→finishing）、busy 自愈重试等全部路径 ──
  // activating→listening = 「叮」可以说话了；listening→finishing = 「咚」收尾
  const prevPhase = useRef<VoicePhase>('idle')
  useEffect(() => {
    if (prevPhase.current !== 'listening' && phase === 'listening') playVoiceReady()
    if (prevPhase.current === 'listening' && phase === 'finishing') playVoiceStop()
    prevPhase.current = phase
  }, [phase])

  // ── voice-note 波形采样：listening 期间以占位电平驱动滚动条形（原型同源函数）。
  // 离开 listening 即停采样，levels 冻结留供 finishing/取消收拢使用。无真实
  // 电平事件，样形由 vnStandIn 模拟语音节奏（音节+词组+留隙）──
  useEffect(() => {
    if (phase !== 'listening') return
    const raw: number[] = []
    rawLen.current = 0
    setLevels([])
    setVnMs(0)
    const t0 = performance.now()
    const timer = window.setInterval(() => {
      const took = performance.now() - t0
      raw.push(vnStandIn(took / 1000))
      rawLen.current = raw.length
      setLevels(raw.slice(-VN_BARS))
      setVnMs(took)
    }, VN_SAMPLE)
    return () => clearInterval(timer)
  }, [phase])

  // ── 命令式句柄：发送前冲刷语音会话（详见 VoiceButtonHandle 注释）──
  useImperativeHandle(
    ref,
    () => ({
      isActive: () => phaseRef.current !== 'idle',
      stopAndFlush: () => {
        if (phaseRef.current === 'idle') return Promise.resolve()
        if (phaseRef.current === 'listening') {
          setPhase('finishing')
          sttStop().catch(() => {})
        } else if (phaseRef.current === 'activating') {
          // 尚未开麦，无尾部可冲刷：取消会话，done 到达后放行发送
          setPhase('idle')
          sttCancel().catch(() => {})
        }
        return new Promise<void>(resolve => {
          const waiter = () => {
            clearTimeout(timer)
            resolve()
          }
          const timer = setTimeout(() => {
            doneWaiters.current = doneWaiters.current.filter(w => w !== waiter)
            resolve()
          }, 3000)
          doneWaiters.current.push(waiter)
        })
      },
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }),
    [],
  )

  // 未探测 / 无麦克风 / 探测失败 → 整体隐藏；模型缺失保留按钮（点击走下载流）
  if (!available && !modelMissing) return null

  // ── 左滑取消（voice-note 手势）：armed 松开或 Esc 执行。stt:done 随后把引擎
  // 相位复位 idle；closing 遮罩在 360ms 内保住胶囊外壳，避免复位抢在收拢动画
  // 前卸载内容条 ──
  const cancelSession = () => {
    sttCancel().catch(() => {})
    setClosing(true)
    clearTimeout(closeTimer.current)
    closeTimer.current = setTimeout(() => setClosing(false), VN_CLOSE_MS)
  }

  const handleClick = async () => {
    if (closing) return // 取消收拢期间的点击不参与状态机
    setErrorMsg('')
    if (modelMissing) {
      setShowDownload(true)
      return
    }
    if (phase === 'idle') {
      // 先进入 activating：模型/VAD 加载与开麦在 worker 内完成，
      // 收到 stt:ready 才真正转 listening，避免用户提前说话丢字
      setPhase('activating')
      const start = async () => {
        await sttStart()
      }
      try {
        await start()
      } catch (e) {
        setPhase('idle')
        const msg = e instanceof Error ? e.message : String(e)
        if (msg.includes('stt_busy')) {
          // 前后端状态错位自愈（旧版 final 早退会留下悬挂会话）：
          // cancel 回收，worker 复位 slot 有 ≤100ms 级延迟，稍候重试一次
          await sttCancel().catch(() => {})
          await new Promise(r => setTimeout(r, 400))
          try {
            setPhase('activating')
            await start()
            return
          } catch (e2) {
            setPhase('idle')
            setErrorMsg(e2 instanceof Error ? e2.message : String(e2))
          }
        } else {
          setErrorMsg(msg)
        }
        clearTimeout(errorTimer.current)
        errorTimer.current = setTimeout(() => setErrorMsg(''), 4000)
      }
    } else if (phase === 'activating') {
      // 就绪前取消：立即复位 UI（stt:ready 守卫会忽略迟到事件），
      // 后端 worker 观察到 cancel 标志后清场并发 stt:done（幂等复位 idle）
      setPhase('idle')
      await sttCancel().catch(() => {})
    } else if (phase === 'listening') {
      setPhase('finishing')
      try {
        await sttStop()
      } catch {
        setPhase('idle')
      }
    }
    // finishing 态忽略点击（尾部识别通常 <1s）
  }

  const label = modelMissing
    ? t('input.voiceNeedModel')
    : phase === 'listening'
      ? t('input.voiceListening')
      : phase === 'finishing'
        ? t('input.voiceFinishing')
        : phase === 'activating'
          ? t('input.voiceActivating')
          : t('input.voiceStart')

  const dlPct = dl.progress ? sttDownloadProgressPct(dl.progress) : null

  // ── voice-note 渲染量 ──
  const armed = pull >= 1
  const vnOpen = phase !== 'idle' || closing // 取消收拢期间保持展开外壳
  const vnActive = phase === 'listening' || closing // 圆点/计时档位（启动期不点灯）
  const g = 0.4 + (VN_GAIN / 100) * 1.2
  const start = rawLen.current - levels.length
  const bars = [] as ReactElement[]
  for (let j = 0; j < VN_BARS; j++) {
    const k = levels.length - VN_BARS + j
    const v = k >= 0 ? levels[k] : 0
    bars.push(
      <i
        key={k >= 0 ? start + k : `e${j}`}
        className="vn-bar"
        style={{ '--l': vnClamp(v * g, 0, 1).toFixed(3) } as CSSProperties}
      />,
    )
  }

  return (
    <>
      {/* 胶囊按钮（voice-note 样式）：idle Chip 仅实心麦克风，hover 延展「图标+文字」；
          激活后弹性展开为「红点+计时+波形」胶囊，录音中左滑出取消圆盘、armed 松开执行。
          pointer 捕获保证拖拽跨出胶囊仍连续；armed 取消后吞掉紧随的 click，
          否则松手会被状态机误判为「点击结束」。 */}
      <IconButton
        variant="raw"
        className={`chat-voice-btn${vnOpen ? (closing ? ' cancel' : ` ${phase}`) : ''}`}
        label={label}
        title={errorMsg || label}
        onClick={() => {
          if (suppressClick.current) {
            suppressClick.current = false
            return
          }
          handleClick()
        }}
        disabled={disabled || phase === 'finishing'}
        style={vnOpen ? ({ '--pull': pull.toFixed(3) } as CSSProperties) : undefined}
        onPointerDown={e => {
          if (phaseRef.current !== 'listening') return
          try {
            e.currentTarget.setPointerCapture(e.pointerId)
          } catch {
            /* 脚本合成的 pointer 无 id 可捕获 */
          }
          dragX.current = e.clientX
          pullRef.current = 0
          setPull(0)
        }}
        onPointerMove={e => {
          if (phaseRef.current !== 'listening' || dragX.current === null) return
          const p = vnClamp((dragX.current - e.clientX) / VN_CANCEL, 0, 1)
          pullRef.current = p
          setPull(p)
        }}
        onPointerUp={e => {
          try {
            e.currentTarget.releasePointerCapture(e.pointerId)
          } catch {
            /* 从未捕获的指针 */
          }
          if (phaseRef.current !== 'listening' || dragX.current === null) return
          dragX.current = null
          if (pullRef.current >= 1) {
            suppressClick.current = true
            cancelSession()
          }
          pullRef.current = 0
          setPull(0)
        }}
        onPointerCancel={() => {
          dragX.current = null
          pullRef.current = 0
          setPull(0)
        }}
        onKeyDown={e => {
          if (e.key === 'Escape' && phaseRef.current === 'listening' && !closing) {
            e.preventDefault()
            cancelSession()
          }
        }}
      >
        {vnOpen ? (
          <>
            <span className="vn-rec" aria-hidden="true">
              {vnActive && <span className="vn-dot" />}
              {vnActive && <span className="vn-time">{vnFmt(vnMs)}</span>}
              <span className="vn-wave">{bars}</span>
            </span>
            {phase === 'listening' && !closing && (
              <span className="vn-cancel" data-on={armed || undefined} aria-hidden="true">
                <IconX size={13} strokeWidth={2.4} />
              </span>
            )}
            {phase === 'listening' && !closing && (
              <span className="vn-hint" data-on data-armed={armed || undefined} aria-hidden="true">
                {armed ? t('input.voiceReleaseCancel') : t('input.voiceSlideCancel')}
              </span>
            )}
          </>
        ) : (
          <>
            <VnMicMark />
            {/* 文字仅 idle 态 hover 延展呈现；进入胶囊态后不再显示文字 */}
            <span className="voice-btn-label">{label}</span>
          </>
        )}
      </IconButton>

      {/* ── 模型下载确认弹窗（复用 cmd-modal 模式，与 workflow 权限确认一致）── */}
      {showDownload &&
        createPortal(
          <div
            className="cmd-modal-overlay"
            onClick={() => {
              if (!dl.downloading) setShowDownload(false)
            }}
          >
            <div
              className="cmd-modal cmd-modal-sm"
              onClick={e => e.stopPropagation()}
              style={{ maxWidth: 420 }}
            >
              <div className="cmd-modal-header">
                <span className="cmd-modal-title">{t('input.voiceDlTitle')}</span>
                <IconButton
                  variant="modal-close"
                  label={t('input.voiceDlCancel')}
                  onClick={() => setShowDownload(false)}
                  disabled={dl.downloading}
                >
                  <IconX size={14} />
                </IconButton>
              </div>
              <div className="cmd-modal-body">
                <p
                  style={{
                    fontSize: 13,
                    color: 'var(--spark-muted)',
                    lineHeight: 1.6,
                    margin: '0 0 16px',
                  }}
                >
                  {t('input.voiceDlDesc')}
                </p>
                {dl.progress && (
                  <>
                    <div className="stt-dl-progress">
                      {dlPct !== null && (
                        <div className="stt-dl-progress-fill" style={{ width: `${dlPct}%` }} />
                      )}
                    </div>
                    <div className="stt-dl-progress-text" style={{ margin: '0 0 12px' }}>
                      {sttDownloadProgressText(dl.progress)}
                    </div>
                  </>
                )}
                {dl.error && (
                  <p style={{ fontSize: 12, color: 'var(--error)', margin: '0 0 12px' }}>
                    {t('input.voiceDlFailed')}：{dl.error}
                  </p>
                )}
                <div style={{ display: 'flex', gap: 8 }}>
                  <Button
                    variant="ghost"
                    size="sm"
                    style={{ flex: 1 }}
                    onClick={() => setShowDownload(false)}
                    disabled={dl.downloading}
                  >
                    {t('input.voiceDlCancel')}
                  </Button>
                  <Button
                    variant="primary"
                    size="sm"
                    style={{ flex: 1 }}
                    loading={dl.downloading}
                    onClick={dl.start}
                  >
                    {dl.error ? t('input.voiceDlRetry') : t('input.voiceDlStart')}
                  </Button>
                </div>
              </div>
            </div>
          </div>,
          document.body,
        )}
    </>
  )
})
