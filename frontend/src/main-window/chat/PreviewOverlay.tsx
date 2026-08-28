import React, { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { convertFileSrc } from '@tauri-apps/api/core'
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
  'txt',
  'log',
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
 * - .md 排版渲染、.html 经 preview:// 协议在沙箱 iframe 运行（游戏/交互 demo 可玩），
 *   代码文件复用 MarkdownContent 高亮
 * - 图片（png/jpg/gif/webp/svg 等）经后端 base64 读取后内联渲染
 * - pdf 等不支持内联预览 → 直接调系统默认程序打开
 */
export function PreviewOverlay({ path, onClose }: PreviewOverlayProps) {
  const ext = extOf(path)
  const isImage = IMAGE_EXTS.has(ext)
  // HTML 经 preview:// 协议在沙箱 iframe 中运行（脚本可执行、同目录资源可引用），
  // 不走文本读取——src 直连协议 URL
  const isText = MD_EXTS.has(ext) || CODE_EXTS.has(ext)
  const [content, setContent] = useState<string | null>(null)
  const [imageData, setImageData] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(isText || isImage)
  /** 系统打开/定位失败信息（此前被静默吞掉，表现为「点了打不开」无反馈） */
  const [openErr, setOpenErr] = useState<string | null>(null)

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
    setOpenErr(null)
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
      // 非文本类型（pdf 等）：不读内容，直接系统默认程序打开；失败必须可见
      let cancelled = false
      openPath(path).catch(err => {
        if (!cancelled) setOpenErr(typeof err === 'string' ? err : String(err))
      })
      return () => {
        cancelled = true
      }
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
    setOpenErr(null)
    revealPath(path).catch(err => setOpenErr(typeof err === 'string' ? err : String(err)))
  }
  const handleOpen = () => {
    setOpenErr(null)
    openPath(path).catch(err => setOpenErr(typeof err === 'string' ? err : String(err)))
  }

  // portal 到 body：调用方（外部 agent 状态栏）挂在 chat-input-dock 内，
  // 其祖先的 transform/backdrop-filter 会劫持 position:fixed 的定位基准，
  // 预览被压进输入框上方区域；portal 逃逸后 .pv-page 才是真正的全窗口覆盖
  return createPortal(
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

      {/* ── 系统打开/定位失败提示（不再静默吞错） ── */}
      {openErr && (
        <div className="pv-open-error" role="alert">
          <span className="pv-open-error-msg">{openErr}</span>
          <button type="button" className="pv-btn" onClick={handleReveal}>
            <FolderOpen size={13} /> 在文件夹显示
          </button>
        </div>
      )}

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
          openErr ? (
            <div className="pv-error">
              <div className="pv-error-title">无法打开此文件</div>
              <div className="pv-error-msg">{openErr}</div>
              <div className="pv-error-path">{path}</div>
              <button type="button" className="pv-btn" onClick={handleReveal}>
                <FolderOpen size={13} /> 在文件夹显示
              </button>
            </div>
          ) : (
            <div className="pv-placeholder">
              <div className="pv-placeholder-title">已请求系统默认程序打开</div>
              <div className="pv-placeholder-path">{path}</div>
              <div className="pv-placeholder-hint">
                该类型不支持内联预览（pdf 等），已调用系统默认程序打开；若未弹出窗口请查看上方提示。
              </div>
            </div>
          )
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
            src={convertFileSrc(path, 'preview')}
            sandbox="allow-scripts allow-same-origin allow-pointer-lock allow-modals allow-forms"
          />
        ) : (
          <div className="pv-code">
            <MarkdownContent content={`\`\`\`${ext}\n${content ?? ''}\n\`\`\``} />
          </div>
        )}
      </div>
    </div>,
    document.body,
  )
}