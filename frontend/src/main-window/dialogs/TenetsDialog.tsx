import { useCallback, useEffect, useState } from 'react'
import { getTenets, deleteTenet, addTenet } from '../lib/api'
import { IconTrash2, IconX } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'

interface TenetsDialogProps {
  onClose: () => void
}

/**
 * 教导原则管理弹窗（自包含）——从记忆页设置 Tab 迁移而来。
 * 列表 / 新增 / 删除，数据经 getTenets/addTenet/deleteTenet 直连后端。
 */
export function TenetsDialog({ onClose }: TenetsDialogProps) {
  const { t } = useLanguage()
  const [tenets, setTenets] = useState<Array<{ id: string; content: string; priority: string }>>([])
  const [tenetsLoading, setTenetsLoading] = useState(false)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [content, setContent] = useState('')
  const [saving, setSaving] = useState(false)

  const load = useCallback(async () => {
    setTenetsLoading(true)
    try {
      const res = await getTenets()
      setTenets(res?.items || [])
    } catch (e) {
      console.error('Failed to load tenets:', e)
    } finally {
      setTenetsLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  const handleAdd = useCallback(async () => {
    if (!content.trim()) return
    setSaving(true)
    try {
      await addTenet(content.trim())
      setDialogOpen(false)
      setContent('')
      void load()
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
        void load()
      } catch (e) {
        console.error('Failed to delete tenet:', e)
      }
    },
    [load],
  )

  return (
    <div className="modal-overlay visible" onClick={onClose}>
      <div className="modal-content modal-content--lg" onClick={e => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">{t('memory.settings.subTenets')}</span>
          <IconButton variant="modal-close" label={t('common.close')} onClick={onClose}>
            <IconX size={16} />
          </IconButton>
        </div>
        <div className="modal-body">
          <div className="ann-top-bar">
            <div className="ann-guide">
              <span>{t('memory.tenet.guide')}</span>
            </div>
            <Button
              variant="primary"
              size="sm"
              onClick={() => {
                setContent('')
                setDialogOpen(true)
              }}
            >
              {t('memory.addTenet')}
            </Button>
          </div>
          <div className="ann-section-body ann-section-body--flush">
            {tenetsLoading ? (
              <div className="ann-empty">{t('common.loading')}</div>
            ) : tenets.length === 0 ? (
              <div className="ann-empty">{t('memory.noTenets')}</div>
            ) : (
              <div className="note-list">
                {tenets.map(tenet => (
                  <div key={tenet.id} className="note-card">
                    <div className="note-card-header note-card-header--static">
                      <div className="note-card-left">
                        <div className="note-card-title note-card-title--mono">
                          <span className="ann-group-tag tenet-priority-tag">
                            {tenet.priority}
                          </span>
                        </div>
                        <div className="note-card-meta">{tenet.content}</div>
                      </div>
                      <IconButton
                        variant="ghost"
                        label={t('common.delete')}
                        onClick={() => handleDelete(tenet.id)}
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

      {/* 新增条目子弹窗 */}
      {dialogOpen && (
        <div
          className="modal-overlay visible"
          onClick={() => setDialogOpen(false)}
          style={{ zIndex: 20 }}
        >
          <div className="modal-content modal-content--md" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <span className="modal-title">{t('memory.tenet.add')}</span>
              <IconButton variant="modal-close" label={t('common.close')} onClick={() => setDialogOpen(false)}>
                <IconX size={16} />
              </IconButton>
            </div>
            <div className="modal-body modal-body--form">
              <div>
                <label className="ann-label">{t('memory.tenet.content')}</label>
                <textarea
                  className="ann-textarea textarea"
                  value={content}
                  onChange={e => setContent(e.target.value)}
                  placeholder={t('memory.tenet.placeholder')}
                  rows={4}
                  autoFocus
                />
              </div>
              <div className="form-footer">
                <Button variant="default" onClick={() => setDialogOpen(false)}>
                  {t('common.cancel')}
                </Button>
                <Button variant="primary" loading={saving} onClick={handleAdd}>
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
