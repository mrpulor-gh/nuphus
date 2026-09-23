/**
 * DataDirsPage.tsx — 设置中心「数据目录」分区
 *
 * 定位：只读展示本机各数据目录的真实绝对路径，让用户知道数据落在磁盘哪里
 * （排障 / 备份 / 手动查看）。**不提供修改、不提供迁移** —— 因此本页没有任何
 * 输入控件，只有「打开」这一个动作，且仅对真实存在的目录开放。
 *
 * 几个刻意的约定：
 * - 不存在的目录照常列出并标注「未创建」，不静默隐藏：用户需要知道「本该在哪」，
 *   静默隐藏会让「为什么没有数据」变成无从排查的问题。
 * - 不做体积统计：`%APPDATA%\Nuphus` 实测数千文件，递归 sum 是 IO 密集操作，
 *   而本页没有对应的用户动作（无迁移），数字只是噱头。
 * - 「打开」复用既有 `openPath`（内部已做存在性检查），不新增替代实现。
 */
import { useEffect, useState } from 'react'
import { listDataDirs, openPath, type DataDirEntry } from '../lib/api'
import { useLanguage } from '../../locales'
import { Section } from '../../ui/PageLayout'
import { IconAlertCircle, IconFolder } from '../../ui/Icons'
import '../../styles/settings-center.css'

export function DataDirsPage() {
  const { t } = useLanguage()
  const [entries, setEntries] = useState<DataDirEntry[] | null>(null)
  const [failed, setFailed] = useState(false)
  /** 正在打开的 key：用于禁用按钮避免连点 */
  const [opening, setOpening] = useState<string | null>(null)
  /** 打开失败的 key → 错误文案（系统层面失败时给出可感知反馈） */
  const [openError, setOpenError] = useState<{ key: string; message: string } | null>(null)

  useEffect(() => {
    let alive = true
    listDataDirs()
      .then(list => {
        if (alive) setEntries(list)
      })
      .catch(e => {
        if (!alive) return
        console.error('列举数据目录失败:', e)
        setFailed(true)
        setEntries([])
      })
    return () => {
      alive = false
    }
  }, [])

  const handleOpen = async (entry: DataDirEntry) => {
    setOpening(entry.key)
    setOpenError(null)
    try {
      await openPath(entry.path)
    } catch (e) {
      setOpenError({ key: entry.key, message: String(e) })
    } finally {
      setOpening(null)
    }
  }

  if (entries === null) return <div className="page-loading">{t('common.loading')}</div>

  return (
    <div>
      <Section title={t('dataDirs.title')} description={t('dataDirs.desc')}>
        {failed ? (
          <div className="data-dirs-empty">{t('dataDirs.loadFailed')}</div>
        ) : (
          <div className="data-dirs-list">
            {entries.map(entry => {
              const resolved = entry.path.length > 0
              return (
                <div className="data-dirs-row" key={entry.key}>
                  <div className="data-dirs-copy">
                    <div className="data-dirs-name">
                      <span className="data-dirs-icon" aria-hidden="true">
                        <IconFolder size={14} />
                      </span>
                      {t(`dataDirs.item.${entry.key}`)}
                      <span
                        className={`data-dirs-status is-${entry.exists ? 'present' : 'missing'}`}
                      >
                        {entry.exists ? t('dataDirs.present') : t('dataDirs.missing')}
                      </span>
                    </div>
                    {resolved ? (
                      /* 路径可能远超一行：允许折行并可选中复制（用户排障要能复制路径） */
                      <div className="data-dirs-path" title={entry.path}>
                        {entry.path}
                      </div>
                    ) : (
                      <div className="data-dirs-path is-unresolved">{t('dataDirs.unresolved')}</div>
                    )}
                    <div className="data-dirs-hint">{t(`dataDirs.item.${entry.key}Desc`)}</div>
                    {openError?.key === entry.key && (
                      <div className="data-dirs-error">
                        <IconAlertCircle size={12} />
                        {openError.message}
                      </div>
                    )}
                  </div>
                  {/* 不存在 / 无法解析 → 不提供打开按钮（点了必然失败，不如不给） */}
                  {entry.exists && (
                    <button
                      type="button"
                      className="data-dirs-open"
                      disabled={opening === entry.key}
                      onClick={() => void handleOpen(entry)}
                    >
                      {t('dataDirs.open')}
                    </button>
                  )}
                </div>
              )
            })}
          </div>
        )}
      </Section>
    </div>
  )
}
