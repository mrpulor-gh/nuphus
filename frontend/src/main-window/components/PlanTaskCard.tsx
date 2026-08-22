import React from 'react'
import type { PlanTask, TaskStatus, TaskPriority } from '../../core/types'

interface PlanTaskCardProps {
  task: PlanTask
  index: number
}

const STATUS_SYMBOL: Record<TaskStatus, string> = {
  pending: '○',
  in_progress: '●',
  completed: '✓',
  cancelled: '—',
  failed: '✗',
}

const PRIORITY_LABEL: Record<TaskPriority, string> = {
  high: '高',
  medium: '中',
  low: '低',
}

export const PlanTaskCard: React.FC<PlanTaskCardProps> = ({ task, index }) => {
  return (
    <div className={`ptc-card is-${task.status}`}>
      <div className="ptc-header">
        <div className="ptc-header-left">
          <span className={`ptc-status is-${task.status}`} title={task.status}>
            {STATUS_SYMBOL[task.status]}
          </span>
          <span
            className={`ptc-priority is-${task.priority}`}
            title={`优先级: ${PRIORITY_LABEL[task.priority]}`}
          >
            {PRIORITY_LABEL[task.priority]}
          </span>
          <span className="ptc-name">{task.name}</span>
        </div>
      </div>

      {task.understanding && <div className="ptc-understanding">{task.understanding}</div>}
    </div>
  )
}
