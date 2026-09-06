import { createPortal } from 'react-dom'
import { useLanguage } from '../../locales'
import { canvasRegistry } from './canvas-registry'
import type { CanvasDef, CanvasKind } from './canvas-types'
import { IconWorkflow, IconPalette, IconX } from '../../ui/Icons'
import './canvases.css'

/**
 * CanvasHubPage — 画布平台首页（全屏覆盖层）
 *
 * 展示 CanvasRegistry 中全部画布能力；点卡片进入对应画布。
 * workflow → 复用现有工作流画布；ui-prototype 等移植中画布置灰。
 * 未来新画布（动效录制/投屏/真机调试）注册后自动出现在此处。
 */
export function CanvasHubPage({
  onClose,
  onOpenWorkflow,
  onOpenUiPrototype,
}: {
  onClose: () => void
  onOpenWorkflow: () => void
  onOpenUiPrototype: () => void
}) {
  const { t } = useLanguage()

  const openCanvas = (def: CanvasDef) => {
    if (def.disabled) return
    if (def.id === 'workflow') {
      onClose()
      onOpenWorkflow()
    } else if (def.id === 'ui-prototype') {
      onClose()
      onOpenUiPrototype()
    }
    // 其它就绪画布在此按 id 分发（未来动效录制/投屏等在此扩展）
  }

  return createPortal(
    <div className="canvas-hub">
      <div className="canvas-hub-toolbar">
        <button type="button" className="canvas-hub-close" onClick={onClose} aria-label="关闭">
          <IconX size={15} />
        </button>
        <span className="canvas-hub-title">{t('canvas.title')}</span>
      </div>
      <div className="canvas-hub-body">
        <div className="canvas-card-grid">
          {canvasRegistry.map(def => {
            const icon =
              def.icon === 'workflow' ? <IconWorkflow size={22} /> : <IconPalette size={22} />
            return (
              <button
                key={def.id}
                type="button"
                className={`canvas-card${def.disabled ? ' is-disabled' : ''}`}
                onClick={() => openCanvas(def)}
                disabled={def.disabled}
              >
                <span className="canvas-card-icon" aria-hidden="true">
                  {icon}
                </span>
                <span className="canvas-card-info">
                  <span className="canvas-card-name">
                    {t(def.nameKey)}
                    {def.badge && <span className="canvas-card-badge">{t(def.badge)}</span>}
                  </span>
                  <span className="canvas-card-desc">{t(def.descKey)}</span>
                </span>
              </button>
            )
          })}
        </div>
        <div className="canvas-hub-foot">{t('canvas.footNote')}</div>
      </div>
    </div>,
    document.body,
  )
}

/** CanvasHub 打开辅助：类型锚点（供未来按 kind 直达画布时引用） */
export type { CanvasKind }