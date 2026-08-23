import { useCallback, useEffect, useState } from 'react'
import { getAnnotations, addAnnotation, updateAnnotation, removeAnnotation } from '../lib/api-memory'
import type { Annotation } from '../../core/types-memory'
import { IconTrash2, IconX, IconEdit3 } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'

interface AnnotationsDialogProps {
  onClose: () => void
}

/**
 * 关系标注管理弹窗（自包含）——从记忆页设置 Tab 迁移而来。
 * 列表 / 新增 / 编辑 / 删除，数据经 annotations API 直连后端。
 */
export function AnnotationsDialog({ onClose }: AnnotationsDialogProps) {
  const { t } = useLanguage()
  const [annotations, setAnnotations] = useState<Annotation[]>([])
  const [dialogOpen, setDialogOpen] = useState(false)
  const [editKeyword, setEditKeyword] = useState<string | null>(null)
  const [keyword, setKeyword] = useState('')
  const [keywords, setKeywords] = useState('')
  const [desc, setDesc] = useState('')
  const [paths, setPaths] = useState('')
  const [tags, setTags] = useState('')
  const [group, setGroup] = useState('custom')
  const [priority, setPriority] = useState(0)

  const load = useCallback(async () => {
    try {
      const res = await getAnnotations()
      setAnnotations((res || []).filter(a => !a.builtin))
    } catch (e) {
      console.error('Failed to load annotations:', e)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const openAdd = useCallback(() => {
    setEditKeyword(null)
    setKeyword('')
    setKeywords('')
    setDesc('')
    setPaths('')
    setTags('')
    setGroup('custom')
    setPriority(0)
    setDialogOpen(true)
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
    setDialogOpen(true)
  }, [])

  const handleSave = useCallback(async () => {
    if (!keyword.trim() || !desc.trim()) return
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
      setDialogOpen(false)
      void load()
    } catch (e) {
      console.error('Failed to save annotation:', e)
    }
  }, [keyword, keywords, desc, paths, tags, group, priority, editKeyword, load])

  const handleDelete = useCallback(
    async (kw: string) => {
      try {
        await removeAnnotation(kw)
        void load()
      } catch (e) {
        console.error('Failed to delete annotation:', e)
      }
    },
    [load],
  )

  return (
    <div className="modal-overlay visible" onClick={onClose}>
      <div className="modal-content modal-content--lg" onClick={e => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">{t('memory.settings.subAnnotations')}</span>
          <IconButton variant="modal-close" label={t('common.close')} onClick={onClose}>
            <IconX size={16} />
          </IconButton>
        </div>
        <div className="modal-body">
          <div className="ann-top-bar">
            <div className="ann-guide">
              <span>{t('memory.annotation.guide')}</span>
            </div>
            <Button variant="primary" size="sm" onClick={openAdd}>
              {t('memory.addAnnotation')}
            </Button>
          </div>
          <div className="ann-section-body ann-section-body--flush">
            {annotations.length === 0 ? (
              <div className="ann-empty">{t('memory.noAnnotations')}</div>
            ) : (
              <div className="ann-list">
                {annotations.map(a => (
                  <div key={a.id} className="ann-item">
                    <div className="ann-item-header">
                      <span className="ann-keyword">{a.keyword}</span>
                      {a.keywords &&
                        a.keywords.length > 0 &&
                        a.keywords.map((kw, i) => (
                          <span key={i} className="ann-sub-keyword">
                            {kw}
                          </span>
                        ))}
                      {a.builtin && <span className="ann-badge">{t('memory.builtin')}</span>}
                      <span className={`ann-group-tag ann-group-${a.group}`}>{a.group}</span>
                    </div>
                    <div className="ann-desc">{a.description}</div>
                    {a.paths.length > 0 && (
                      <div className="ann-paths">
                        {a.paths.map((p, i) => (
                          <span key={i} className="ann-path">
                            {p}
                          </span>
                        ))}
                      </div>
                    )}
                    {a.tags.length > 0 && (
                      <div className="ann-tags">
                        {a.tags.map((tag, i) => (
                          <span key={i} className="ann-tag">
                            {tag}
                          </span>
                        ))}
                      </div>
                    )}
                    <div className="ann-item-actions">
                      <IconButton variant="ghost" label={t('common.edit')} onClick={() => openEdit(a)}>
                        <IconEdit3 size={12} />
                      </IconButton>
                      <IconButton
                        variant="ghost"
                        label={a.builtin ? t('memory.builtinNoDelete') : t('common.delete')}
                        onClick={() => !a.builtin && handleDelete(a.keyword)}
                        disabled={a.builtin}
                      >
                        <IconTrash2 size={12} />
                      </IconButton>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* 新增/编辑子弹窗 */}
      {dialogOpen && (
        <div
          className="modal-overlay visible"
          onClick={() => setDialogOpen(false)}
          style={{ zIndex: 20 }}
        >
          <div className="modal-content modal-content--lg" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <span className="modal-title">
                {editKeyword ? t('memory.annotation.edit') : t('memory.annotation.add')}
              </span>
              <IconButton
                variant="modal-close"
                label={t('common.close')}
                onClick={() => setDialogOpen(false)}
              >
                <IconX size={16} />
              </IconButton>
            </div>
            <div className="modal-body modal-body--form">
              <div>
                <label className="ann-label">{t('memory.annotation.keyword')}</label>
                <input
                  className="ann-input input"
                  value={keyword}
                  onChange={e => setKeyword(e.target.value)}
                  placeholder={t('memory.annotation.keywordPlaceholder')}
                  disabled={!!editKeyword}
                />
              </div>
              <div>
                <label className="ann-label">{t('memory.annotation.keywords')}</label>
                <input
                  className="ann-input input"
                  value={keywords}
                  onChange={e => setKeywords(e.target.value)}
                  placeholder={t('memory.annotation.keywordsPlaceholder')}
                />
              </div>
              <div>
                <label className="ann-label">{t('memory.annotation.desc')}</label>
                <textarea
                  className="ann-textarea textarea"
                  value={desc}
                  onChange={e => setDesc(e.target.value)}
                  placeholder={t('memory.annotation.descPlaceholder')}
                  rows={3}
                />
              </div>
              <div>
                <label className="ann-label">{t('memory.annotation.paths')}</label>
                <textarea
                  className="ann-textarea textarea"
                  value={paths}
                  onChange={e => setPaths(e.target.value)}
                  placeholder={t('memory.annotation.pathsPlaceholder')}
                  rows={3}
                />
              </div>
              <div>
                <label className="ann-label">{t('memory.annotation.tags')}</label>
                <input
                  className="ann-input input"
                  value={tags}
                  onChange={e => setTags(e.target.value)}
                  placeholder={t('memory.annotation.tagsPlaceholder')}
                />
              </div>
              <div className="ann-row-2col">
                <div>
                  <label className="ann-label">{t('memory.annotation.group')}</label>
                  <select
                    className="ann-select select"
                    value={group}
                    onChange={e => setGroup(e.target.value)}
                  >
                    <option value="system">system</option>
                    <option value="custom">custom</option>
                    <option value="user">user</option>
                  </select>
                </div>
                <div>
                  <label className="ann-label">{t('memory.annotation.priority')}</label>
                  <input
                    className="ann-input input"
                    type="number"
                    value={priority}
                    onChange={e => setPriority(parseInt(e.target.value) || 0)}
                  />
                </div>
              </div>
              <div className="form-footer">
                <Button variant="default" onClick={() => setDialogOpen(false)}>
                  {t('common.cancel')}
                </Button>
                <Button variant="primary" onClick={handleSave}>
                  {t('common.save')}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
