import { useState, useEffect, useCallback, useRef } from 'react'
import { createPortal } from 'react-dom'
import { invoke } from '../../core/bridge'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { playPopupSound } from '../../ui/sound'
import { IconCheck, IconX } from '../../ui/Icons'
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
  const desktop = kind === 'desktop_action'
  const [visible, setVisible] = useState(false)
  const [animating, setAnimating] = useState(false)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<'approved' | 'rejected' | null>(null)
  const activeAction = useRef(actionId)
  activeAction.current = actionId

  useEffect(() => {
    if (open) {
      setResult(null)
      setBusy(false)
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
  }, [open, actionId])

  // Desktop requests expire after 5 minutes; tenet proposals after 10 minutes.
  useEffect(() => {
    if (!open) return
    const timer = setTimeout(
      () => {
        onClose()
      },
      desktop ? 300_000 : 600_000,
    )
    return () => clearTimeout(timer)
  }, [open, onClose, desktop, actionId])

  const handleApprove = useCallback(async () => {
    if (busy) return
    setBusy(true)
    try {
      await invoke('approve_pending', { actionId })
      if (activeAction.current === actionId) setResult('approved')
    } catch (e) {
      // 审批项已过期/不存在（后端 TTL 600s 清理）→ 弹窗已无意义，直接关闭避免卡死
      console.error('Approve failed:', e)
      if (activeAction.current === actionId) onClose()
    } finally {
      if (activeAction.current === actionId) setBusy(false)
    }
  }, [actionId, busy, onClose])

  const handleReject = useCallback(async () => {
    if (busy) return
    setBusy(true)
    try {
      await invoke('reject_pending', { actionId })
      if (activeAction.current === actionId) setResult('rejected')
    } catch (e) {
      console.error('Reject failed:', e)
      if (activeAction.current === actionId) onClose()
    } finally {
      if (activeAction.current === actionId) setBusy(false)
    }
  }, [actionId, busy, onClose])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (busy || result) return
      // A desktop approval may appear while the user is typing in another app.
      // Require an intentional button activation; do not treat an incidental
      // Enter key as approval of an irreversible action.
      if (e.key === 'Enter' && !desktop) {
        e.preventDefault()
        handleApprove()
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        handleReject()
      }
    },
    [busy, result, desktop, handleApprove, handleReject],
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
        style={{
          transform: animating ? 'none' : undefined,
          opacity: animating ? 1 : undefined,
        }} /* 动态：入场动画状态 */
      >
        {result ? (
          <div className="approval-result">
            <div
              className={`approval-result-icon ${
                result === 'approved' ? 'approval-result-icon--ok' : 'approval-result-icon--err'
              }`}
            >
              {result === 'approved' ? <IconCheck size={22} /> : <IconX size={22} />}
            </div>
            <div className="approval-result-title">
              {result === 'approved'
                ? t(desktop ? 'approval.desktopApproved' : 'approval.saved')
                : t('approval.rejected')}
            </div>
            <div className="approval-result-desc">
              {result === 'approved'
                ? t(desktop ? 'approval.desktopApprovedDesc' : 'approval.savedDesc')
                : t(desktop ? 'approval.desktopRejectedDesc' : 'approval.rejectedDesc')}
            </div>
            <Button variant="default" className="approval-close-btn" onClick={onClose}>
              {t('approval.close')}
            </Button>
          </div>
        ) : (
          <>
            <div className="compact-header compact-header--stacked">
              <div className="approval-eyebrow">
                {t(desktop ? 'approval.desktopTitle' : 'approval.title')}
              </div>
              <div className="approval-title">{title}</div>
            </div>
            <div className="approval-desc">
              {t(desktop ? 'approval.desktopDesc' : 'approval.desc')}
            </div>
            <div className="approval-content">{content}</div>
            {!desktop && tenetCount !== undefined && (
              <div className="approval-tenet">
                <span className="approval-tenet-dot" aria-hidden="true" />
                <span>{t('approval.count', String(tenetCount))}</span>
              </div>
            )}
            <div className="approval-actions">
              <Button variant="default" onClick={handleReject} disabled={busy}>
                {t('approval.reject')}
              </Button>
              <Button variant="primary" onClick={handleApprove} disabled={busy} loading={busy}>
                {busy
                  ? t('common.processing')
                  : t(desktop ? 'approval.desktopApprove' : 'approval.approve')}
              </Button>
            </div>
            <div className="approval-hints">
              {!desktop && (
                <span>
                  <kbd className="kbd">↵</kbd> {t('approval.hintApprove')}
                </span>
              )}
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
