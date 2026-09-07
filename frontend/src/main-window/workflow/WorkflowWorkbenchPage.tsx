import { useState } from 'react'
import { useLanguage } from '../../locales'
import { IconPalette, IconWrench, IconWorkflow } from '../../ui/Icons'
import { WorkflowPage } from './WorkflowPage'
import { CanvasPage } from '../workflow-canvas/CanvasPage'
import { ToolsPage } from '../tools/ToolsPage'
import { UiPrototypeCanvas } from '../canvases/ui-prototype/UiPrototypeCanvas'
import type { WorkflowItem } from '../../core/types'
import './workflow-workbench.css'

type WorkType = 'workflow' | 'prototype' | 'tools'

interface WorkflowWorkbenchPageProps {
  onClose: () => void
  onRunClick: (workflow: WorkflowItem) => void
  onCanvasClick: (workflow: WorkflowItem) => void
}

export function WorkflowWorkbenchPage({
  onClose,
  onRunClick,
  onCanvasClick,
}: WorkflowWorkbenchPageProps) {
  const { t } = useLanguage()
  const [workType, setWorkType] = useState<WorkType>('workflow')
  const [editingWorkflowId, setEditingWorkflowId] = useState<string | null>(null)

  return (
    <div className="workflow-workbench">
      <header className="workflow-workbench-header">
        <div className="workflow-workbench-title">
          <IconWorkflow size={16} />
          <span>{t('cmd.workflows')}</span>
        </div>
        <nav className="workflow-workbench-types" aria-label={t('workflow.workTypeLabel')}>
          <button
            type="button"
            className={workType === 'workflow' ? 'is-active' : ''}
            onClick={() => setWorkType('workflow')}
          >
            <IconWorkflow size={14} />
            <span>{t('workflow.workType.workflow')}</span>
          </button>
          <button
            type="button"
            className={workType === 'prototype' ? 'is-active' : ''}
            onClick={() => setWorkType('prototype')}
          >
            <IconPalette size={14} />
            <span>{t('workflow.workType.prototype')}</span>
          </button>
          <button
            type="button"
            className={workType === 'tools' ? 'is-active' : ''}
            onClick={() => setWorkType('tools')}
          >
            <IconWrench size={14} />
            <span>{t('workflow.workType.tools')}</span>
          </button>
        </nav>
        <button type="button" className="workflow-workbench-close" onClick={onClose}>
          ×
        </button>
      </header>
      <main className={`workflow-workbench-body workflow-workbench-body--${workType}`}>
        {workType === 'workflow' && !editingWorkflowId && (
          <WorkflowPage
            onClose={onClose}
            onRunClick={onRunClick}
            onCanvasClick={workflow => {
              setEditingWorkflowId(workflow.id)
              setWorkType('workflow')
              onCanvasClick(workflow)
            }}
          />
        )}
        {workType === 'workflow' && editingWorkflowId && (
          <CanvasPage workflowId={editingWorkflowId} onClose={() => setEditingWorkflowId(null)} />
        )}
        {workType === 'prototype' && (
          <div className="workflow-workbench-prototype">
            <UiPrototypeCanvas />
          </div>
        )}
        {workType === 'tools' && <ToolsPage onClose={onClose} embedded />}
      </main>
    </div>
  )
}
