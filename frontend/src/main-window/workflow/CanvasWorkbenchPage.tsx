import { lazy, Suspense, useEffect, useState } from 'react'
import { useLanguage } from '../../locales'
import { IconPalette, IconWrench, IconWorkflow, IconX } from '../../ui/Icons'
import { listWorkflows, wfSave } from '../lib/api'
import { scheduleIdle } from '../lib/idle'
import type { WorkflowItem } from '../../core/types'
import './workflow-workbench.css'

// ── 按 tab 拆包 ──
// 三个页面体量差异极大（UI 原型画布自身 4600+ 行且静态引入 motion / html-to-image），
// 静态 import 会把三者合并进同一个 chunk，打开任一 tab 都要付全部解析代价。
// tab 本身是条件渲染、切换即卸载，因此改为 lazy 不引入任何状态损失。
const CanvasPage = lazy(() =>
  import('../workflow-canvas/CanvasPage').then(m => ({ default: m.CanvasPage })),
)
const ToolsPage = lazy(() => import('../tools/ToolsPage').then(m => ({ default: m.ToolsPage })))
const UiPrototypeCanvas = lazy(() =>
  import('../canvases/ui-prototype/UiPrototypeCanvas').then(m => ({
    default: m.UiPrototypeCanvas,
  })),
)

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

  // ── 空闲预取默认 tab 的 chunk ──
  // 壳挂载后等浏览器空闲再发请求，不阻塞首屏；其余 tab 不预取，
  // 用户切过去时有 Suspense fallback 兜底反馈。
  useEffect(() => {
    scheduleIdle(() => {
      void import('../workflow-canvas/CanvasPage')
    })
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
            {!loading && workflowId && (
              <Suspense fallback={<div className="page-loading">{t('common.loading')}</div>}>
                <CanvasPage workflowId={workflowId} onClose={onClose} />
              </Suspense>
            )}
          </>
        )}
        {canvasType === 'prototype' && (
          <div className="workflow-workbench-prototype">
            <Suspense fallback={<div className="page-loading">{t('common.loading')}</div>}>
              <UiPrototypeCanvas onSent={onClose} />
            </Suspense>
          </div>
        )}
        {canvasType === 'tools' && (
          <Suspense fallback={<div className="page-loading">{t('common.loading')}</div>}>
            <ToolsPage onClose={onClose} embedded />
          </Suspense>
        )}
      </main>
    </div>
  )
}
