import { useCallback, useEffect, useState } from 'react'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { useLanguage } from '../../locales'
import { Button } from '../../ui/Button'
import { IconRefresh } from '../../ui/Icons'
import '../../styles/update.css'

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

  useEffect(() => {
    void getVersion()
      .then(setCurrentVersion)
      .catch(() => {})
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
    </div>
  )
}
