import { useMemo, useState, type FormEvent } from 'react'
import { t } from '../i18n'
import {
  getCurrentMobileServerUrl,
  getPreferredMobileServerUrl,
  prepareMobileServerNavigation,
} from '../serverConfig'

interface Props {
  /** 网络设置中默认展开；配对页/错误页默认收起，避免打断主操作。 */
  defaultExpanded?: boolean
  onNavigated?: () => void
}

export default function ServerAddressControl({ defaultExpanded = false, onNavigated }: Props) {
  const currentUrl = useMemo(() => getCurrentMobileServerUrl(), [])
  const [expanded, setExpanded] = useState(defaultExpanded)
  const [value, setValue] = useState(() => getPreferredMobileServerUrl())
  const [error, setError] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<string | null>(null)

  const submit = (event: FormEvent) => {
    event.preventDefault()
    setError(null)
    setFeedback(null)
    try {
      const prepared = prepareMobileServerNavigation(value)
      if (!prepared.changed) {
        setFeedback(t('mobile.serverAddressSame'))
        return
      }
      onNavigated?.()
      window.location.assign(prepared.url)
    } catch (e) {
      setError(
        e instanceof Error && e.message === 'empty'
          ? t('mobile.serverAddressEmpty')
          : t('mobile.serverAddressInvalid'),
      )
    }
  }

  return (
    <div className="mobile-server-config">
      <button
        type="button"
        className="mobile-server-config-toggle"
        aria-expanded={expanded}
        onClick={() => setExpanded(v => !v)}
      >
        <span>{t('mobile.serverAddressTitle')}</span>
        <code title={currentUrl}>{currentUrl}</code>
        <span className="mobile-server-config-action">
          {expanded ? t('mobile.serverAddressCollapse') : t('mobile.serverAddressExpand')}
        </span>
      </button>
      {expanded && (
        <form className="mobile-server-config-body" onSubmit={submit}>
          <label htmlFor="mobile-server-address">{t('mobile.serverAddressLabel')}</label>
          <div className="mobile-server-config-row">
            <input
              id="mobile-server-address"
              type="text"
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              value={value}
              onChange={e => {
                setValue(e.target.value)
                setError(null)
                setFeedback(null)
              }}
              placeholder={t('mobile.serverAddressPlaceholder')}
              aria-label={t('mobile.serverAddressLabel')}
            />
            <button type="submit" disabled={!value.trim()}>
              {t('mobile.serverAddressOpen')}
            </button>
          </div>
          <p>{t('mobile.serverAddressHint')}</p>
          {error && <div className="is-error">{error}</div>}
          {feedback && <div className="is-ok">{feedback}</div>}
        </form>
      )}
    </div>
  )
}
