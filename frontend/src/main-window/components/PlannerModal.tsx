import React, { useEffect, useState } from 'react'
import { PlanTaskCard } from './PlanTaskCard'
import type { PlanData, PlanTask } from '../../core/types'
import { useLanguage } from '../../locales'
import '../../styles/planner.css'

interface PlannerModalProps {
  open: boolean
  plan: PlanData | null
  onClose: () => void
}

export const PlannerModal: React.FC<PlannerModalProps> = ({ open, plan, onClose }) => {
  const { t } = useLanguage()
  const [visible, setVisible] = useState(false)
  const [animating, setAnimating] = useState(false)

  useEffect(() => {
    if (open) {
      setVisible(true)
      requestAnimationFrame(() => setAnimating(true))
    } else {
      setAnimating(false)
      const t = setTimeout(() => setVisible(false), 250)
      return () => clearTimeout(t)
    }
  }, [open])

  if (!visible) return null

  const doneCount = plan?.tasks.filter(t => t.status === 'completed').length ?? 0
  const inProgressCount = plan?.tasks.filter(t => t.status === 'in_progress').length ?? 0
  const failedCount = plan?.tasks.filter(t => t.status === 'failed').length ?? 0
  const totalCount = plan?.tasks.length ?? 0

  return (
    <div className="pm-wrapper" style={{ pointerEvents: open ? 'auto' : 'none' }}>
      <div className={`pm-backdrop ${animating ? '' : 'is-hidden'}`} onClick={onClose} />

      <div className={`pm-content ${animating ? '' : 'is-closing'}`}>
        <div className="pm-header">
          <div className="planner-header-title">{plan?.topic || t('planner.title')}</div>
          <div className="pm-header-meta">
            <span>
              {t('planner.project')}: {plan?.project || '-'}
            </span>
            <span>{plan?.goalType || '-'}</span>
            <span
              className={`pm-header-status ${plan?.status === 'active' ? 'is-active' : 'is-archived'}`}
            >
              {plan?.status === 'active' ? t('planner.active') : t('planner.archived')}
            </span>
          </div>
        </div>

        <div className="planner-body">
          {plan?.requirement && (
            <div className="pm-req-card">
              <div className="pm-req-label">{t('planner.goal')}</div>
              <div className="pm-req-text">{plan.requirement}</div>
            </div>
          )}

          <div className="pm-context-card">
            <div className="pm-context-header">
              <div className="pm-context-label">
                <div className="planner-context-dot" />
                <span className="pm-context-title">{t('planner.context')}</span>
              </div>
            </div>
            <div className="planner-context-text">{plan?.context}</div>
          </div>

          <div className="pm-progress">
            <span>{t('planner.tasks', String(totalCount))}</span>
            <span className="pm-progress-stats">
              {inProgressCount > 0 && (
                <span className="pm-stat-inprogress">
                  {inProgressCount} {t('planner.inProgress')}
                </span>
              )}
              <span className="pm-stat-done">
                {doneCount} {t('planner.completed')}
              </span>
              {failedCount > 0 && (
                <span className="pm-stat-failed">
                  {failedCount} {t('planner.failed')}
                </span>
              )}
            </span>
          </div>

          {plan?.tasks?.map(task => (
            <PlanTaskCard key={task.id} task={task} index={task.id} />
          ))}
        </div>
      </div>
    </div>
  )
}
