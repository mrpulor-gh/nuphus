import { useCallback, useEffect, useState } from 'react'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { useLanguage } from '../../locales'
import { Button } from '../../ui/Button'
import { IconRefresh } from '../../ui/Icons'
import { getChangelog } from '../lib/api'
import { countSectionItems, parseChangelogSection, type ChangelogSection } from '../lib/changelog'
import '../../styles/update.css'

/** 本版更新内容区块的状态（四态互斥，避免多个 flag 互相打架） */
type ChangelogState =
  /** 版本号或 CHANGELOG 尚未就绪 */
  | { status: 'loading' }
  | { status: 'ready'; section: ChangelogSection }
  /** CHANGELOG 里没有当前版本段落（或段落为空） */
  | { status: 'empty' }
  /** 命令调用失败 */
  | { status: 'unavailable' }

export function UpdatePage() {
  const { t } = useLanguage()
  const [status, setStatus] = useState<
    'idle' | 'checking' | 'available' | 'latest' | 'downloading' | 'error'
  >('idle')
  const [update, setUpdate] = useState<Update | null>(null)
  const [progress, setProgress] = useState(0)
  const [error, setError] = useState('')
  const [, setDownloaded] = useState(0)
  const [currentVersion, setCurrentVersion] = useState('—')
  const [changelog, setChangelog] = useState<ChangelogState>({ status: 'loading' })

  useEffect(() => {
    let cancelled = false
    void (async () => {
      // 版本号同时驱动「当前版本」展示与本版段落定位；取不到就无从定位（保持空态而非伪版本号）
      let version = ''
      try {
        version = await getVersion()
      } catch {
        // 与既有行为一致：拿不到版本号时静默（界面仍显示占位符）
      }
      if (cancelled) return
      if (version) setCurrentVersion(version)

      try {
        const raw = await getChangelog()
        if (cancelled) return
        const section = version ? parseChangelogSection(raw, version) : null
        setChangelog(
          section && countSectionItems(section) > 0
            ? { status: 'ready', section }
            : { status: 'empty' },
        )
      } catch {
        if (!cancelled) setChangelog({ status: 'unavailable' })
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const checkVersion = useCallback(async () => {
    setStatus('checking')
    setError('')
    try {
      const found = await check()
      setUpdate(found)
      setStatus(found ? 'available' : 'latest')
    } catch {
      setError('暂时无法获取官方版本信息，请稍后重试。')
      setStatus('error')
    }
  }, [])

  const install = useCallback(async () => {
    if (!update) return
    setStatus('downloading')
    setProgress(0)
    setDownloaded(0)
    try {
      let contentLength: number | null = null
      let downloadedBytes = 0
      await update.download(event => {
        if (event.event === 'Started') {
          contentLength = event.data.contentLength ?? null
          // Content length is kept locally for progress calculation.
        } else if (event.event === 'Progress') {
          downloadedBytes += event.data.chunkLength
          setDownloaded(downloadedBytes)
          setProgress(contentLength ? Math.min(100, (downloadedBytes / contentLength) * 100) : 0)
        } else if (event.event === 'Finished') {
          setProgress(100)
        }
      })
      await update.install({ restartAfterInstall: true })
      await relaunch()
    } catch {
      setError('更新下载或安装未完成，请稍后重试。')
      setStatus('error')
    }
  }, [update])

  return (
    <div className="update-page">
      <div className="update-current">
        <span>{t('update.current')}</span>
        <strong>v{currentVersion}</strong>
      </div>
      {status === 'available' && update ? (
        <div className="update-available">
          <div className="update-version">{t('update.available', update.version)}</div>
          {update.body && <div className="update-notes">{update.body}</div>}
          <Button variant="primary" onClick={install}>
            {t('update.downloadInstall')}
          </Button>
        </div>
      ) : (
        <div className="update-status">
          {status === 'checking' && t('update.checking')}
          {status === 'latest' && t('update.latest')}
          {status === 'idle' && t('update.hint')}
          {status === 'error' && <span className="update-error">{t('update.failed', error)}</span>}
          {status === 'downloading' && (
            <div className="update-progress">
              <span>{t('update.downloading', progress ? `${progress.toFixed(0)}%` : '…')}</span>
              <progress max="100" value={progress} />
            </div>
          )}
        </div>
      )}
      {status !== 'downloading' && (
        <Button variant="default" onClick={checkVersion} disabled={status === 'checking'}>
          <IconRefresh size={14} />
          {status === 'error' ? t('update.retry') : t('update.check')}
        </Button>
      )}

      {/* ── 本版更新内容：当前版本在 CHANGELOG 里的段落（离线可读，不依赖网络）── */}
      <div className="update-changes">
        <div className="update-changes-title">{t('update.changesTitle')}</div>
        {changelog.status === 'loading' && (
          <div className="update-changes-empty">{t('common.loading')}</div>
        )}
        {changelog.status === 'empty' && (
          <div className="update-changes-empty">{t('update.changesEmpty')}</div>
        )}
        {changelog.status === 'unavailable' && (
          <div className="update-changes-empty">{t('update.changesUnavailable')}</div>
        )}
        {changelog.status === 'ready' && (
          <div className="update-changes-body">
            {changelog.section.groups.map(group => (
              <div className="update-changes-group" key={group.title}>
                {group.title && <div className="update-changes-group-title">{group.title}</div>}
                <ul className="update-changes-list">
                  {group.items.map((item, index) => (
                    <li key={`${group.title}-${index}`}>{item}</li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
