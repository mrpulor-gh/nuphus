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
//   idle       空闲        34px 圆胶囊·麦克风图标；hover 延展「图标+语音输入」
//   activating 启动中      点击即宽胶囊·accent 音波（弱）；可再点取消
//   listening  聆听·转录   stt:ready 到达 · 红色音波 + 「叮」上行提示音
//   finishing  收尾识别    停止后尾部识别 · accent 音波（更弱）+ 「咚」下行提示音
//   错误                   stt:error → 低沉警示音 + 错误文案 4s
//
// stt_status 不可用时降级：no_microphone / 探测失败 → 整体隐藏（用户无法自救）；
// model_missing → 保留按钮，点击弹下载确认框（stt_download_model 后台下载，
// stt:download 事件驱动进度，完成后重新探测恢复正常态）。

import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { IconMic, IconX } from '../../ui/Icons'
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
        u1(); u2(); u3(); u4(); u5()
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

  // ── 命令式句柄：发送前冲刷语音会话（详见 VoiceButtonHandle 注释）──
  useImperativeHandle(ref, () => ({
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
  }), [])

  // 未探测 / 无麦克风 / 探测失败 → 整体隐藏；模型缺失保留按钮（点击走下载流）
  if (!available && !modelMissing) return null

  const handleClick = async () => {
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

  return (
    <>
      {/* 胶囊按钮：idle 默认 Chip 容器仅图标，hover 延展「图标+文字」；点击后胶囊保持
          展开宽度不变，内部整体切换为 9 条镜像包络音波（中心振幅最大向外递减+
          相位错开形成波传播，状态全靠颜色/节奏区分——见文件头状态机表） */}
      <IconButton
        variant="raw"
        className={`chat-voice-btn${phase !== 'idle' ? ` ${phase}` : ''}`}
        label={label}
        title={errorMsg || label}
        onClick={handleClick}
        disabled={disabled || phase === 'finishing'}
      >
        {phase !== 'idle' ? (
          <span className="voice-wave" aria-hidden>
            <i />
            <i />
            <i />
            <i />
            <i />
            <i />
            <i />
            <i />
            <i />
          </span>
        ) : (
          <IconMic size={15} />
        )}
        {/* 文字仅 idle 态 hover 延展呈现；进入音波状态后不再显示文字 */}
        {phase === 'idle' && <span className="voice-btn-label">{label}</span>}
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