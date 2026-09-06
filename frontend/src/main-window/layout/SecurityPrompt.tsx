import { useState, useEffect, useCallback } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { createPortal } from 'react-dom'
import { approveOnceSecurity, approveSessionSecurity, rejectSecurity } from '../lib/api'
import { IconX, IconShield } from '../../ui/Icons'
import { IconButton } from '../../ui/Button'
import { playPopupSound, playUiSound } from '../../ui/sound'
import { useLanguage } from '../../locales'
import '../../styles/security.css'

interface SecurityPromptProps {
  tool: string
  risk: 'low' | 'medium' | 'high' | 'critical'
  reason: string
  actionId: string
  onApprove: (id: string) => void
  onReject: (id: string) => void
}

const RISK_CONFIG: Record<string, { color: string; bg: string }> = {
  low: { color: 'var(--status-success)', bg: 'var(--status-success-soft)' },
  medium: { color: 'var(--status-warning)', bg: 'var(--status-warning-soft)' },
  high: { color: 'var(--status-error)', bg: 'var(--status-error-soft)' },
  critical: { color: 'var(--status-error)', bg: 'rgba(var(--error-rgb), 0.16)' },
}

export function SecurityPrompt({
  tool,
  risk,
  reason,
  actionId,
  onApprove,
  onReject,
}: SecurityPromptProps) {
  const { t } = useLanguage()
  const [selected, setSelected] = useState(0)
  const [busy, setBusy] = useState(false)
  const config = RISK_CONFIG[risk] || RISK_CONFIG.medium

  const options = [
    { id: 'once', textKey: 'security.allowOnce', descKey: 'security.allowOnceDesc' },
    { id: 'session', textKey: 'security.allowSession', descKey: 'security.allowSessionDesc' },
    { id: 'deny', textKey: 'security.deny', descKey: 'security.denyDesc' },
  ]

  const handleChoice = useCallback(
    async (choice: string) => {
      if (busy) return
      setBusy(true)
      switch (choice) {
        case 'once':
          playUiSound('send')
          await approveOnceSecurity(actionId)
          onApprove(actionId)
          break
        case 'session':
          playUiSound('send')
          await approveSessionSecurity(actionId, tool)
          onApprove(actionId)
          break
        case 'deny':
          playUiSound('deny')
          await rejectSecurity(actionId)
          onReject(actionId)
          break
      }
    },
    [actionId, tool, onApprove, onReject, busy],
  )

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (busy) return
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSelected(prev => (prev > 0 ? prev - 1 : options.length - 1))
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSelected(prev => (prev < options.length - 1 ? prev + 1 : 0))
      }
      if (e.key === 'Enter') {
        e.preventDefault()
        handleChoice(options[selected].id)
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        handleChoice('deny')
      }
    },
    [selected, busy, handleChoice],
  )

  useEffect(() => {
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  // 5-minute timeout auto-deny (aligned with backend polling timeout)
  useEffect(() => {
    const timer = setTimeout(() => {
      if (!busy) {
        rejectSecurity(actionId)
        onReject(actionId)
      }
    }, 300000) // 5 minutes = 300000ms
    return () => clearTimeout(timer)
  }, [actionId, busy, onReject])

  useEffect(() => {
    getCurrentWindow()
      .setFocus()
      .catch(() => {})
  }, [])

  // 弹窗出现提示音：审批型打断（agent 请求权限，需用户决策）
  useEffect(() => {
    playPopupSound('approval')
  }, [])

  const promptContent = (
    <div className="compact-overlay" style={{ zIndex: 200 }}>
      <div
        className="compact-modal compact-modal--sm compact-modal--fit"
        onClick={e => e.stopPropagation()}
      >
        <div className="compact-header">
          <span className="compact-header-icon">
            <IconShield size={14} />
          </span>
          <span className="compact-header-title">{t('security.title')}</span>
          {busy ? null : (
            <IconButton
              variant="compact-header-close"
              label={t('common.close')}
              onClick={() => handleChoice('deny')}
            >
              <IconX size={14} />
            </IconButton>
          )}
        </div>
        <div className="compact-divider" />
        <div className="compact-body">
          {/* Request: tool + risk badge（工具左、风险徽标右） */}
          <div className="security-request">
            <span className="security-tool">{tool}</span>
            <span className="security-risk" style={{ color: config.color, background: config.bg }}>
              {t(`security.${risk}`)}
            </span>
          </div>

          {/* Reason / evidence */}
          <div className="security-reason">{reason}</div>

          {/* Decision group: allow ×2 / deny（radio 单选行，危险色仅用于拒绝） */}
          <div className={`security-options${busy ? ' is-disabled' : ''}`}>
            {options.map((opt, idx) => (
              <div key={opt.id}>
                {opt.id === 'deny' && <div className="security-divider" role="presentation" />}
                <div
                  className={`security-choice ${opt.id === 'deny' ? 'security-choice--deny' : ''} ${
                    idx === selected ? 'is-selected' : ''
                  }`}
                  onClick={() => !busy && handleChoice(opt.id)}
                >
                  <span className="security-choice-marker" aria-hidden="true" />
                  <div className="security-choice-copy">
                    <div className="security-choice-label">{t(opt.textKey)}</div>
                    <div className="security-choice-desc">{t(opt.descKey)}</div>
                  </div>
                </div>
              </div>
            ))}
          </div>

          {/* Shortcuts */}
          <div className="security-hints">
            <span>{t('security.hintUpDown')}</span>
            <span>{t('security.hintEnter')}</span>
            <span>{t('security.hintEsc')}</span>
          </div>
        </div>
      </div>
    </div>
  )

  return createPortal(promptContent, document.body)
}
