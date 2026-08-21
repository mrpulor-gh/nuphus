import { useState, useEffect } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { IconTrash2, IconFolder } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { Section } from '../../ui/PageLayout'
import { useLanguage } from '../../locales'
import { setProjectDir as apiSetProjectDir, setProjectBookmarks } from '../lib/api'
import '../../styles/project.css'

interface Bookmark {
  name: string
  path: string
}

const STORAGE_KEY = 'nuphus_project_bookmarks'

function loadBookmarks(): Bookmark[] {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]')
  } catch {
    return []
  }
}

function saveBookmarks(bookmarks: Bookmark[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(bookmarks))
}

export function ProjectPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [projectDir, setProjectDir] = useState('')
  const [saved, setSaved] = useState(false)
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([])
  const [bookmarkName, setBookmarkName] = useState('')

  useEffect(() => {
    setProjectDir(localStorage.getItem('nuphus_project_dir') || '')
    setBookmarks(loadBookmarks())
  }, [])

  const handleBrowse = async () => {
    const dir = await open({ directory: true, multiple: false, title: t('project.selectDir') })
    if (dir) setProjectDir(dir)
  }

  const handleSave = () => {
    if (!projectDir.trim()) return
    localStorage.setItem('nuphus_project_dir', projectDir.trim())
    apiSetProjectDir(projectDir.trim())
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const handleAddBookmark = () => {
    if (!projectDir.trim()) return
    const name =
      bookmarkName.trim() ||
      projectDir.split('\\\\').pop() ||
      projectDir.split('/').pop() ||
      projectDir
    const bookmarks = loadBookmarks()
    if (bookmarks.some(b => b.path === projectDir.trim())) return
    bookmarks.push({ name, path: projectDir.trim() })
    saveBookmarks(bookmarks)
    setBookmarks(bookmarks)
    setBookmarkName('')
    setProjectBookmarks(bookmarks) // sync to backend
  }

  const handleDeleteBookmark = (path: string) => {
    const bookmarks = loadBookmarks()
    const next = bookmarks.filter(b => b.path !== path)
    saveBookmarks(next)
    setBookmarks(next)
    setProjectBookmarks(next) // sync to backend
  }

  return (
    <div>
      {/* ── 当前项目目录 ── */}
      <Section title={t('project.currentDir')}>
        <div className="compact-input-row">
          <input
            className="compact-input input-flex"
            value={projectDir}
            onChange={e => setProjectDir(e.target.value)}
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
        <div className="form-footer">
          {saved && <span className="badge badge-success">{t('common.saved')}</span>}
          <Button variant="primary" onClick={handleSave}>
            {t('project.setCurrent')}
          </Button>
        </div>
      </Section>

      {/* ── 项目书签 ── */}
      <Section title={t('project.bookmarks')}>
        {bookmarks.length === 0 ? (
          <div className="page-empty">
            <div>{t('project.noBookmarks')}</div>
            <div className="page-empty-hint">{t('project.bookmarkHint')}</div>
          </div>
        ) : (
          <div className="page-list">
            {bookmarks.map(b => (
              <div
                key={b.path}
                className="page-list-item"
                onClick={() => {
                  setProjectDir(b.path)
                  localStorage.setItem('nuphus_project_dir', b.path)
                  setSaved(true)
                  setTimeout(() => setSaved(false), 2000)
                }}
              >
                <div className="bookmark-info">
                  <div className="bookmark-name">{b.name}</div>
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
        <div className="compact-input-row input-row-spaced">
          <input
            className="compact-input input-flex"
            value={bookmarkName}
            onChange={e => setBookmarkName(e.target.value)}
            placeholder={t('project.bookmarkNamePlaceholder')}
          />
          <Button variant="default" size="sm" onClick={handleAddBookmark}>
            {t('project.addBookmark')}
          </Button>
        </div>
      </Section>
    </div>
  )
}
