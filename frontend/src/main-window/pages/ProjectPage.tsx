import { useCallback, useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { IconRestore, IconTrash2, IconFolder } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { Section } from '../../ui/PageLayout'
import { useLanguage } from '../../locales'
import {
  getProjectBookmarks,
  getProjectDir,
  setProjectBookmarks,
  setProjectDir,
  setProjectFolderArchived,
} from '../lib/api'
import type { ProjectBookmark, ProjectDirState } from '../lib/api'
import { friendlyIpcError } from '../lib/ipcError'
import '../../styles/project.css'

/** 路径末段名（书签默认名） */
function nameFromPath(p: string): string {
  return (
    p
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() || p
  )
}

/**
 * 项目中心 —— 项目目录配置与切换的**唯一界面**。
 *
 * 版式沿用原「Ctrl+K → 项目配置」的 Section 结构（本组件即其唯一实现：入口
 * 收敛到输入框项目入口 / `/project` 后，不存在第二份 UI）。
 *
 * 与旧实现的三点差异（缺陷修复，不改变视觉语言）：
 * 1. 数据源改为后端配置 —— preferences 是当前目录与书签的单一事实源；旧版前端
 *    自持 localStorage，且与另一处键名/字段都不同，两处书签互不相通；
 * 2. 点击书签 = **真正切换项目**（落盘 + 后端向活跃会话注入 user 内部消息），
 *    旧版只是把路径回填输入框、未应用；
 * 3. 失败显式提示（旧版把后端错误静默吞掉）。
 *
 * 文件夹归档的**恢复入口收敛到这里**（会话工作台只归档不恢复，决策 6 修订）：
 * 「项目书签」区只列未归档书签，「已归档文件夹」区列出归档项并逐项恢复
 * （`set_project_folder_archived(path, false)`）——恢复只改归档标记，**不切换工作目录**。
 */
export function ProjectCenter({ onApplied }: { onApplied?: (state: ProjectDirState) => void }) {
  const { t } = useLanguage()
  const [current, setCurrent] = useState<ProjectDirState>({ path: '', name: '', tag: 'default' })
  const [dirInput, setDirInput] = useState('')
  const [bookmarks, setBookmarks] = useState<ProjectBookmark[]>([])
  const [bookmarkName, setBookmarkName] = useState('')
  const [busy, setBusy] = useState(false)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)
  /** 选中的书签（点书签行仅选中，由「设为当前」应用 → 避免误点即切换） */
  const [selectedPath, setSelectedPath] = useState('')

  /** 书签分区：归档标记只影响分区归属，不影响书签表本身（书签表保留全部条目） */
  const activeBookmarks = bookmarks.filter(b => !b.archived)
  const archivedBookmarks = bookmarks.filter(b => b.archived)

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const [state, list] = await Promise.all([getProjectDir(), getProjectBookmarks()])
        if (cancelled) return
        setCurrent(state)
        setDirInput(state.path)
        setBookmarks(list)
      } catch (e) {
        if (!cancelled) setError(friendlyIpcError(e, '读取项目配置失败'))
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const flashSaved = () => {
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  /** 应用（切换/清除）项目目录：落盘 + 通知活跃会话（后端注入 user 内部消息）
   *  空路径 = 清除项目目录（后端有「已清除」分支，界面此前缺入口） */
  const applyDir = useCallback(
    async (path: string) => {
      const target = path.trim()
      if (!target && !current.path) return
      setBusy(true)
      setError(null)
      try {
        const state = await setProjectDir(target)
        setCurrent(state)
        setDirInput(state.path)
        setSelectedPath('')
        flashSaved()
        onApplied?.(state)
      } catch (e) {
        setError(friendlyIpcError(e, '切换项目目录失败'))
      } finally {
        setBusy(false)
      }
    },
    [onApplied, current.path],
  )

  const handleBrowse = async () => {
    try {
      const dir = await open({ directory: true, multiple: false, title: t('project.selectDir') })
      if (typeof dir === 'string' && dir) {
        setDirInput(dir)
        setSelectedPath('')
        if (!bookmarkName.trim()) setBookmarkName(nameFromPath(dir))
      }
    } catch (e) {
      setError(friendlyIpcError(e))
    }
  }

  const handleAddBookmark = async () => {
    const path = dirInput.trim()
    if (!path) return
    setError(null)
    if (bookmarks.some(b => b.path === path)) {
      setError('该书签已存在')
      return
    }
    try {
      const next = await setProjectBookmarks([
        ...bookmarks,
        { name: bookmarkName.trim() || nameFromPath(path), path },
      ])
      setBookmarks(next)
      setBookmarkName('')
      flashSaved()
    } catch (e) {
      setError(friendlyIpcError(e))
    }
  }

  const handleDeleteBookmark = async (path: string) => {
    setError(null)
    try {
      setBookmarks(await setProjectBookmarks(bookmarks.filter(b => b.path !== path)))
    } catch (e) {
      setError(friendlyIpcError(e))
    }
  }

  /**
   * 恢复已归档文件夹：归档标记置回 false（返回的最新书签表直接回填 → 该文件夹回到书签区）。
   *
   * **恢复 ≠ 切换**：只改归档标记，不触碰当前工作目录（不调用 setProjectDir）。
   * 这是「归档」的唯一反向路径——会话工作台侧只归档不恢复。
   */
  const handleRestoreArchived = async (path: string) => {
    setError(null)
    try {
      setBookmarks(await setProjectFolderArchived(path, false))
    } catch (e) {
      setError(friendlyIpcError(e, t('project.restoreFail')))
    }
  }

  return (
    <div>
      {/* ── 当前项目目录 ── */}
      <Section title={t('project.currentDir')}>
        <div className="compact-input-row">
          <input
            className="compact-input input-flex"
            value={dirInput}
            onChange={e => setDirInput(e.target.value)}
            placeholder={t('project.pathPlaceholder')}
          />
          <Button
            variant="default"
            size="sm"
            icon={<IconFolder size={13} />}
            onClick={handleBrowse}
          >
            {t('project.browse')}
          </Button>
        </div>
        {current.path && <div className="bookmark-path">项目记忆：memory/{current.tag}.md</div>}
        {/* 书签创建紧跟路径选择：选好目录 → 命名 → 加入下方书签区 */}
        <div className="compact-input-row input-row-spaced">
          <input
            className="compact-input input-flex"
            value={bookmarkName}
            onChange={e => setBookmarkName(e.target.value)}
            placeholder={t('project.bookmarkNamePlaceholder')}
          />
          <Button
            variant="default"
            size="sm"
            disabled={!dirInput.trim()}
            onClick={handleAddBookmark}
          >
            {t('project.addBookmark')}
          </Button>
        </div>
        {(saved || error || current.path) && (
          <div className="form-footer">
            {saved && <span className="badge badge-success">{t('common.saved')}</span>}
            {error && (
              <span style={{ color: 'var(--error)', fontSize: 'var(--fz-xs)' }}>{error}</span>
            )}
            {current.path && (
              <Button variant="ghost" size="sm" disabled={busy} onClick={() => applyDir('')}>
                清除项目目录
              </Button>
            )}
          </div>
        )}
      </Section>

      {/* ── 项目书签（只列未归档书签；归档项见下方「已归档文件夹」区）── */}
      <Section title={t('project.bookmarks')}>
        {activeBookmarks.length === 0 ? (
          <div className="page-empty">
            <div>{t('project.noBookmarks')}</div>
            <div className="page-empty-hint">{t('project.bookmarkHint')}</div>
          </div>
        ) : (
          <div className="page-list">
            {activeBookmarks.map(b => (
              <div
                key={b.path}
                className={`page-list-item${selectedPath === b.path ? ' active' : ''}`}
                role="button"
                tabIndex={0}
                title={b.path}
                onClick={() => {
                  setSelectedPath(b.path)
                  setDirInput(b.path)
                  setError(null)
                }}
                onKeyDown={e => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault()
                    setSelectedPath(b.path)
                    setDirInput(b.path)
                  }
                }}
              >
                <div className="bookmark-info">
                  <div className="bookmark-name">
                    {b.name}
                    {current.path === b.path && (
                      <span className="badge badge-success" style={{ marginLeft: 6 }}>
                        当前
                      </span>
                    )}
                  </div>
                  <div className="bookmark-path">{b.path}</div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={e => {
                    e.stopPropagation()
                    handleDeleteBookmark(b.path)
                  }}
                  title={t('project.deleteBookmark')}
                  icon={<IconTrash2 size={11} />}
                />
              </div>
            ))}
          </div>
        )}
        {/* 选中书签后由此应用 —— 避免「点一下就切换」的误操作；未选中时作用于上方路径 */}
        <div className="form-footer">
          <Button
            variant="primary"
            disabled={busy || !(selectedPath || dirInput).trim()}
            onClick={() => applyDir(selectedPath || dirInput)}
          >
            {t('project.setCurrent')}
          </Button>
        </div>
      </Section>

      {/* ── 已归档文件夹：会话工作台整组隐藏的文件夹在此恢复（恢复 ≠ 切换工作目录）── */}
      <Section title={t('project.archivedFolders')}>
        {archivedBookmarks.length === 0 ? (
          <div className="page-empty">
            <div>{t('project.archivedEmpty')}</div>
            <div className="page-empty-hint">{t('project.archivedHint')}</div>
          </div>
        ) : (
          <div className="page-list">
            {archivedBookmarks.map(b => (
              <div key={b.path} className="page-list-item is-static" title={b.path}>
                <div className="bookmark-info">
                  <div className="bookmark-name">{b.name}</div>
                  <div className="bookmark-path">{b.path}</div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => void handleRestoreArchived(b.path)}
                  title={t('project.restoreFolder')}
                  icon={<IconRestore size={11} />}
                >
                  {t('project.restoreFolder')}
                </Button>
              </div>
            ))}
          </div>
        )}
      </Section>
    </div>
  )
}
