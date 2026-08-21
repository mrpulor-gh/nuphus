import { useState, useEffect } from 'react'
import { getToolPermissions, setToolPermissions } from '../lib/api'
import { useLanguage } from '../../locales'
import { Section, FormRow } from '../../ui/PageLayout'

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
  }, [])

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
    </div>
  )
}
