import { useState, useEffect } from 'react'
import {
  getMacosPermissionStatus,
  getToolPermissions,
  openMacosPermissionSettings,
  requestMacosPermission,
  setToolPermissions,
  type MacosPermissionItem,
  type MacosPermissionReport,
} from '../lib/api'
import { useLanguage } from '../../locales'
import { Section, FormRow } from '../../ui/PageLayout'
import '../../styles/macos-permissions.css'

const TOOLS = [
  {
    id: 'file_access',
    labelKey: 'security.tool.fileAccess',
    descKey: 'security.tool.fileAccessDesc',
  },
  { id: 'web_search', labelKey: 'security.tool.webSearch', descKey: 'security.tool.webSearchDesc' },
  {
    id: 'system_automation',
    labelKey: 'security.tool.systemAutomation',
    descKey: 'security.tool.systemAutomationDesc',
  },
]

export function SecurityPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [perm, setPerm] = useState<Record<string, boolean>>({
    file_access: true,
    web_search: true,
    system_automation: false,
  })
  const [loading, setLoading] = useState(true)
  const [macosReport, setMacosReport] = useState<MacosPermissionReport | null>(null)
  const [macosLoading, setMacosLoading] = useState(true)
  const [macosAction, setMacosAction] = useState<string | null>(null)

  const refreshMacosPermissions = async () => {
    setMacosLoading(true)
    try {
      setMacosReport(await getMacosPermissionStatus())
    } catch {
      setMacosReport(null)
    } finally {
      setMacosLoading(false)
    }
  }

  useEffect(() => {
    getToolPermissions()
      .then(r => {
        if (r && typeof r === 'string') {
          try {
            const data = JSON.parse(r)
            if (typeof data === 'object' && data !== null) {
              setPerm({
                file_access: data.file_access ?? data.fileAccess ?? true,
                web_search: data.web_search ?? data.webSearch ?? true,
                system_automation: data.system_automation ?? data.systemAutomation ?? false,
              })
            }
          } catch {
            /* ignore */
          }
        }
        setLoading(false)
      })
      .catch(() => setLoading(false))
    void refreshMacosPermissions()
  }, [])

  const authorizeMacosPermission = async (permission: MacosPermissionItem) => {
    setMacosAction(permission.id)
    try {
      if (permission.status === 'on_demand') {
        await openMacosPermissionSettings(permission.id)
      } else {
        await requestMacosPermission(permission.id)
      }
    } catch (e) {
      console.error('打开 macOS 权限设置失败:', e)
    } finally {
      setMacosAction(null)
    }
  }

  const toggle = async (id: string) => {
    const prev = { ...perm }
    const next = { ...perm, [id]: !perm[id] }
    setPerm(next)
    try {
      await setToolPermissions(
        next.file_access ?? false,
        next.web_search ?? false,
        next.system_automation ?? false,
      )
    } catch (e) {
      setPerm(prev) // 回滚乐观更新
      console.error('保存权限失败:', e)
    }
  }

  if (loading) return <div className="page-loading">{t('common.loading')}</div>

  return (
    <div>
      <Section title={t('security.tools')}>
        {TOOLS.map(tool => (
          <FormRow
            key={tool.id}
            label={t(tool.labelKey)}
            hint={t(tool.descKey)}
            control={
              <button
                type="button"
                role="switch"
                aria-checked={perm[tool.id] ?? false}
                className="switch"
                onClick={() => toggle(tool.id)}
              />
            }
          />
        ))}
      </Section>
      {macosReport?.platformSupported && (
        <Section title={t('macosPermission.sectionTitle')}>
          <div className="macos-permission-summary">
            <span>{t('macosPermission.sectionHint')}</span>
            <button
              type="button"
              className="macos-permission-refresh"
              onClick={() => void refreshMacosPermissions()}
              disabled={macosLoading}
            >
              {macosLoading ? t('common.loading') : t('macosPermission.refresh')}
            </button>
          </div>
          <div className="macos-permission-list">
            {macosReport.permissions.map(permission => (
              <div className="macos-permission-row" key={permission.id}>
                <div className="macos-permission-copy">
                  <div className="macos-permission-name">
                    {t(`macosPermission.${permission.id}`)}
                    <span className={`macos-permission-status is-${permission.status}`}>
                      {t(`macosPermission.status.${permission.status}`)}
                    </span>
                  </div>
                  <div className="macos-permission-description">
                    {t(`macosPermission.${permission.id}Desc`)}
                  </div>
                </div>
                {permission.status !== 'granted' && (
                  <button
                    type="button"
                    className="macos-permission-action"
                    disabled={macosAction === permission.id}
                    onClick={() => void authorizeMacosPermission(permission)}
                  >
                    {permission.status === 'on_demand'
                      ? t('macosPermission.openSettings')
                      : t('macosPermission.authorize')}
                  </button>
                )}
              </div>
            ))}
          </div>
        </Section>
      )}
    </div>
  )
}
