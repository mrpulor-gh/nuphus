// KnowledgePage.tsx — 知识库管理页面
// 搜索、标签筛选、文档列表、新建/删除

import { useState, useEffect, useCallback, useMemo } from 'react'
import { searchKnowledge, listKnowledge, listKnowledgeTags, deleteKnowledge } from '../lib/api'
import type { KnowledgeHit } from '../../core/types'
import { IconSearch, IconTrash2, IconX, IconFile } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import '../../styles/knowledge.css'
import { useLanguage } from '../../locales'

function formatTime(ts: number, t?: (key: string, ...args: string[]) => string): string {
  try {
    const d = new Date(ts * 1000)
    const now = new Date()
    const diff = now.getTime() - d.getTime()
    if (diff < 60000) return t ? t('time.justNow') : 'Just now'
    if (diff < 3600000)
      return t
        ? t('time.minAgo', String(Math.floor(diff / 60000)))
        : `${Math.floor(diff / 60000)} min ago`
    if (diff < 86400000)
      return t
        ? t('time.hrAgo', String(Math.floor(diff / 3600000)))
        : `${Math.floor(diff / 3600000)} hr ago`
    return d.toLocaleDateString('default', { month: 'short', day: 'numeric' })
  } catch {
    return ''
  }
}

function truncate(text: string, max = 120): string {
  if (!text) return ''
  return text.length <= max ? text : text.slice(0, max) + '...'
}

export function KnowledgePage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [items, setItems] = useState<KnowledgeHit[]>([])
  const [allTags, setAllTags] = useState<string[]>([])
  const [selectedTag, setSelectedTag] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const loadData = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const [hits, tags] = await Promise.all([
        listKnowledge().catch(() => []),
        listKnowledgeTags().catch(() => []),
      ])
      // 过滤内置 nuphus-self 文档，仅搜索时可见
      setItems((hits || []).filter(h => !h.rel_path.startsWith('nuphus-self/')))
      setAllTags(tags || [])
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadData()
  }, [loadData])

  const handleSearch = useCallback(async () => {
    if (!searchQuery.trim() && !selectedTag) {
      loadData()
      return
    }
    setLoading(true)
    try {
      const tags = selectedTag ? [selectedTag] : undefined
      const hits = await searchKnowledge(searchQuery, tags)
      setItems(hits || [])
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }, [searchQuery, selectedTag, loadData])

  useEffect(() => {
    const timer = setTimeout(handleSearch, 300)
    return () => clearTimeout(timer)
  }, [handleSearch])

  const [deleting, setDeleting] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)

  const handleDelete = useCallback(async (relPath: string) => {
    setDeleting(relPath)
    try {
      await deleteKnowledge(relPath)
      setItems(prev => prev.filter(i => i.rel_path !== relPath))
      setConfirmDelete(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setDeleting(null)
    }
  }, [])

  const toggleTag = async (tag: string) => {
    const next = tag === selectedTag ? null : tag
    setSelectedTag(next)
    if (!searchQuery.trim() && !next) {
      loadData()
      return
    }
    setLoading(true)
    try {
      const tags = next ? [next] : undefined
      const hits = await searchKnowledge(searchQuery, tags)
      setItems(hits || [])
    } catch (e) {
      setError(String(e))
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="page">
      <div className="page-search">
        <IconSearch size={14} />
        <input
          placeholder={t('knowledge.search')}
          value={searchQuery}
          onChange={e => setSearchQuery(e.target.value)}
        />
      </div>

      {allTags.length > 0 && (
        <div className="tag-filter">
          {allTags.map(tag => (
            <button
              key={tag}
              className={`tag-filter-btn ${selectedTag === tag ? 'active' : ''}`}
              onClick={() => toggleTag(tag)}
            >
              {tag}
            </button>
          ))}
          {selectedTag && (
            <button
              className="tag-filter-clear"
              onClick={() => setSelectedTag(null)}
              title={t('knowledge.clearFilter')}
            >
              <IconX size={12} />
            </button>
          )}
        </div>
      )}

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button className="tag-filter-clear" onClick={() => setError(null)}>
            <IconX size={12} />
          </button>
        </div>
      )}

      {loading && <div className="page-loading">{t('common.loading')}</div>}

      {!loading && items.length === 0 && (
        <div className="page-empty">
          <IconFile size={32} />
          <div>{t('knowledge.noDocuments')}</div>
          <div className="page-empty-hint">{t('knowledge.hint')}</div>
        </div>
      )}

      {!loading && items.length > 0 && (
        <div className="page-list">
          {items.map(item => (
            <div key={item.rel_path} className="page-list-item">
              <div className="item-row">
                <div className="item-main">
                  <div className="doc-title">{item.title}</div>
                  <div className="doc-path">{item.rel_path}</div>
                  {item.tags.length > 0 && (
                    <div className="item-tags">
                      {item.tags.map(tag => (
                        <span key={tag} className="item-tag">
                          #{tag}
                        </span>
                      ))}
                    </div>
                  )}
                  {item.snippet && <div className="doc-snippet">{truncate(item.snippet, 200)}</div>}
                  <div className="doc-time">{formatTime(item.file_mtime, t)}</div>
                </div>
                <div className="item-actions">
                  {confirmDelete === item.rel_path ? (
                    <>
                      <Button
                        variant="danger"
                        size="sm"
                        onClick={() => handleDelete(item.rel_path)}
                        disabled={deleting === item.rel_path}
                      >
                        {deleting === item.rel_path ? '...' : t('common.confirmDelete')}
                      </Button>
                      <Button variant="default" size="sm" onClick={() => setConfirmDelete(null)}>
                        {t('common.cancel')}
                      </Button>
                    </>
                  ) : (
                    <IconButton
                      variant="ghost"
                      label={t('common.delete')}
                      onClick={() => setConfirmDelete(item.rel_path)}
                      className="delete-btn"
                    >
                      <IconTrash2 size={14} />
                    </IconButton>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
