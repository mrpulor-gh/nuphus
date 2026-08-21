import { type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useLanguage } from '../../locales'

interface PauseOverlayProps {
  /** 暂停状态（null 表示未暂停，不渲染） */
  pauseState: { actionId: string } | null
  /** 'menu' | 'preparing' | 'input' */
  pauseMode: string
  pauseActionBusy: boolean
  selectedOption: number
  appendInput: string
  pauseSubmitting: boolean
  onPauseChoice: (choice: string) => void
  onAppendInputChange: (value: string) => void
  onBackToMenu: () => void
  onSubmitAppend: () => void
}

export function PauseOverlay({
  pauseState,
  pauseMode,
  pauseActionBusy,
  selectedOption,
  appendInput,
  pauseSubmitting,
  onPauseChoice,
  onAppendInputChange,
  onBackToMenu,
  onSubmitAppend,
}: PauseOverlayProps): ReactNode | null {
  const { t } = useLanguage()

  if (!pauseState) return null

  return createPortal(
    <div
      className="compact-overlay"
      style={{ zIndex: 210 }}
      onClick={() => pauseMode === 'preparing' && onBackToMenu()}
    >
      <div
        className="compact-modal compact-modal--sm compact-modal--fit"
        onClick={e => e.stopPropagation()}
      >
        {pauseMode === 'menu' || pauseMode === 'preparing' ? (
          <>
            <div className="compact-header">
              <span className="compact-header-title">{t('pause.title')}</span>
            </div>
            <div className="compact-divider" />
            <div className="compact-body">
              <div
                style={{
                  fontSize: 13,
                  color: 'var(--spark-secondary)',
                  lineHeight: 1.5,
                  marginBottom: 14,
                }}
              >
                {t('pause.desc')}
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 10 }}>
                {[
                  { id: 'continue', textKey: 'pause.continue', descKey: 'pause.continueDesc' },
                  {
                    id: 'append',
                    textKey: pauseMode === 'preparing' ? 'pause.appendPreparing' : 'pause.append',
                    descKey: pauseMode === 'preparing' ? 'pause.appendWaiting' : 'pause.appendDesc',
                  },
                  { id: 'terminate', textKey: 'pause.terminate', descKey: 'pause.terminateDesc' },
                  { id: 'interrupt', textKey: 'pause.interrupt', descKey: 'pause.interruptDesc' },
                ].map((opt, idx) => {
                  const isPreparing = pauseMode === 'preparing' && opt.id === 'append'
                  const disabled = pauseActionBusy || isPreparing
                  return (
                    <div
                      key={opt.id}
                      onClick={() => (disabled ? null : onPauseChoice(opt.id))}
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        gap: 8,
                        padding: '8px 10px',
                        borderRadius: 8,
                        cursor: disabled ? 'default' : 'pointer',
                        background: selectedOption === idx ? 'var(--void-hover)' : 'transparent',
                        opacity: disabled ? 0.5 : 1,
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
                        {isPreparing
                          ? '⟳'
                          : pauseActionBusy
                            ? '◌'
                            : selectedOption === idx
                              ? '▸'
                              : ' '}
                      </span>
                      <div>
                        <div
                          style={{ fontSize: 13, color: 'var(--spark-primary)', fontWeight: 500 }}
                        >
                          {pauseActionBusy && opt.id !== 'append'
                            ? t('common.processing')
                            : t(opt.textKey)}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--spark-muted)' }}>
                          {isPreparing
                            ? t('pause.appendWaiting')
                            : pauseActionBusy
                              ? ''
                              : t(opt.descKey)}
                        </div>
                      </div>
                    </div>
                  )
                })}
              </div>
              <div
                style={{
                  display: 'flex',
                  gap: 12,
                  fontSize: 10,
                  color: 'var(--spark-dim)',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                <span>{t('common.hint.upDown')}</span>
                <span>{t('common.hint.enter')}</span>
                <span>{t('common.hint.esc')}</span>
              </div>
            </div>
          </>
        ) : (
          <>
            <div className="compact-header">
              <span className="compact-header-title">{t('pause.appendLabel')}</span>
            </div>
            <div className="compact-divider" />
            <div className="compact-body">
              <div
                style={{
                  fontSize: 13,
                  color: 'var(--spark-secondary)',
                  lineHeight: 1.5,
                  marginBottom: 10,
                }}
              >
                {t('pause.appendPlaceholder')}
              </div>
              <textarea
                style={{
                  width: '100%',
                  padding: '10px 12px',
                  borderRadius: 8,
                  border: '1px solid var(--glass-2)',
                  background: 'var(--void-input)',
                  color: 'var(--spark-primary)',
                  fontSize: 13,
                  fontFamily: 'var(--font-ui)',
                  resize: 'vertical',
                  outline: 'none',
                  boxSizing: 'border-box',
                  transition: 'border-color .15s ease',
                }}
                placeholder={t('pause.appendInput')}
                value={appendInput}
                onChange={e => onAppendInputChange(e.target.value)}
                rows={3}
                autoFocus
              />
              <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
                <button
                  onClick={onSubmitAppend}
                  disabled={pauseSubmitting || !appendInput.trim()}
                  style={{
                    padding: '6px 16px',
                    borderRadius: 8,
                    border: 'none',
                    background:
                      pauseSubmitting || !appendInput.trim() ? 'var(--glass-2)' : 'var(--accent)',
                    color: pauseSubmitting || !appendInput.trim() ? 'var(--spark-dim)' : '#fff',
                    fontSize: 13,
                    cursor: pauseSubmitting || !appendInput.trim() ? 'default' : 'pointer',
                    fontWeight: 500,
                  }}
                >
                  {pauseSubmitting ? t('common.processing') : t('pause.appendSubmit')}
                </button>
                <button
                  onClick={onBackToMenu}
                  style={{
                    padding: '6px 16px',
                    borderRadius: 8,
                    border: '1px solid var(--glass-2)',
                    background: 'transparent',
                    color: 'var(--spark-secondary)',
                    fontSize: 13,
                    cursor: 'pointer',
                  }}
                >
                  {t('common.cancel')}
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>,
    document.body,
  )
}