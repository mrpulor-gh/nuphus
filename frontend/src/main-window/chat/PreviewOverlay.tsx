import React, { useEffect, useState } from 'react'
import { X, FolderOpen, ExternalLink } from 'lucide-react'
import MarkdownContent from './MarkdownContent'
import { readFile, readFileBase64, openPath, revealPath } from '../lib/api'
import './preview-overlay.css'

interface PreviewOverlayProps {
  path: string
  onClose: () => void
}

const MD_EXTS = new Set(['md'])
const HTML_EXTS = new Set(['html', 'htm'])
const CODE_EXTS = new Set([
  'rs',
  'ts',
  'tsx',
  'js',
  'jsx',
  'py',
  'json',
  'toml',
  'css',
  'yml',
  'yaml',
  'sh',
])
/** 支持内联预览的图片扩展名（后端 base64 读取，CSP 已允许 data:） */
const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg', 'ico'])
const IMAGE_MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
  svg: 'image/svg+xml',
  ico: 'image/x-icon',
}

function extOf(path: string): string {
  const m = /\.([^.\\/]+)$/.exec(path)
  return m ? m[1].toLowerCase() : ''
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/**
 * 全屏文件预览覆盖层（对齐工作流画布 .wfc-page 交互范式，非居中弹窗）。
 * - 关闭按钮在工具栏最左侧，防连续点击误触主窗口关闭
 * - .md 排版渲染、.html 用 iframe 渲染成页面、代码文件复用 MarkdownContent 高亮
 * - 图片（png/jpg/gif/webp/svg 等）经后端 base64 读取后内联渲染
 * - pdf 等不支持内联预览 → 直接调系统默认程序打开
 */
export function PreviewOverlay({ path, onClose }: PreviewOverlayProps) {
  const ext = extOf(path)
  const isImage = IMAGE_EXTS.has(ext)
  const isText = MD_EXTS.has(ext) || HTML_EXTS.has(ext) || CODE_EXTS.has(ext)
  const [content, setContent] = useState<string | null>(null)
  const [imageData, setImageData] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(isText || isImage)

  // Esc 关闭
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  // 读取内容
  useEffect(() => {
    if (isImage) {
      let cancelled = false
      setLoading(true)
      setError(null)
      readFileBase64(path)
        .then(b64 => {
          if (cancelled) return
          setImageData(b64)
          setLoading(false)
        })
        .catch(err => {
          if (cancelled) return
          setError(typeof err === 'string' ? err : String(err))
          setLoading(false)
        })
      return () => {
        cancelled = true
      }
    }
    if (!isText) {
      // 非文本类型（pdf 等）：不读内容，直接系统默认程序打开
      openPath(path).catch(() => {})
      return
    }
    let cancelled = false
    setLoading(true)
    setError(null)
    readFile(path)
      .then(text => {
        if (cancelled) return
        setContent(text)
        setLoading(false)
      })
      .catch(err => {
        if (cancelled) return
        setError(typeof err === 'string' ? err : String(err))
        setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [path, isText])

  const handleReveal = () => {
    revealPath(path).catch(() => {})
  }
  const handleOpen = () => {
    openPath(path).catch(() => {})
  }

  return (
    <div className="pv-page">
      {/* ── 工具栏：关闭按钮在最左（与画布一致），spacer 之后才是右侧操作 ── */}
      <div className="pv-toolbar">
        <button type="button" className="pv-icon-btn" onClick={onClose} title="关闭预览">
          <X size={15} />
        </button>
        <span className="pv-title" title={path}>
          {baseName(path)}
        </span>
        <div className="pv-toolbar-spacer" />
        <button type="button" className="pv-btn" onClick={handleReveal} title="在文件管理器中定位">
          <FolderOpen size={13} /> 在文件夹显示
        </button>
        <button type="button" className="pv-btn" onClick={handleOpen} title="用系统默认程序打开">
          <ExternalLink size={13} /> 系统打开
        </button>
      </div>

      {/* ── 主体 ── */}
      <div className="pv-body">
        {isImage ? (
          loading ? (
            <div className="pv-loading">读取中…</div>
          ) : error ? (
            <div className="pv-error">
              <div className="pv-error-title">无法预览此文件</div>
              <div className="pv-error-msg">{error}</div>
              <div className="pv-error-path">{path}</div>
              <button type="button" className="pv-btn" onClick={handleOpen}>
                <ExternalLink size={13} /> 系统打开
              </button>
            </div>
          ) : (
            <div className="pv-image-wrap">
              <img
                className="pv-image"
                src={`data:${IMAGE_MIME[ext]};base64,${imageData ?? ''}`}
                alt={baseName(path)}
              />
            </div>
          )
        ) : !isText ? (
          <div className="pv-placeholder">
            <div className="pv-placeholder-title">已用系统默认程序打开</div>
            <div className="pv-placeholder-path">{path}</div>
            <div className="pv-placeholder-hint">
              该类型不支持内联预览（pdf 等），已调用系统默认程序打开。
            </div>
          </div>
        ) : loading ? (
          <div className="pv-loading">读取中…</div>
        ) : error ? (
          <div className="pv-error">
            <div className="pv-error-title">无法预览此文件</div>
            <div className="pv-error-msg">{error}</div>
            <div className="pv-error-path">{path}</div>
            <button type="button" className="pv-btn" onClick={handleOpen}>
              <ExternalLink size={13} /> 系统打开
            </button>
          </div>
        ) : MD_EXTS.has(ext) ? (
          <div className="pv-md">
            <MarkdownContent content={content ?? ''} />
          </div>
        ) : HTML_EXTS.has(ext) ? (
          <iframe
            className="pv-iframe"
            title={path}
            srcDoc={content ?? ''}
            sandbox="allow-scripts"
          />
        ) : (
          <div className="pv-code">
            <MarkdownContent content={`\`\`\`${ext}\n${content ?? ''}\n\`\`\``} />
          </div>
        )}
      </div>
    </div>
  )
}
