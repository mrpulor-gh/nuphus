import React, { useState } from 'react'
import type { PlanTask, TaskStatus } from '../../core/types'
import { useLanguage } from '../../locales'
import '../../styles/task-bubble.css'

interface TaskBubbleProps {
  visible: boolean
  tasks: PlanTask[]
  onClose: () => void
}

const STATUS_CLASS: Record<TaskStatus, string> = {
  pending: 'pending',
  in_progress: 'running',
  completed: 'completed',
  cancelled: 'cancelled',
  failed: 'failed',
}

export const TaskBubble: React.FC<TaskBubbleProps> = ({ visible, tasks, onClose }) => {
  const { t } = useLanguage()
  // 手风琴展开：点击步骤行查看 planner 写入的完整理解（数据已在 PlanTask 内，零请求）
  const [expandedId, setExpandedId] = useState<number | null>(null)

  if (!visible || tasks.length === 0) return null

  const active = tasks.find(t => t.status === 'in_progress')
  const done = tasks.filter(t => t.status === 'completed').length
  const failed = tasks.filter(t => t.status === 'failed').length
  const total = tasks.length

  return (
    <div className="task-track">
      <div className="task-track-header">
        <div className="task-track-title">
          {active ? (
            <>
              <span className="task-track-shimmer" />
              <span className="task-track-active-name">{active.name}</span>
            </>
          ) : (
            <span className="task-track-idle">{t('taskBubble.idle')}</span>
          )}
        </div>
        <button className="task-track-close" onClick={onClose}>
          ✕
        </button>
      </div>

      <div className="task-track-progress">
        <div className="task-track-bar">
          <div
            className="task-track-fill"
            style={{ width: `${total > 0 ? (done / total) * 100 : 0}%` }}
          />
        </div>
        <span className="task-track-count">
          {done}/{total}
          {failed > 0 ? t('taskBubble.failed', String(failed)) : ''}
        </span>
      </div>

      <div className="task-track-list">
        {tasks.map(task => {
          const hasDetail = !!task.understanding?.trim()
          const expanded = expandedId === task.id
          return (
            <div
              key={task.id}
              className={[
                'task-track-item',
                STATUS_CLASS[task.status],
                hasDetail ? 'expandable' : '',
                expanded ? 'expanded' : '',
              ]
                .filter(Boolean)
                .join(' ')}
              onClick={() => hasDetail && setExpandedId(expanded ? null : task.id)}
            >
              <div className="task-track-item-row">
                <span className={`task-track-dot ${STATUS_CLASS[task.status]}`}>
                  {task.status === 'in_progress'
                    ? '●'
                    : task.status === 'completed'
                      ? '✓'
                      : task.status === 'failed'
                        ? '✗'
                        : task.status === 'cancelled'
                          ? '—'
                          : '○'}
                </span>
                <span className="task-track-name">{task.name}</span>
                {hasDetail && (
                  <span className={`task-track-chevron ${expanded ? 'open' : ''}`}>▾</span>
                )}
              </div>
              {expanded && hasDetail && (
                <div className="task-track-detail">{task.understanding}</div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}