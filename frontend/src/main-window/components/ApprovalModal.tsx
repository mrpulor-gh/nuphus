import { useState, useEffect, useCallback } from 'react'
import { createPortal } from 'react-dom'
import { invoke } from '../../core/bridge'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { playPopupSound } from '../../ui/sound'
import '../../styles/approval.css'

interface ApprovalModalProps {
  open: boolean
  kind: string
  title: string
  content: string
  actionId: string
  tenetCount?: number
  onClose: () => void
}

export function ApprovalModal({
  open,
  kind,
  title,
  content,
  actionId,
  tenetCount,
  onClose,
}: ApprovalModalProps) {
  const { t } = useLanguage()
  const [visible, setVisible] = useState(false)
  const [animating, setAnimating] = useState(false)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<'approved' | 'rejected' | null>(null)

  useEffect(() => {
    if (open) {
      playPopupSound('approval')
      setVisible(true)
      requestAnimationFrame(() => setAnimating(true))
    } else {
      setAnimating(false)
      const t = setTimeout(() => {
        setVisible(false)
        setResult(null)
      }, 300)
      return () => clearTimeout(t)
    }
  }, [open])

  // 审批项后端 TTL 600s 自动过期；前端同步兜底关闭，避免弹窗无限挂起
  useEffect(() => {
    if (!open) return
    const timer = setTimeout(() => {
      onClose()
    }, 600_000)
    return () => clearTimeout(timer)
  }, [open, onClose])

  const handleApprove = useCallback(async () => {
    if (busy) return
    setBusy(true)
    try {
      await invoke('approve_pending', { actionId })
      setResult('approved')
    } catch (e) {
      // 审批项已过期/不存在（后端 TTL 600s 清理）→ 弹窗已无意义，直接关闭避免卡死
      console.error('Approve failed:', e)
      onClose()
    } finally {
      setBusy(false)
    }
  }, [actionId, busy, onClose])

  const handleReject = useCallback(async () => {
    if (busy) return
    setBusy(true)
    try {
      await invoke('reject_pending', { actionId })
      setResult('rejected')
    } catch (e) {
      console.error('Reject failed:', e)
      onClose()
    } finally {
      setBusy(false)
    }
  }, [actionId, busy, onClose])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (busy || result) return
      if (e.key === 'Enter') {
        e.preventDefault()
        handleApprove()
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        handleReject()
      }
    },
    [busy, result, handleApprove, handleReject],
  )

  useEffect(() => {
    if (open) window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [open, handleKeyDown])

  if (!visible) return null

  const modalContent = (
    <div
      className="compact-overlay approval-overlay"
      style={{ pointerEvents: open ? 'auto' : 'none' }} /* 动态：开关联动 */
    >
      <div
        className={`compact-modal compact-modal--md compact-modal--fit ${animating ? '' : 'is-closing'}`}
        onClick={e => e.stopPropagation()}
        style={{ transform: animating ? 'none' : undefined, opacity: animating ? 1 : undefined }} /* 动态：入场动画状态 */
      >
        {result ? (
          <div className="approval-result">
            <div
              className={`approval-result-icon ${
                result === 'approved' ? 'approval-result-icon--ok' : 'approval-result-icon--neutral'
              }`}
            >
              {result === 'approved' ? (
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <polyline points="20 6 9 17 4 12" />
                </svg>
              ) : (
                <svg
                  width="22"
                  height="22"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              )}
            </div>
            <div className="approval-result-title">
              {result === 'approved' ? t('approval.saved') : t('approval.rejected')}
            </div>
            <div className="approval-result-desc">
              {result === 'approved' ? t('approval.savedDesc') : t('approval.rejectedDesc')}
            </div>
            <Button variant="default" className="approval-close-btn" onClick={onClose}>
              {t('approval.close')}
            </Button>
          </div>
        ) : (
          <>
            <div className="compact-header compact-header--stacked">
              <div className="approval-eyebrow">{t('approval.title')}</div>
              <div className="approval-title">{title}</div>
            </div>
            <div className="approval-desc">{t('approval.desc')}</div>
            <div className="approval-content">{content}</div>
            {tenetCount !== undefined && (
              <div className="approval-tenet">
                <span className="approval-tenet-dot">●</span>
                <span>{t('approval.count', String(tenetCount))}</span>
              </div>
            )}
            <div className="approval-actions">
              <Button
                variant="default"
                onClick={handleReject}
                disabled={busy}
              >
                {t('approval.reject')}
              </Button>
              <Button
                variant="primary"
                onClick={handleApprove}
                disabled={busy}
                loading={busy}
              >
                {busy ? t('common.processing') : t('approval.approve')}
              </Button>
            </div>
            <div className="approval-hints">
              <span>
                <kbd className="kbd">↵</kbd> {t('approval.hintApprove')}
              </span>
              <span>
                <kbd className="kbd">Esc</kbd> {t('approval.hintReject')}
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  )

  return createPortal(modalContent, document.body)
}