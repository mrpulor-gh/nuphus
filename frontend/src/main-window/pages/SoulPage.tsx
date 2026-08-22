import { useState } from 'react'
import { loadRelation, saveRelation } from '../lib/relation'
import { setRelation } from '../lib/api'
import { Button } from '../../ui/Button'
import { Section, FormRow } from '../../ui/PageLayout'
import { useLanguage } from '../../locales'

export function SoulPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [rel, setRel] = useState(() => loadRelation())
  const [saved, setSaved] = useState(false)

  const handleSave = () => {
    saveRelation(rel)
    // 同步持久化到后端 relation.json（手机端 /identity 经 relation_cache 下发显示名）
    void setRelation(rel).catch(() => {})
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const update = (key: 'assistantName' | 'userLabel', val: string) => {
    setRel(prev => ({ ...prev, [key]: val }))
    setSaved(false)
  }

  return (
    <div>
      <Section title={t('app.soul')}>
        <FormRow
          stacked
          label={t('soul.aiLabel')}
          control={
            <input
              className="input"
              value={rel.assistantName}
              onChange={e => update('assistantName', e.target.value)}
              placeholder="Nuphus"
            />
          }
        />
        <FormRow
          stacked
          label={t('soul.userLabel')}
          control={
            <input
              className="input"
              value={rel.userLabel}
              onChange={e => update('userLabel', e.target.value)}
              placeholder="USER"
            />
          }
        />
        <div className="form-footer">
          {saved && <span className="badge badge-success">{t('common.saved')}</span>}
          <Button variant="primary" onClick={handleSave}>
            {t('common.save')}
          </Button>
        </div>
      </Section>
    </div>
  )
}
