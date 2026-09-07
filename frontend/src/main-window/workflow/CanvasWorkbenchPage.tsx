import { useEffect, useState } from 'react'
import { useLanguage } from '../../locales'
import { IconPalette, IconWrench, IconWorkflow, IconX } from '../../ui/Icons'
import { listWorkflows, wfSave } from '../lib/api'
import { CanvasPage } from '../workflow-canvas/CanvasPage'
import { ToolsPage } from '../tools/ToolsPage'
import { UiPrototypeCanvas } from '../canvases/ui-prototype/UiPrototypeCanvas'
import type { WorkflowItem } from '../../core/types'
import './workflow-workbench.css'

type CanvasType = 'workflow-editor' | 'prototype' | 'tools'

export function CanvasWorkbenchPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [canvasType, setCanvasType] = useState<CanvasType>('workflow-editor')
  const [workflowId, setWorkflowId] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  useEffect(() => {
    let cancelled = false
    const openWorkflowEditor = async () => {
      try {
        const workflows = (await listWorkflows()) ?? []
        const draft = workflows
          .filter((item: WorkflowItem) => item.status === 'draft')
          .sort((a: WorkflowItem, b: WorkflowItem) => b.updated_at - a.updated_at)[0]
        if (draft) {
          if (!cancelled) setWorkflowId(draft.id)
          return
        }
        const id = crypto.randomUUID()
        const result = await wfSave({
          id,
          name: '未命名工作流',
          status: 'Draft',
          steps: [],
          doc: null,
          schedule: null,
          run_history: [],
          dry_run: false,
        })
        if (!result?.saved) throw new Error('工作流创建失败')
        if (!cancelled) setWorkflowId(id)
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err))
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void openWorkflowEditor()
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <div className="workflow-workbench">
      <header className="workflow-workbench-header">
        <button
          type="button"
          className="workflow-workbench-close"
          onClick={onClose}
          aria-label={t('common.close')}
        >
          <IconX size={16} />
        </button>
        <div className="workflow-workbench-title">
          <IconPalette size={16} />
          <span>{t('cmd.canvas')}</span>
        </div>
        <nav className="workflow-workbench-types" aria-label={t('workflow.workTypeLabel')}>
          <button
            type="button"
            className={canvasType === 'workflow-editor' ? 'is-active' : ''}
            onClick={() => setCanvasType('workflow-editor')}
          >
            <IconWorkflow size={14} />
            <span>{t('workflow.workType.workflowEditor')}</span>
          </button>
          <button
            type="button"
            className={canvasType === 'prototype' ? 'is-active' : ''}
            onClick={() => setCanvasType('prototype')}
          >
            <IconPalette size={14} />
            <span>{t('workflow.workType.prototype')}</span>
          </button>
          <button
            type="button"
            className={canvasType === 'tools' ? 'is-active' : ''}
            onClick={() => setCanvasType('tools')}
          >
            <IconWrench size={14} />
            <span>{t('workflow.workType.tools')}</span>
          </button>
        </nav>
      </header>
      <main className={`workflow-workbench-body workflow-workbench-body--${canvasType}`}>
        {canvasType === 'workflow-editor' && (
          <>
            {loading && <div className="page-loading">{t('common.loading')}</div>}
            {!loading && error && <div className="error-banner">{error}</div>}
            {!loading && workflowId && <CanvasPage workflowId={workflowId} onClose={onClose} />}
          </>
        )}
        {canvasType === 'prototype' && (
          <div className="workflow-workbench-prototype">
            <UiPrototypeCanvas />
          </div>
        )}
        {canvasType === 'tools' && <ToolsPage onClose={onClose} embedded />}
      </main>
    </div>
  )
}
