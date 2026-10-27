import { CompactModal } from '../layout/CompactModal'
import { MarkdownInline } from '../chat/MarkdownContent'
import { Button } from '../../ui/Button'
import { IconRefresh } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import { countSectionItems, parseChangelogSection } from '../lib/changelog'
import type { AppUpdate } from '../hooks/useAppUpdate'
import '../../styles/update.css'

/**
 * 新版本更新内容预览（提示条「更新内容」入口）。
 *
 * 数据源：`update.body` —— Tauri updater 从 latest.json 的 `notes` 字段带回，
 * 发布流程会把该版 CHANGELOG 段落写入 notes（从 1.1.3 起）。
 * 正文按 CHANGELOG 段落解析（`## [x]` 起）；解析不出条目时按原文降级展示，
 * 字段为空则显示空态——绝不伪装成本次有说明。
 */
export function UpdatePreviewModal({
  open,
  onClose,
  updater,
}: {
  open: boolean
  onClose: () => void
  updater: AppUpdate
}) {
  const { t } = useLanguage()
  const { update, downloaded, installBlocked } = updater
  if (!open || !update) return null

  const body = update.body ?? ''
  const section = body ? parseChangelogSection(body, update.version) : null
  const sectionReady = section !== null && countSectionItems(section) > 0

  return (
    <CompactModal
      open={open}
      onClose={onClose}
      title={t('update.previewTitle', update.version)}
      icon={<IconRefresh size={14} />}
      size="md"
      footer={
        <>
          <Button
            variant="primary"
            size="sm"
            disabled={installBlocked}
            onClick={() => {
              void updater.install()
              onClose()
            }}
          >
            {t(downloaded ? 'update.installRestart' : 'update.downloadInstall')}
          </Button>
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t('common.close')}
          </Button>
        </>
      }
    >
      <div className="update-changes">
        <div className="update-changes-body">
          {sectionReady ? (
            section.groups.map(group => (
              <div className="update-changes-group" key={group.title}>
                {group.title && (
                  <div className="update-changes-group-title">
                    <MarkdownInline text={group.title} />
                  </div>
                )}
                <ul className="update-changes-list">
                  {group.items.map((item, index) => (
                    <li key={`${group.title}-${index}`}>
                      <MarkdownInline text={item} />
                    </li>
                  ))}
                </ul>
              </div>
            ))
          ) : body ? (
            <div className="update-preview-raw">{body}</div>
          ) : (
            <div className="update-changes-empty">{t('update.previewEmpty')}</div>
          )}
        </div>
      </div>
    </CompactModal>
  )
}
