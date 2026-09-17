import { useCallback, useEffect, useMemo, useState } from 'react'
import { IconShield, IconX } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import {
  getMacosPermissionStatus,
  type MacosPermissionItem,
  type MacosPermissionReport,
} from '../lib/api'
import '../../styles/macos-permissions.css'

interface MacosPermissionNoticeProps {
  mode: string
  onOpenSettings: () => void
}

export function MacosPermissionNotice({ mode, onOpenSettings }: MacosPermissionNoticeProps) {
  const { t } = useLanguage()
  const [report, setReport] = useState<MacosPermissionReport | null>(null)
  const [dismissed, setDismissed] = useState(false)

  const refresh = useCallback(() => {
    let active = true
    void getMacosPermissionStatus()
      .then(next => {
        if (active && next) setReport(next)
      })
      .catch(() => {
        if (active) setReport(null)
      })
    return () => {
      active = false
    }
  }, [])

  useEffect(() => {
    if (mode !== 'workflow') return
    let cancelRequest = refresh()
    const handleFocus = () => {
      cancelRequest()
      cancelRequest = refresh()
    }
    const handleVisibility = () => {
      if (document.visibilityState === 'visible') handleFocus()
    }
    window.addEventListener('focus', handleFocus)
    document.addEventListener('visibilitychange', handleVisibility)
    return () => {
      cancelRequest()
      window.removeEventListener('focus', handleFocus)
      document.removeEventListener('visibilitychange', handleVisibility)
    }
  }, [mode, refresh])

  const missing = useMemo(
    () =>
      report?.permissions.filter(
        permission => permission.requiredForWorkflow && permission.status === 'missing',
      ) ?? [],
    [report],
  )

  if (mode !== 'workflow' || dismissed || !report?.platformSupported || missing.length === 0) {
    return null
  }

  const names = missing.map((item: MacosPermissionItem) => t(`macosPermission.${item.id}`))

  return (
    <div className="macos-permission-notice" role="status">
      <button
        type="button"
        className="macos-permission-notice-main"
        onClick={() => {
          setDismissed(true)
          onOpenSettings()
        }}
      >
        <IconShield size={16} />
        <span>
          <strong>{t('macosPermission.noticeTitle')}</strong>
          <small>{t('macosPermission.noticeMissing', names.join('、'))}</small>
        </span>
      </button>
      <button
        type="button"
        className="macos-permission-notice-close"
        aria-label={t('macosPermission.dismiss')}
        onClick={() => setDismissed(true)}
      >
        <IconX size={12} />
      </button>
    </div>
  )
}
