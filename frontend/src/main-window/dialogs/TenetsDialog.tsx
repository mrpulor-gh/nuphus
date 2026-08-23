import { useCallback, useEffect, useState } from 'react'
import { getTenets, deleteTenet, addTenet } from '../lib/api'
import { IconPlus, IconTrash2, IconX } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import '../../styles/memory-dialogs.css'

interface TenetsDialogProps {
  onClose: () => void
}

/**
 * 教导原则管理弹窗（gemini 布局：guide 行 + td-card 列表 + 新增子弹窗）。
 * 数据直连后端 getTenets / addTenet / deleteTenet。
 */
export function TenetsDialog({ onClose }: TenetsDialogProps) {
  const { t } = useLanguage()
  const [tenets, setTenets] = useState<Array<{ id: string; content: string; priority: string }>>([])
  const [loading, setLoading] = useState(true)
  const [addOpen, setAddOpen] = useState(false)
  const [content, setContent] = useState('')
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const res = await getTenets()
      setTenets(res?.items || [])
    } catch (e) {
      console.error('Failed to load tenets:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  // Esc 关闭最上层：子弹窗优先
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (addOpen) setAddOpen(false)
        else onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [addOpen, onClose])

  const handleAdd = useCallback(async () => {
    if (!content.trim()) return
    setSaving(true)
    try {
      await addTenet(content.trim())
      setAddOpen(false)
      setContent('')
      await load()
    } catch (e) {
      console.error('Failed to add tenet:', e)
    } finally {
      setSaving(false)
    }
  }, [content, load])

  const handleDelete = useCallback(
    async (id: string) => {
      try {
        await deleteTenet(id)
        await load()
      } catch (e) {
        console.error('Failed to delete tenet:', e)
      }
    },
    [load],
  )

  return (
    <div className="memdlg-overlay" onClick={onClose}>
      <div className="memdlg-container" onClick={e => e.stopPropagation()}>
        <div className="memdlg-header">
          <div className="memdlg-title">{t('memory.settings.subTenets')}</div>
          <button type="button" className="memdlg-close" onClick={onClose} aria-label={t('common.close')}>
            <IconX size={16} />
          </button>
        </div>

        <div className="memdlg-body">
          <div className="memdlg-guide-row">
            <span className="memdlg-guide-text">{t('memory.tenet.guide')}</span>
            <button type="button" className="memdlg-btn-primary" onClick={() => setAddOpen(true)}>
              <IconPlus size={14} />
              {t('common.add')}
            </button>
          </div>

          {loading ? (
            <div className="memdlg-guide-text">{t('common.loading')}</div>
          ) : tenets.length === 0 ? (
            <div className="memdlg-guide-text">{t('memory.noTenets')}</div>
          ) : (
            tenets.map(tenet => (
              <div key={tenet.id} className="td-card">
                <div className="td-card-header">
                  <span className="td-badge-priority">{tenet.priority}</span>
                  <button
                    type="button"
                    className="td-icon-btn"
                    title={t('common.delete')}
                    onClick={() => handleDelete(tenet.id)}
                  >
                    <IconTrash2 size={14} />
                  </button>
                </div>
                <div className="td-card-body">{tenet.content}</div>
              </div>
            ))
          )}
          <div className="list-padding-bottom" />
        </div>
      </div>

      {/* 新增子弹窗 */}
      {addOpen && (
        <div className="memdlg-overlay memdlg-overlay--sub" onClick={() => setAddOpen(false)}>
          <div className="memdlg-container memdlg-container--sm" onClick={e => e.stopPropagation()}>
            <div className="memdlg-header">
              <div className="memdlg-title">{t('memory.tenet.add')}</div>
              <button type="button" className="memdlg-close" onClick={() => setAddOpen(false)} aria-label={t('common.close')}>
                <IconX size={16} />
              </button>
            </div>
            <div className="memdlg-body">
              <div className="memdlg-form-group">
                <label className="memdlg-label">{t('memory.tenet.content')}</label>
                <textarea
                  className="memdlg-textarea"
                  rows={4}
                  value={content}
                  onChange={e => setContent(e.target.value)}
                  placeholder={t('memory.tenet.placeholder')}
                  autoFocus
                />
              </div>
            </div>
            <div className="memdlg-footer">
              <button type="button" className="memdlg-btn-ghost" onClick={() => setAddOpen(false)}>
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className="memdlg-btn-primary"
                disabled={!content.trim()}
                onClick={handleAdd}
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
