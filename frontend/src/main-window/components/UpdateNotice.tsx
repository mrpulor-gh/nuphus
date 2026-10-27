import { useState } from 'react'
import type { AppUpdate } from '../hooks/useAppUpdate'
import { UpdatePreviewModal } from './UpdatePreviewModal'
import { useLanguage } from '../../locales'
import { Button } from '../../ui/Button'
import { IconRefresh } from '../../ui/Icons'
import '../../styles/update.css'

/** 顶部常驻提示不抢焦点、不遮挡对话，下载安装必须由用户明确点击。 */
export function UpdateNotice({ updater, onOpen }: { updater: AppUpdate; onOpen: () => void }) {
  const { t } = useLanguage()
  const [previewOpen, setPreviewOpen] = useState(false)
  const { status, update, progress, noticeDismissed, downloaded, installBlocked, installGuard } =
    updater
  const downloading = status === 'downloading'
  if (
    !update ||
    (!downloading && noticeDismissed) ||
    !['available', 'downloading', 'error'].includes(status)
  )
    return null

  return (
    <aside className="update-notice" aria-label={t('app.update')}>
      <span className="update-notice-icon" aria-hidden="true">
        <IconRefresh size={18} />
      </span>
      <div className="update-notice-copy" role="status" aria-live="polite">
        <strong>{t('update.available', update.version)}</strong>
        <span>
          {downloading
            ? t('update.downloading', progress ? `${progress.toFixed(0)}%` : '…')
            : status === 'error'
              ? t('update.noticeFailed')
              : installGuard
                ? t(installGuard === 'busy' ? 'update.waitForIdle' : 'update.stateUnavailable')
                : downloaded
                  ? t('update.downloadReady')
                  : t('update.noticeHint')}
        </span>
      </div>
      <div className="update-notice-actions">
        <Button
          variant="primary"
          size="sm"
          disabled={status === 'available' && installBlocked}
          onClick={() => {
            onOpen()
            if (status === 'available') void updater.install()
          }}
        >
          {status === 'available'
            ? t(downloaded ? 'update.installRestart' : 'update.downloadInstall')
            : t('update.viewDetails')}
        </Button>
        {status === 'available' && (
          <Button variant="ghost" size="sm" onClick={() => setPreviewOpen(true)}>
            {t('update.previewButton')}
          </Button>
        )}
        {!downloading && (
          <Button variant="ghost" size="sm" onClick={updater.dismissNotice}>
            {t('update.later')}
          </Button>
        )}
      </div>
      <UpdatePreviewModal
        open={previewOpen}
        onClose={() => setPreviewOpen(false)}
        updater={updater}
      />
    </aside>
  )
}
