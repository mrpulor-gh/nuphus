import type { ChatReference, PendingImage, PendingFile } from '../../core/types'
import { IconImage, IconFile, IconCamera, IconWrench, IconBrain, IconWorkflow } from '../../ui/Icons'
import '../../styles/ref-bar.css'

interface ReferenceBarProps {
  references: ChatReference[]
  pendingImages: PendingImage[]
  pendingFiles: PendingFile[]
  onRemoveReference: (index: number) => void
  onRemoveImage: (index: number) => void
  onRemoveFile: (index: number) => void
}

const ICON_SIZE = 14

function RefIcon({ type }: { type: ChatReference['type'] }) {
  switch (type) {
    case 'capture':
      return <IconCamera size={ICON_SIZE} />
    case 'skill':
      return <IconWrench size={ICON_SIZE} />
    case 'knowledge':
      return <IconBrain size={ICON_SIZE} />
    case 'workflow':
      return <IconWorkflow size={ICON_SIZE} />
  }
}

function truncateName(name: string, max = 15): string {
  if (name.length <= max) return name
  return name.slice(0, max - 1) + '…'
}

export default function ReferenceBar({
  references,
  pendingImages,
  pendingFiles,
  onRemoveReference,
  onRemoveImage,
  onRemoveFile,
}: ReferenceBarProps) {
  if (references.length === 0 && pendingImages.length === 0 && pendingFiles.length === 0) {
    return null
  }

  return (
    <div className="ref-bar" role="status" aria-label="引用栏">
      {references.length > 0 && (
        <div className="ref-chips">
          {references.map((ref, i) => (
            <span
              key={`${ref.type}-${ref.id}-${i}`}
              className={`ref-chip ref-chip--${ref.type}`}
              title={ref.id}
            >
              <span className="ref-chip-icon" aria-hidden="true">
                <RefIcon type={ref.type} />
              </span>
              <span className="ref-chip-label">{ref.label}</span>
              <button
                type="button"
                className="ref-chip-remove"
                onClick={() => onRemoveReference(i)}
                aria-label={`移除引用: ${ref.label}`}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}

      {pendingImages.length > 0 && (
        <div className="ref-chips">
          {pendingImages.map((img, i) => (
            <span key={`img-${i}`} className="ref-img-pill">
              <span className="ref-img-pill-icon" aria-hidden="true">
                <IconImage size={14} />
              </span>
              <span className="ref-img-pill-name">
                {truncateName(img.name)}
              </span>
              <button
                type="button"
                className="ref-img-pill-remove"
                onClick={() => onRemoveImage(i)}
                aria-label={`移除图片: ${img.name}`}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}

      {pendingFiles.length > 0 && (
        <div className="ref-chips">
          {pendingFiles.map((f, i) => (
            <span key={`file-${i}`} className="ref-img-pill" title={f.path}>
              <span className="ref-img-pill-icon" aria-hidden="true">
                <IconFile size={14} />
              </span>
              <span className="ref-img-pill-name">
                {truncateName(f.name)}
              </span>
              <button
                type="button"
                className="ref-img-pill-remove"
                onClick={() => onRemoveFile(i)}
                aria-label={`移除文件: ${f.name}`}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  )
}