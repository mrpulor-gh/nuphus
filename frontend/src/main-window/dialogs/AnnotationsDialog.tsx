import { useCallback, useEffect, useState } from 'react'
import { getAnnotations, addAnnotation, updateAnnotation, removeAnnotation } from '../lib/api-memory'
import type { Annotation } from '../../core/types-memory'
import { IconEdit3, IconFolder, IconPlus, IconTrash2, IconX } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import '../../styles/memory-dialogs.css'

interface AnnotationsDialogProps {
  onClose: () => void
}

/**
 * 关系标注管理弹窗（gemini 布局：ad-card 五段式 + 新增/编辑子弹窗）。
 * 数据直连后端 annotations API。
 */
export function AnnotationsDialog({ onClose }: AnnotationsDialogProps) {
  const { t } = useLanguage()
  const [annotations, setAnnotations] = useState<Annotation[]>([])
  const [loading, setLoading] = useState(true)
  const [editOpen, setEditOpen] = useState(false)
  const [editKeyword, setEditKeyword] = useState<string | null>(null)
  const [keyword, setKeyword] = useState('')
  const [keywords, setKeywords] = useState('')
  const [desc, setDesc] = useState('')
  const [paths, setPaths] = useState('')
  const [tags, setTags] = useState('')
  const [group, setGroup] = useState('custom')
  const [priority, setPriority] = useState(0)
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const res = await getAnnotations()
      setAnnotations((res || []).filter(a => !a.builtin))
    } catch (e) {
      console.error('Failed to load annotations:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  // Esc 关闭最上层
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (editOpen) setEditOpen(false)
        else onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [editOpen, onClose])

  const openAdd = useCallback(() => {
    setEditKeyword(null)
    setKeyword('')
    setKeywords('')
    setDesc('')
    setPaths('')
    setTags('')
    setGroup('custom')
    setPriority(0)
    setEditOpen(true)
  }, [])

  const openEdit = useCallback((a: Annotation) => {
    setEditKeyword(a.keyword)
    setKeyword(a.keyword)
    setKeywords((a.keywords || []).join(', '))
    setDesc(a.description)
    setPaths(a.paths.join('\n'))
    setTags(a.tags.join(', '))
    setGroup(a.group)
    setPriority(a.priority)
    setEditOpen(true)
  }, [])

  const handleSave = useCallback(async () => {
    if (!keyword.trim() || !desc.trim()) return
    setSaving(true)
    const pathsArr = paths
      .split('\n')
      .map(s => s.trim())
      .filter(Boolean)
    const tagsArr = tags
      .split(',')
      .map(s => s.trim())
      .filter(Boolean)
    const keywordsArr = keywords
      .split(/[,，、\n]+/)
      .map(s => s.trim())
      .filter(Boolean)
    try {
      if (editKeyword) {
        await updateAnnotation(editKeyword, desc, pathsArr, tagsArr, group, priority)
      } else {
        await addAnnotation(
          keyword,
          desc,
          pathsArr,
          tagsArr,
          group,
          priority,
          keywordsArr.length > 0 ? keywordsArr : undefined,
        )
      }
      setEditOpen(false)
      await load()
    } catch (e) {
      console.error('Failed to save annotation:', e)
    } finally {
      setSaving(false)
    }
  }, [keyword, keywords, desc, paths, tags, group, priority, editKeyword, load])

  const handleDelete = useCallback(
    async (kw: string) => {
      try {
        await removeAnnotation(kw)
        await load()
      } catch (e) {
        console.error('Failed to delete annotation:', e)
      }
    },
    [load],
  )

  return (
    <div className="memdlg-overlay" onClick={onClose}>
      <div className="memdlg-container" onClick={e => e.stopPropagation()}>
        <div className="memdlg-header">
          <div className="memdlg-title">{t('memory.settings.subAnnotations')}</div>
          <button type="button" className="memdlg-close" onClick={onClose} aria-label={t('common.close')}>
            <IconX size={16} />
          </button>
        </div>

        <div className="memdlg-body">
          <div className="memdlg-guide-row">
            <span className="memdlg-guide-text">{t('memory.annotation.guide')}</span>
            <button type="button" className="memdlg-btn-primary" onClick={openAdd}>
              <IconPlus size={14} />
              {t('common.add')}
            </button>
          </div>

          {loading ? (
            <div className="memdlg-guide-text">{t('common.loading')}</div>
          ) : annotations.length === 0 ? (
            <div className="memdlg-guide-text">{t('memory.noAnnotations')}</div>
          ) : (
            annotations.map(a => (
              <div key={a.id} className="ad-card">
                <div className="ad-row1">
                  <span className="ad-keyword">{a.keyword}</span>
                  {(a.keywords || []).map(kw => (
                    <span key={kw} className="ad-sub-keyword">
                      {kw}
                    </span>
                  ))}
                  <span className={`ad-group-badge ad-group-${a.group}`}>{a.group}</span>
                </div>
                <div className="ad-desc">{a.description}</div>
                {a.paths.length > 0 && (
                  <div className="ad-paths">
                    {a.paths.map(p => (
                      <div key={p} className="ad-path-item">
                        <IconFolder size={12} />
                        {p}
                      </div>
                    ))}
                  </div>
                )}
                {a.tags.length > 0 && (
                  <div className="ad-chips-row">
                    {a.tags.map(tag => (
                      <span key={tag} className="ad-chip ad-chip-success">
                        {tag}
                      </span>
                    ))}
                  </div>
                )}
                <div className="ad-actions">
                  <button type="button" className="ad-icon-btn" title={t('common.edit')} onClick={() => openEdit(a)}>
                    <IconEdit3 size={14} />
                  </button>
                  <button
                    type="button"
                    className="ad-icon-btn delete"
                    title={a.builtin ? t('memory.builtinNoDelete') : t('common.delete')}
                    disabled={a.builtin}
                    onClick={() => handleDelete(a.keyword)}
                  >
                    <IconTrash2 size={14} />
                  </button>
                </div>
              </div>
            ))
          )}
          <div className="list-padding-bottom" />
        </div>
      </div>

      {/* 新增/编辑子弹窗 */}
      {editOpen && (
        <div className="memdlg-overlay memdlg-overlay--sub" onClick={() => setEditOpen(false)}>
          <div className="memdlg-container memdlg-container--sm" onClick={e => e.stopPropagation()}>
            <div className="memdlg-header">
              <div className="memdlg-title">
                {editKeyword ? t('memory.annotation.edit') : t('memory.annotation.add')}
              </div>
              <button type="button" className="memdlg-close" onClick={() => setEditOpen(false)} aria-label={t('common.close')}>
                <IconX size={16} />
              </button>
            </div>
            <div className="memdlg-body">
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.annotation.keyword')} *</label>
                <input
                  type="text"
                  className="memdlg-input"
                  value={keyword}
                  onChange={e => setKeyword(e.target.value)}
                  placeholder={t('memory.annotation.keywordPlaceholder')}
                  disabled={!!editKeyword}
                />
              </div>
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.annotation.keywords')}</label>
                <input
                  type="text"
                  className="memdlg-input"
                  value={keywords}
                  onChange={e => setKeywords(e.target.value)}
                  placeholder={t('memory.annotation.keywordsPlaceholder')}
                />
              </div>
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.annotation.desc')} *</label>
                <textarea
                  className="memdlg-textarea"
                  rows={3}
                  value={desc}
                  onChange={e => setDesc(e.target.value)}
                  placeholder={t('memory.annotation.descPlaceholder')}
                />
              </div>
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.annotation.paths')}</label>
                <textarea
                  className="memdlg-textarea"
                  rows={3}
                  value={paths}
                  onChange={e => setPaths(e.target.value)}
                  placeholder={t('memory.annotation.pathsPlaceholder')}
                />
              </div>
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.annotation.tags')}</label>
                <input
                  type="text"
                  className="memdlg-input"
                  value={tags}
                  onChange={e => setTags(e.target.value)}
                  placeholder={t('memory.annotation.tagsPlaceholder')}
                />
              </div>
              <div className="memdlg-form-row">
                <div className="memdlg-form-group">
                  <label className="memdlg-label">{t('memory.annotation.group')}</label>
                  <select
                    className="memdlg-select"
                    value={group}
                    onChange={e => setGroup(e.target.value)}
                  >
                    <option value="system">system</option>
                    <option value="custom">custom</option>
                    <option value="user">user</option>
                  </select>
                </div>
                <div className="memdlg-form-group">
                  <label className="memdlg-label">{t('memory.annotation.priority')}</label>
                  <input
                    type="number"
                    className="memdlg-input"
                    value={priority}
                    onChange={e => setPriority(parseInt(e.target.value) || 0)}
                  />
                </div>
              </div>
            </div>
            <div className="memdlg-footer">
              <button type="button" className="memdlg-btn-ghost" onClick={() => setEditOpen(false)}>
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className="memdlg-btn-primary"
                disabled={!keyword.trim() || !desc.trim() || saving}
                onClick={handleSave}
              >
                {saving ? '…' : t('common.save')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
