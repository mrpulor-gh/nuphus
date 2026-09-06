import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import type { ChatReference, PendingImage, PendingFile } from '../../core/types'
import {
  IconImage,
  IconFile,
  IconCamera,
  IconWrench,
  IconBrain,
  IconWorkflow,
} from '../../ui/Icons'
import '../../styles/ref-bar.css'

interface ReferenceBarProps {
  references: ChatReference[]
  pendingImages: PendingImage[]
  pendingFiles: PendingFile[]
  onRemoveReference: (index: number) => void
  onRemoveImage: (index: number) => void
  onRemoveFile: (index: number) => void
  /** 点击文档 chip 请求预览（父级接 PreviewOverlay，path = 本地文件路径） */
  onPreviewFile?: (path: string) => void
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

/** 文件类型标签：按扩展名归类为短大写标签（IMAGE/FILE/PDF/CODE…），未知归 FILE */
const FILE_TYPE_LABELS: Record<string, string> = {
  png: 'IMAGE',
  jpg: 'IMAGE',
  jpeg: 'IMAGE',
  gif: 'IMAGE',
  webp: 'IMAGE',
  bmp: 'IMAGE',
  svg: 'IMAGE',
  pdf: 'PDF',
  mp4: 'VIDEO',
  mov: 'VIDEO',
  avi: 'VIDEO',
  mkv: 'VIDEO',
  webm: 'VIDEO',
  mp3: 'AUDIO',
  wav: 'AUDIO',
  ogg: 'AUDIO',
  flac: 'AUDIO',
  m4a: 'AUDIO',
  md: 'MD',
  markdown: 'MD',
  txt: 'TXT',
  log: 'TXT',
  json: 'DATA',
  yaml: 'DATA',
  yml: 'DATA',
  toml: 'DATA',
  csv: 'DATA',
  html: 'HTML',
  htm: 'HTML',
  rs: 'CODE',
  ts: 'CODE',
  tsx: 'CODE',
  js: 'CODE',
  jsx: 'CODE',
  py: 'CODE',
  go: 'CODE',
  c: 'CODE',
  cpp: 'CODE',
  h: 'CODE',
  java: 'CODE',
  kt: 'CODE',
  rb: 'CODE',
  sh: 'CODE',
  ps1: 'CODE',
  sql: 'CODE',
}

function fileTypeLabel(name: string): string {
  const ext = (name.split('.').pop() || '').toLowerCase()
  return FILE_TYPE_LABELS[ext] || 'FILE'
}

export default function ReferenceBar({
  references,
  pendingImages,
  pendingFiles,
  onRemoveReference,
  onRemoveImage,
  onRemoveFile,
  onPreviewFile,
}: ReferenceBarProps) {
  /** 图片大图预览（dataUrl 轻量 lightbox；文档走父级 PreviewOverlay） */
  const [previewImg, setPreviewImg] = useState<PendingImage | null>(null)

  // Esc 关闭图片大图
  useEffect(() => {
    if (!previewImg) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setPreviewImg(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [previewImg])

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
            <div
              key={`img-${i}`}
              className="ref-img-pill ref-img-pill--clickable"
              title={`点击预览: ${img.name}`}
              onClick={() => setPreviewImg(img)}
            >
              <span className="ref-img-pill-icon" aria-hidden="true">
                <IconImage size={14} />
              </span>
              <span className="ref-img-pill-type">IMAGE</span>
              <span className="ref-img-pill-name">{truncateName(img.name)}</span>
              <button
                type="button"
                className="ref-img-pill-remove"
                onClick={e => {
                  e.stopPropagation()
                  onRemoveImage(i)
                }}
                aria-label={`移除图片: ${img.name}`}
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}

      {pendingFiles.length > 0 && (
        <div className="ref-chips">
          {pendingFiles.map((f, i) => {
            const clickable = !!onPreviewFile
            return (
              <div
                key={`file-${i}`}
                className={`ref-img-pill${clickable ? ' ref-img-pill--clickable' : ''}`}
                title={clickable ? `点击预览: ${f.path}` : f.path}
                onClick={
                  clickable
                    ? () => {
                        onPreviewFile!(f.path)
                      }
                    : undefined
                }
              >
                <span className="ref-img-pill-icon" aria-hidden="true">
                  <IconFile size={14} />
                </span>
                <span className="ref-img-pill-type">{fileTypeLabel(f.name)}</span>
                <span className="ref-img-pill-name">{truncateName(f.name)}</span>
                <button
                  type="button"
                  className="ref-img-pill-remove"
                  onClick={e => {
                    e.stopPropagation()
                    onRemoveFile(i)
                  }}
                  aria-label={`移除文件: ${f.name}`}
                >
                  ×
                </button>
              </div>
            )
          })}
        </div>
      )}

      {previewImg &&
        createPortal(
          <div
            className="ref-lightbox"
            onClick={() => setPreviewImg(null)}
            onContextMenu={e => e.stopPropagation()}
          >
            <img src={previewImg.dataUrl} alt={previewImg.name} />
            <div className="ref-lightbox-footer">
              <span className="ref-lightbox-name">{previewImg.name}</span>
              <button type="button" onClick={() => setPreviewImg(null)}>
                关闭
              </button>
            </div>
          </div>,
          document.body,
        )}
    </div>
  )
}
