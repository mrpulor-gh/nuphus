import { useCallback, useEffect, useState } from 'react'
import {
  getAnnotations,
  addAnnotation,
  updateAnnotation,
  removeAnnotation,
} from '../lib/api-memory'
import type { Annotation } from '../../core/types-memory'
import { IconEdit3, IconFolder, IconPin, IconPlus, IconTrash2, IconX } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'
import '../../styles/memory-dialogs.css'

interface AnnotationsDialogProps {
  onClose: () => void
}

/**
 * 关系标注管理弹窗：标注卡片列表 + 新增/编辑子弹窗。
 * 弹窗骨架复用全局 modal-* 体系；数据直连后端 annotations API。
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
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-content"
        role="dialog"
        aria-label={t('memory.settings.subAnnotations')}
        onClick={e => e.stopPropagation()}
      >
        <div className="modal-header">
          <span className="modal-title">{t('memory.settings.subAnnotations')}</span>
          <IconButton variant="modal-close" label={t('common.close')} onClick={onClose}>
            <IconX size={16} />
          </IconButton>
        </div>

        <div className="modal-body">
          <div className="memdlg-intro">
            <span className="memdlg-intro-icon" aria-hidden>
              <IconPin size={16} />
            </span>
            <span className="memdlg-intro-text">
              <span className="memdlg-intro-title">{t('memory.settings.subAnnotations')}</span>
              <span className="memdlg-guide-text">{t('memory.annotation.guide')}</span>
            </span>
            <Button variant="primary" size="sm" onClick={openAdd}>
              <IconPlus size={14} />
              {t('common.add')}
            </Button>
          </div>

          {loading ? (
            <div className="memdlg-empty">{t('common.loading')}</div>
          ) : annotations.length === 0 ? (
            <div className="memdlg-empty">{t('memory.noAnnotations')}</div>
          ) : (
            <div className="memdlg-list">
              {annotations.map(a => (
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
                    <button
                      type="button"
                      className="memdlg-icon-btn"
                      title={t('common.edit')}
                      aria-label={t('common.edit')}
                      onClick={() => openEdit(a)}
                    >
                      <IconEdit3 size={14} />
                    </button>
                    <button
                      type="button"
                      className="memdlg-icon-btn danger"
                      title={a.builtin ? t('memory.builtinNoDelete') : t('common.delete')}
                      aria-label={a.builtin ? t('memory.builtinNoDelete') : t('common.delete')}
                      disabled={a.builtin}
                      onClick={() => handleDelete(a.keyword)}
                    >
                      <IconTrash2 size={14} />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 新增/编辑子弹窗（嵌套在外层遮罩内，DOM 顺序天然置顶） */}
      {editOpen && (
        <div className="modal-overlay" style={{ zIndex: 1100 }} onClick={() => setEditOpen(false)}>
          <div
            className="modal-content memdlg-modal-sm"
            role="dialog"
            aria-label={editKeyword ? t('memory.annotation.edit') : t('memory.annotation.add')}
            onClick={e => e.stopPropagation()}
          >
            <div className="modal-header">
              <span className="modal-title">
                {editKeyword ? t('memory.annotation.edit') : t('memory.annotation.add')}
              </span>
              <IconButton
                variant="modal-close"
                label={t('common.close')}
                onClick={() => setEditOpen(false)}
              >
                <IconX size={16} />
              </IconButton>
            </div>
            <div className="modal-body memdlg-form-body">
              {/* ── 标识：关键词 + 同义词 ── */}
              <div className="memdlg-form-row">
                <div className="memdlg-form-group">
                  <label className="memdlg-label">
                    {t('memory.annotation.keyword')}
                    <em className="memdlg-req">*</em>
                  </label>
                  <input
                    type="text"
                    className="memdlg-input memdlg-input--mono"
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
              </div>

              {/* ── 内容：描述 ── */}
              <div className="memdlg-form-group">
                <label className="memdlg-label">
                  {t('memory.annotation.desc')}
                  <em className="memdlg-req">*</em>
                </label>
                <textarea
                  className="memdlg-textarea"
                  rows={3}
                  value={desc}
                  onChange={e => setDesc(e.target.value)}
                  placeholder={t('memory.annotation.descPlaceholder')}
                />
              </div>

              {/* ── 作用域：生效路径 ── */}
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.annotation.paths')}</label>
                <textarea
                  className="memdlg-textarea memdlg-textarea--mono"
                  rows={3}
                  value={paths}
                  onChange={e => setPaths(e.target.value)}
                  placeholder={t('memory.annotation.pathsPlaceholder')}
                />
              </div>

              {/* ── 元数据：标签 / 分组 / 优先级 ── */}
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
              <div className="memdlg-form-row memdlg-form-row--tail">
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
              <Button variant="default" onClick={() => setEditOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button
                variant="primary"
                loading={saving}
                disabled={!keyword.trim() || !desc.trim()}
                onClick={handleSave}
              >
                {t('common.save')}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
