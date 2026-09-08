/**
 * StopChoiceDialog — 执行中「终止方式选择」弹窗（桌面 / 手机共用，唯一数据源）。
 *
 * 设计约束（2026-09-08）：终止弹窗的选项定义、文案与行为映射必须单一来源——
 * 桌面端 ChatInputBar 与手机端 Composer 均调用本组件，禁止任一端再实现第二套。
 * 两端差异仅在视觉（variant：desktop = compact-modal；mobile = 底部弹层）。
 *
 * 行为映射（与后端语义一致）：
 *   继续执行 → 关闭，无动作
 *   优雅终止 → onGraceful（桌面 graceful_stop / 手机 POST /stop → graceful_stop）
 *   强制终止 → onForce   （桌面 interrupt / 手机 POST /interrupt → interrupt）
 */
import { useEffect } from 'react'
import { createPortal } from 'react-dom'
import { useLanguage } from '../locales'

/** 终止方式选项（文案 key 唯一来源；两端共用，禁止在端内复制） */
export const STOP_CHOICES = [
  { id: 'continue', textKey: 'input.stopContinue', descKey: 'input.stopContinueDesc' },
  { id: 'graceful', textKey: 'input.stopGraceful', descKey: 'input.stopGracefulDesc' },
  { id: 'force', textKey: 'input.stopForce', descKey: 'input.stopForceDesc' },
] as const

export type StopChoiceId = (typeof STOP_CHOICES)[number]['id']

interface StopChoiceDialogProps {
  open: boolean
  /** desktop = compact-modal 视觉（桌面）；mobile = 底部弹层视觉（手机） */
  variant?: 'desktop' | 'mobile'
  onClose: () => void
  /** 选择「继续执行」（可省略，语义为仅关闭弹窗） */
  onContinue?: () => void
  /** 选择「优雅终止」 */
  onGraceful: () => void
  /** 选择「强制终止」 */
  onForce: () => void
}

export function StopChoiceDialog({
  open,
  variant = 'desktop',
  onClose,
  onContinue,
  onGraceful,
  onForce,
}: StopChoiceDialogProps) {
  const { t } = useLanguage()

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, onClose])

  if (!open) return null

  const isMobile = variant === 'mobile'

  const handleSelect = (id: StopChoiceId) => {
    onClose()
    if (id === 'continue') {
      onContinue?.()
      return
    }
    if (id === 'graceful') onGraceful()
    else onForce()
  }

  return createPortal(
    <div
      className={isMobile ? 'mobile-stop-choice-overlay' : 'compact-overlay'}
      style={isMobile ? undefined : { zIndex: 210 }}
      onClick={onClose}
    >
      <div
        className={
          isMobile ? 'mobile-stop-choice' : 'compact-modal compact-modal--sm compact-modal--fit'
        }
        onClick={e => e.stopPropagation()}
        role="dialog"
        aria-label={t('input.stopChooseTitle')}
      >
        <div className={isMobile ? 'mobile-stop-choice-head' : 'compact-header'}>
          {isMobile ? (
            <span className="mobile-stop-choice-title">{t('input.stopChooseTitle')}</span>
          ) : (
            <span className="compact-header-title">{t('input.stopChooseTitle')}</span>
          )}
        </div>
        {!isMobile && <div className="compact-divider" />}
        <div className={isMobile ? 'mobile-stop-choice-body' : 'compact-body'}>
          <div
            className={isMobile ? 'mobile-stop-choice-hint' : undefined}
            style={
              isMobile
                ? undefined
                : {
                    fontSize: 13,
                    color: 'var(--spark-secondary)',
                    lineHeight: 1.5,
                    marginBottom: 10,
                    opacity: 0.85,
                  }
            }
          >
            {t('input.forceStopConfirm')}
          </div>
          <div
            className={isMobile ? 'mobile-stop-choice-list' : undefined}
            style={isMobile ? undefined : { display: 'flex', flexDirection: 'column', gap: 4 }}
          >
            {STOP_CHOICES.map(opt => (
              <div
                key={opt.id}
                className={isMobile ? 'mobile-stop-choice-item' : 'compact-option-btn'}
                onClick={() => handleSelect(opt.id)}
              >
                <div
                  className={isMobile ? 'mobile-stop-choice-label' : 'compact-option-label'}
                  style={opt.id === 'force' ? { color: 'var(--danger, #ef4444)' } : undefined}
                >
                  {t(opt.textKey)}
                </div>
                <div className={isMobile ? 'mobile-stop-choice-desc' : 'compact-option-desc'}>
                  {t(opt.descKey)}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>,
    document.body,
  )
}
