import { useState, useEffect } from 'react'
import {
  setBrowserCdpUrl,
  getBrowserConnection,
  testBrowserCdpUrl,
  detectCdpBrowsers,
  type BrowserConnection,
  type DetectedBrowser,
} from '../lib/api'
import { Button } from '../../ui/Button'
import { Section } from '../../ui/PageLayout'
import { useLanguage } from '../../locales'

type Mode = 'managed' | 'external'
type Probe = 'idle' | 'probing' | 'ok' | 'fail'

/** Display fallback when no identity name is persisted: host:port of the URL. */
function hostPort(url: string): string {
  return url.replace(/^https?:\/\//, '').replace(/\/+$/, '')
}

/**
 * Browser execution environment.
 * - Managed: Nuphus launches and manages its own browser (zero config).
 * - External (fingerprint browser): the user opens a window in their
 *   fingerprint-browser platform, taps "detect", and picks the detected
 *   window — no addresses, ports, or manual testing are ever exposed.
 * Picking an option applies and persists immediately.
 */
export function BrowserPage({ onClose: _onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [mode, setMode] = useState<Mode>('managed')
  const [savedUrl, setSavedUrl] = useState('')
  const [conn, setConn] = useState<BrowserConnection | null>(null)
  const [probe, setProbe] = useState<Probe>('idle')
  const [probeDetail, setProbeDetail] = useState('')
  const [loading, setLoading] = useState(true)
  const [detecting, setDetecting] = useState(false)
  const [detected, setDetected] = useState<DetectedBrowser[] | null>(null)
  const [applying, setApplying] = useState(false)
  const [hoverUrl, setHoverUrl] = useState('')
  const [notice, setNotice] = useState('')
  const [noticeOk, setNoticeOk] = useState(true)

  // Liveness probe of the persisted endpoint (no_proxy GET /json/version).
  // Failure is not an error state to fix here — the agent side self-heals once
  // the window is reopened; the card just tells the user what it sees.
  const probeLiveness = (url: string) => {
    setProbe('probing')
    setProbeDetail('')
    testBrowserCdpUrl(url)
      .then(msg => {
        setProbe('ok')
        setProbeDetail((msg ?? '').replace(/^已连接：/, ''))
      })
      .catch(() => setProbe('fail'))
  }

  useEffect(() => {
    getBrowserConnection()
      .then(c => {
        const v = c?.url ?? ''
        setSavedUrl(v)
        setConn(c)
        setMode(v ? 'external' : 'managed')
        setLoading(false)
        if (v) probeLiveness(v)
      })
      .catch(() => setLoading(false))
  }, [])

  const flash = (msg: string, ok: boolean) => {
    setNotice(msg)
    setNoticeOk(ok)
  }

  const applyManaged = async () => {
    if (mode === 'managed' && !savedUrl) return
    setMode('managed')
    setDetected(null)
    if (!savedUrl) return
    setApplying(true)
    try {
      await setBrowserCdpUrl('')
      setSavedUrl('')
      setConn(null)
      setProbe('idle')
      flash(t('browser.managedActive'), true)
    } catch (e) {
      flash(String(e), false)
    }
    setApplying(false)
  }

  const handleDetect = async () => {
    setDetecting(true)
    setDetected(null)
    setNotice('')
    try {
      setDetected(await detectCdpBrowsers())
    } catch (e) {
      setDetected([])
      flash(String(e), false)
    }
    setDetecting(false)
  }

  const handlePick = async (b: DetectedBrowser) => {
    if (savedUrl === b.url) return
    setApplying(true)
    try {
      const identity = {
        name: b.name,
        exe_path: b.exe_path,
        user_data_dir: b.user_data_dir ?? null,
      }
      await setBrowserCdpUrl(b.url, identity)
      setSavedUrl(b.url)
      setConn({ url: b.url, ...identity })
      flash(t('browser.picked', b.name), true)
      probeLiveness(b.url)
    } catch (e) {
      flash(String(e), false)
    }
    setApplying(false)
  }

  const switchExternal = () => {
    setMode('external')
    if (savedUrl) probeLiveness(savedUrl)
  }

  if (loading) return null

  return (
    <div>
      <Section title={t('browser.envTitle')} description={t('browser.envDesc')}>
        <div className="btn-row">
          <Button
            variant={mode === 'managed' ? 'primary' : 'default'}
            onClick={applyManaged}
            loading={applying && mode === 'managed'}
          >
            {t('browser.modeManaged')}
          </Button>
          <Button variant={mode === 'external' ? 'primary' : 'default'} onClick={switchExternal}>
            {t('browser.modeExternal')}
          </Button>
        </div>
        {notice && (
          <div
            style={{
              marginTop: 8,
              fontSize: 'var(--fs-caption)',
              color: noticeOk ? 'var(--success)' : 'var(--error)',
            }}
          >
            {notice}
          </div>
        )}
      </Section>

      {mode === 'external' && (
        <Section title={t('browser.connectTitle')}>
          {savedUrl && (
            <div
              style={{
                marginBottom: 10,
                padding: '8px 10px',
                borderRadius: 'var(--radius-md)',
                border: '1px solid var(--accent)',
                background: 'var(--surface-2)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
              }}
            >
              <div style={{ minWidth: 0 }}>
                <div
                  style={{
                    fontSize: 'var(--fz-sm)',
                    color: 'var(--fg-1)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  <span style={{ color: 'var(--fg-3)', marginRight: 8, fontSize: 'var(--fz-xs)' }}>
                    {t('browser.current')}
                  </span>
                  {conn?.name || hostPort(savedUrl)}
                </div>
                <div
                  style={{
                    fontSize: 'var(--fz-xs)',
                    color: 'var(--fg-3)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                >
                  {savedUrl}
                </div>
                <div
                  style={{
                    fontSize: 'var(--fz-xs)',
                    marginTop: 2,
                    color:
                      probe === 'fail'
                        ? 'var(--error)'
                        : probe === 'ok'
                          ? 'var(--success)'
                          : 'var(--fg-3)',
                  }}
                >
                  {probe === 'probing' && t('browser.statusProbing')}
                  {probe === 'ok' && t('browser.statusConnected', probeDetail)}
                  {probe === 'fail' && t('browser.statusUnreachable')}
                </div>
              </div>
              <span className="badge badge-success">{t('browser.inUse')}</span>
            </div>
          )}
          <div style={{ fontSize: 'var(--fs-caption)', lineHeight: 1.8, color: 'var(--fg-2)' }}>
            <div>{t('browser.guideStep1')}</div>
            <div>{t('browser.guideStep2')}</div>
          </div>
          <div style={{ marginTop: 10 }}>
            <Button variant="default" onClick={handleDetect} loading={detecting}>
              {t('browser.detect')}
            </Button>
          </div>

          {detected !== null && detected.length === 0 && (
            <div
              style={{
                marginTop: 10,
                fontSize: 'var(--fs-caption)',
                lineHeight: 1.7,
                color: 'var(--fg-2)',
              }}
            >
              {t('browser.detectEmpty')}
            </div>
          )}

          {detected && detected.length > 0 && (
            <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
              {detected.map(b => {
                const active = savedUrl === b.url
                const hovered = hoverUrl === b.url
                return (
                  <div
                    key={b.url}
                    role="button"
                    tabIndex={0}
                    onClick={() => !active && handlePick(b)}
                    onKeyDown={e => {
                      if (e.key === 'Enter' && !active) handlePick(b)
                    }}
                    onMouseEnter={() => setHoverUrl(b.url)}
                    onMouseLeave={() => setHoverUrl('')}
                    style={{
                      textAlign: 'left',
                      padding: '8px 10px',
                      borderRadius: 'var(--radius-md)',
                      border: `1px solid ${active ? 'var(--accent)' : hovered ? 'var(--accent)' : 'var(--line-1)'}`,
                      background: active
                        ? 'var(--surface-2)'
                        : hovered
                          ? 'var(--bg-hover)'
                          : 'var(--surface-2)',
                      cursor: active ? 'default' : 'pointer',
                      color: 'var(--fg-1)',
                      transition: 'background 120ms ease, border-color 120ms ease',
                    }}
                  >
                    <div
                      style={{
                        display: 'flex',
                        alignItems: 'center',
                        justifyContent: 'space-between',
                        gap: 8,
                      }}
                    >
                      <span style={{ fontSize: 'var(--fz-sm)', fontWeight: 'var(--fw-semibold)' }}>
                        {b.name}
                        <span
                          style={{
                            marginLeft: 8,
                            fontWeight: 'normal',
                            color: 'var(--fg-3)',
                            fontSize: 'var(--fz-xs)',
                          }}
                        >
                          {b.version}
                        </span>
                      </span>
                      {active ? (
                        <span className="badge badge-success">{t('browser.inUse')}</span>
                      ) : (
                        <Button
                          variant="primary"
                          size="sm"
                          disabled={applying}
                          onClick={e => {
                            e.stopPropagation()
                            handlePick(b)
                          }}
                        >
                          {t('browser.use')}
                        </Button>
                      )}
                    </div>
                    {b.pages.length > 0 && (
                      <div
                        style={{
                          marginTop: 4,
                          fontSize: 'var(--fz-xs)',
                          color: 'var(--fg-3)',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {b.pages.slice(0, 2).join(' · ')}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )}
        </Section>
      )}
    </div>
  )
}
