import { useLanguage } from '../../locales'
import './workflow-workbench.css'

/** Keep native window dragging available while the lazy workbench chunk loads. */
export function CanvasWorkbenchLoading() {
  const { t } = useLanguage()
  return (
    <div className="canvas-workbench-host">
      <div className="workflow-workbench">
        <header className="workflow-workbench-header">
          <span className="workflow-workbench-title">{t('cmd.canvas')}</span>
          <span className="workflow-workbench-drag" data-tauri-drag-region />
        </header>
        <div className="workflow-workbench-body page-loading">{t('common.loading')}</div>
      </div>
    </div>
  )
}
