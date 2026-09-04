import { useEffect, useRef, useState, useCallback } from 'react'
import { NuphusAvatar, type NuphusAvatarState } from '../../ui/NuphusAvatar'
import { ParticleField } from './ParticleField'
import { useLanguage } from '../../locales'
import { hasResumeCandidate } from '../lib/api'
import '../../styles/welcome.css'

interface WelcomeScreenProps {
  onSend: (text: string, images?: string[]) => void
  startupStats: { tools: number; memories: number }
  /** 「继续对话」点击后由父级重拉历史（Session Shelf 恢复链） */
  onResume?: () => void
}

/**
 * 表情表演时序（[状态, 进入时刻 ms]），复用 NuphusAvatar 状态机。
 * 唤醒 → 环顾 → 对视 → 欢喜 → 待命，一个生命体从沉睡到认出你的叙事。
 */
const PERFORMANCE: ReadonlyArray<[NuphusAvatarState, number]> = [
  ['working', 0], // 唤醒：双竖均衡器（相位错开）
  ['thinking', 1800], // 环顾：眼珠左右巡视 + 头部轻摆
  ['confirm', 3800], // 对视：目睁大 + 注意力脉冲
  ['success', 5200], // 欢喜：跃起 + 笑眼弧
  ['idle', 6500], // 待命：呼吸 + 眨眼（此阶段眼睛跟随鼠标）
]
const REVEAL_AT = 5350 // 文字在 success 段浮现
const BRAND = 'NUPHUS'

export function WelcomeScreen({ onResume }: WelcomeScreenProps) {
  const { t } = useLanguage()
  const [state, setState] = useState<NuphusAvatarState>('working')
  const [revealed, setRevealed] = useState(false)
  const [gaze, setGaze] = useState<{ x: number; y: number } | undefined>(undefined)
  const logoRef = useRef<HTMLDivElement>(null)
  const [resumable, setResumable] = useState(false)
  const [resuming, setResuming] = useState(false)

  // 是否存在可恢复的最近会话镜像（决定「继续对话」按钮显示）
  useEffect(() => {
    hasResumeCandidate()
      .then(v => setResumable(v === true))
      .catch(() => {})
  }, [])

  const handleResume = useCallback(async () => {
    if (resuming || !onResume) return
    setResuming(true)
    try {
      await onResume()
    } finally {
      setResuming(false)
    }
  }, [onResume, resuming])

  // 表情表演状态机
  useEffect(() => {
    const timers = PERFORMANCE.map(([s, delay]) => window.setTimeout(() => setState(s), delay))
    const revealTimer = window.setTimeout(() => setRevealed(true), REVEAL_AT)
    return () => {
      timers.forEach(clearTimeout)
      clearTimeout(revealTimer)
    }
  }, [])

  // 眼睛跟随鼠标：仅 idle 待命阶段开启（表演阶段走状态动画）
  useEffect(() => {
    if (state !== 'idle') {
      setGaze(undefined)
      return
    }
    let raf = 0
    let latest = { x: 0, y: 0 }
    const onMove = (e: MouseEvent) => {
      const rect = logoRef.current?.getBoundingClientRect()
      if (!rect) return
      const cx = rect.left + rect.width / 2
      const cy = rect.top + rect.height / 2
      latest = {
        x: Math.max(-1, Math.min(1, (e.clientX - cx) / (window.innerWidth / 2))),
        y: Math.max(-1, Math.min(1, (e.clientY - cy) / (window.innerHeight / 2))),
      }
      if (raf === 0) {
        raf = requestAnimationFrame(() => {
          raf = 0
          setGaze(prev => {
            if (prev && Math.abs(prev.x - latest.x) < 0.01 && Math.abs(prev.y - latest.y) < 0.01) {
              return prev
            }
            return latest
          })
        })
      }
    }
    window.addEventListener('mousemove', onMove)
    return () => {
      window.removeEventListener('mousemove', onMove)
      if (raf) cancelAnimationFrame(raf)
    }
  }, [state])

  return (
    <div className="welcome-screen">
      <ParticleField />
      <div className="welcome-logo" ref={logoRef}>
        <NuphusAvatar state={state} size={68} gaze={gaze} />
      </div>
      <div className={`welcome-brand${revealed ? ' is-revealed' : ''}`}>
        <h1 className="welcome-name" aria-label={BRAND}>
          {BRAND.split('').map((ch, i) => (
            <span
              key={i}
              className="welcome-letter"
              style={{ transitionDelay: `${i * 120}ms` }}
              aria-hidden
            >
              {ch}
            </span>
          ))}
        </h1>
        <p className="welcome-subtitle">{t('welcome.subtitle')}</p>
        {revealed && resumable && onResume && (
          <button
            type="button"
            className="welcome-resume-btn"
            onClick={() => void handleResume()}
            disabled={resuming}
          >
            {resuming ? '…' : t('welcome.resume')}
          </button>
        )}
      </div>
    </div>
  )
}
