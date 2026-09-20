/**
 * SessionGroupsPage.tsx — 会话工作台「项目文件夹分组」设置页（设置中心分区）
 *
 * 唯一设置项：**组内折叠上限**（全局单值，后端 `preferences.session_group_collapsed_limit`）。
 * - 读：`list_shelf_sessions` 返回体自带 `collapsed_limit` —— 后端 `session_group_limit()`
 *   是唯一读数入口（已把 0 收敛为默认值），无需为设置页新增后端命令；
 * - 写：`set_session_group_collapsed_limit`（后端显式拒绝 0，见 preferences.rs:532-535）；
 * - 生效：写入后 `setSessionGroupCollapsedLimit` 广播 SESSION_GROUP_LIMIT_CHANGED_EVENT，
 *   会话工作台监听后立即重绘（不必等下一轮 5s 轮询）。
 *
 * 版式复用 ProjectPage（Section + FormRow + compact-input-row + form-footer），不新造样式。
 */
import { useCallback, useEffect, useState } from 'react'
import { Button } from '../../ui/Button'
import { Section, FormRow } from '../../ui/PageLayout'
import { useLanguage } from '../../locales'
import { listShelfSessions, setSessionGroupCollapsedLimit } from '../lib/api'
import { DEFAULT_GROUP_LIMIT, normalizeGroupLimit } from '../chat/sessionGroups'

export function SessionGroupsPage() {
  const { t } = useLanguage()
  const [draft, setDraft] = useState(String(DEFAULT_GROUP_LIMIT))
  const [busy, setBusy] = useState(false)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const r = await listShelfSessions()
        if (cancelled) return
        setDraft(String(normalizeGroupLimit(r?.collapsed_limit)))
      } catch {
        if (!cancelled) setError(t('settings.sessionGroups.loadFail'))
      }
    })()
    return () => {
      cancelled = true
    }
  }, [t])

  const save = useCallback(async () => {
    const n = Number(draft.trim())
    // 后端拒绝 0（会让每个分组折叠成空列表）→ 前端先拦，避免把「正常拒绝」装成失败
    if (!Number.isInteger(n) || n < 1) {
      setError(t('settings.sessionGroups.limitInvalid'))
      return
    }
    setError(null)
    setBusy(true)
    try {
      const applied = normalizeGroupLimit(await setSessionGroupCollapsedLimit(n))
      setDraft(String(applied))
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch {
      setError(t('settings.sessionGroups.saveFail'))
    } finally {
      setBusy(false)
    }
  }, [draft, t])

  return (
    <div data-testid="page-session-groups">
      <Section
        title={t('app.sessionGroups')}
        description={t('settings.sessionGroups.limitHint', String(DEFAULT_GROUP_LIMIT))}
      >
        <FormRow
          label={t('settings.sessionGroups.limit')}
          control={
            <div className="compact-input-row">
              <input
                className="compact-input input-flex"
                type="number"
                min={1}
                max={99}
                value={draft}
                disabled={busy}
                onChange={e => {
                  setDraft(e.target.value)
                  setError(null)
                }}
                onKeyDown={e => {
                  if (e.key === 'Enter') void save()
                }}
              />
              <Button variant="default" size="sm" disabled={busy} onClick={() => void save()}>
                {t('common.save')}
              </Button>
            </div>
          }
        />
        {(saved || error) && (
          <div className="form-footer">
            {saved && <span className="badge badge-success">{t('common.saved')}</span>}
            {error && (
              <span style={{ color: 'var(--error)', fontSize: 'var(--fz-xs)' }}>{error}</span>
            )}
          </div>
        )}
      </Section>
    </div>
  )
}