import { useCallback, useEffect, useState } from 'react'
import { getTenets, deleteTenet, addTenet } from '../lib/api'
import { IconPlus, IconShield, IconTrash2, IconX } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'
import '../../styles/memory-dialogs.css'

interface TenetsDialogProps {
  onClose: () => void
}

/**
 * 教导原则管理弹窗：guide 行 + 原则卡片列表 + 新增子弹窗。
 * 弹窗骨架复用全局 modal-* 体系；数据直连后端 getTenets / addTenet / deleteTenet。
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
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-content"
        role="dialog"
        aria-label={t('memory.settings.subTenets')}
        onClick={e => e.stopPropagation()}
      >
        <div className="modal-header">
          <span className="modal-title">{t('memory.settings.subTenets')}</span>
          <IconButton variant="modal-close" label={t('common.close')} onClick={onClose}>
            <IconX size={16} />
          </IconButton>
        </div>

        <div className="modal-body">
          <div className="memdlg-intro">
            <span className="memdlg-intro-icon" aria-hidden>
              <IconShield size={16} />
            </span>
            <span className="memdlg-intro-text">
              <span className="memdlg-intro-title">{t('memory.settings.subTenets')}</span>
              <span className="memdlg-guide-text">{t('memory.tenet.guide')}</span>
            </span>
            <Button variant="primary" size="sm" onClick={() => setAddOpen(true)}>
              <IconPlus size={14} />
              {t('common.add')}
            </Button>
          </div>

          {loading ? (
            <div className="memdlg-empty">{t('common.loading')}</div>
          ) : tenets.length === 0 ? (
            <div className="memdlg-empty">{t('memory.noTenets')}</div>
          ) : (
            <div className="memdlg-list">
              {tenets.map(tenet => (
                <div key={tenet.id} className="td-card">
                  <div className="td-card-header">
                    <span className="td-badge-priority">{tenet.priority}</span>
                    <button
                      type="button"
                      className="memdlg-icon-btn danger"
                      title={t('common.delete')}
                      aria-label={t('common.delete')}
                      onClick={() => handleDelete(tenet.id)}
                    >
                      <IconTrash2 size={14} />
                    </button>
                  </div>
                  <div className="td-card-body">{tenet.content}</div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* 新增子弹窗（嵌套在外层遮罩内，DOM 顺序天然置顶） */}
      {addOpen && (
        <div className="modal-overlay" style={{ zIndex: 1100 }} onClick={() => setAddOpen(false)}>
          <div
            className="modal-content memdlg-modal-sm"
            role="dialog"
            aria-label={t('memory.tenet.add')}
            onClick={e => e.stopPropagation()}
          >
            <div className="modal-header">
              <span className="modal-title">{t('memory.tenet.add')}</span>
              <IconButton
                variant="modal-close"
                label={t('common.close')}
                onClick={() => setAddOpen(false)}
              >
                <IconX size={16} />
              </IconButton>
            </div>
            <div className="modal-body memdlg-form-body">
              <div className="memdlg-form-group">
                <label className="memdlg-label">
                  {t('memory.tenet.content')}
                  <em className="memdlg-req">*</em>
                </label>
                <textarea
                  className="memdlg-textarea"
                  rows={5}
                  value={content}
                  onChange={e => setContent(e.target.value)}
                  placeholder={t('memory.tenet.placeholder')}
                  autoFocus
                />
              </div>
            </div>
            <div className="memdlg-footer">
              <Button variant="default" onClick={() => setAddOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button
                variant="primary"
                loading={saving}
                disabled={!content.trim()}
                onClick={handleAdd}
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
