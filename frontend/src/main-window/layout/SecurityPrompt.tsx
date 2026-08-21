import { useState, useEffect, useCallback } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { createPortal } from 'react-dom'
import { approveOnceSecurity, approveSessionSecurity, rejectSecurity } from '../lib/api'
import { IconX, IconShield } from '../../ui/Icons'
import { IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'

interface SecurityPromptProps {
  tool: string
  risk: 'low' | 'medium' | 'high' | 'critical'
  reason: string
  actionId: string
  onApprove: (id: string) => void
  onReject: (id: string) => void
}

const RISK_CONFIG: Record<string, { color: string; bg: string }> = {
  low: { color: '#22c55e', bg: 'rgba(34, 197, 94, 0.1)' },
  medium: { color: '#f59e0b', bg: 'rgba(245, 158, 11, 0.1)' },
  high: { color: '#ef4444', bg: 'rgba(239, 68, 68, 0.1)' },
  critical: { color: '#ef4444', bg: 'rgba(239, 68, 68, 0.15)' },
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
          await approveOnceSecurity(actionId)
          onApprove(actionId)
          break
        case 'session':
          await approveSessionSecurity(actionId, tool)
          onApprove(actionId)
          break
        case 'deny':
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
          {/* Risk indicator + tool */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
            <span
              style={{
                padding: '2px 8px',
                borderRadius: 4,
                fontSize: 10,
                fontWeight: 600,
                textTransform: 'uppercase',
                color: config.color,
                background: config.bg,
              }}
            >
              {t(`security.${risk}`)}
            </span>
            <span
              style={{
                fontSize: 12,
                color: 'var(--spark-primary)',
                fontFamily: 'var(--font-mono)',
              }}
            >
              {tool}
            </span>
          </div>

          {/* Reason */}
          <div
            style={{
              fontSize: 13,
              color: 'var(--spark-secondary)',
              lineHeight: 1.5,
              marginBottom: 14,
            }}
          >
            {reason}
          </div>

          {/* Options */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 10 }}>
            {options.map((opt, idx) => (
              <div
                key={opt.id}
                className={`compact-row ${idx === selected ? 'selected' : ''}`}
                onClick={() => !busy && handleChoice(opt.id)}
                style={{
                  cursor: busy ? 'not-allowed' : 'pointer',
                  padding: '8px 10px',
                  borderRadius: 8,
                  border: 'none',
                  opacity: busy ? 0.5 : 1,
                  background: idx === selected ? 'var(--void-hover)' : 'transparent',
                }}
              >
                <span
                  style={{
                    fontSize: 12,
                    color: 'var(--accent)',
                    width: 14,
                    flexShrink: 0,
                    fontFamily: 'var(--font-mono)',
                  }}
                >
                  {idx === selected ? '▸' : ' '}
                </span>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 13, color: 'var(--spark-primary)', fontWeight: 500 }}>
                    {t(opt.textKey)}
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--spark-muted)' }}>{t(opt.descKey)}</div>
                </div>
              </div>
            ))}
          </div>

          {/* Shortcuts */}
          <div
            style={{
              display: 'flex',
              gap: 12,
              fontSize: 10,
              color: 'var(--spark-dim)',
              fontFamily: 'var(--font-mono)',
            }}
          >
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
