import React, { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as pdfjsLib from 'pdfjs-dist'
import { X, FolderOpen, ExternalLink, ChevronLeft, ChevronRight } from 'lucide-react'
import MarkdownContent from './MarkdownContent'
import { readFile, readFileBase64, openPath, revealPath } from '../lib/api'
import './preview-overlay.css'

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
/** 视频（preview:// 协议源，≤64MB；guess_mime 已覆盖 mp4/webm/mov） */
const VIDEO_EXTS = new Set(['mp4', 'webm', 'mov', 'mkv', 'avi', 'flv', 'ts', 'm4v'])
/** 音频（preview:// 协议源；guess_mime 已覆盖 mp3/wav/ogg/flac） */
const AUDIO_EXTS = new Set(['mp3', 'wav', 'ogg', 'flac', 'm4a', 'aac'])
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
  const m = /\\.([^.\\\\/]+)$/.exec(path)
  return m ? m[1].toLowerCase() : ''
}

export function baseName(path: string): string {
  const parts = path.split(/[\\\\/]/)
  return parts[parts.length - 1] || path
}

function base64ToUint8(b64: string): Uint8Array {
  const bin = atob(b64)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return bytes
}

/**
 * PDF 内联预览：pdf.js 渲染 canvas + 翻页。
 * 复用 core/pdf-render.ts 已全局设置的 GlobalWorkerOptions.workerSrc（main.tsx 导入）。
 * 数据源与图片一致（后端 readFileBase64 ≤8MB），不引入新的协议依赖。
 */
function PdfPreview({ path }: { path: string }) {
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [page, setPage] = useState(1)
  const [numPages, setNumPages] = useState(0)
  const taskRef = useRef<pdfjsLib.PDFDocumentLoadingTask | null>(null)
  const pdfRef = useRef<pdfjsLib.PDFDocumentProxy | null>(null)
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const wrapRef = useRef<HTMLDivElement | null>(null)

  // 渲染指定页到 canvas（宽度贴合容器，保持清晰度 scale 上限 2）
  const renderPage = async (pdf: pdfjsLib.PDFDocumentProxy, pageNum: number) => {
    const canvas = canvasRef.current
    if (!canvas) return
    const pageData = await pdf.getPage(pageNum)
    const baseViewport = pageData.getViewport({ scale: 1 })
    const containerWidth = wrapRef.current?.clientWidth || 800
    const scale = Math.min(containerWidth / baseViewport.width, 2)
    const viewport = pageData.getViewport({ scale })
    canvas.width = Math.floor(viewport.width)
    canvas.height = Math.floor(viewport.height)
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    await pageData.render({ canvas, canvasContext: ctx, viewport }).promise
  }

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setError(null)
    readFileBase64(path)
      .then(b64 => {
        if (cancelled || !b64) return null
        const task = pdfjsLib.getDocument({ data: base64ToUint8(b64) })
        taskRef.current = task
        return task.promise
      })
      .then(async pdf => {
        if (cancelled || !pdf) {
          taskRef.current?.destroy()
          return
        }
        pdfRef.current = pdf
        setNumPages(pdf.numPages)
        setPage(1)
        try {
          await renderPage(pdf, 1)
        } catch (e) {
          setError(e instanceof Error ? e.message : String(e))
        }
        setLoading(false)
      })
      .catch(e => {
        if (cancelled) return
        setError(typeof e === 'string' ? e : e instanceof Error ? e.message : String(e))
        setLoading(false)
      })
    return () => {
      cancelled = true
      pdfRef.current = null
      taskRef.current?.destroy()
      taskRef.current = null
    }
  }, [path])

  // 翻页重渲染
  useEffect(() => {
    const pdf = pdfRef.current
    if (!pdf || loading) return
    renderPage(pdf, page).catch(e => setError(e instanceof Error ? e.message : String(e)))
  }, [page, loading])

  const prev = () => setPage(p => Math.max(1, p - 1))
  const next = () => setPage(p => Math.min(numPages, p + 1))

  return (
    <div className="pv-pdf">
      <div className="pv-pdf-toolbar">
        <button
          type="button"
          className="pv-icon-btn"
          onClick={prev}
          disabled={page <= 1}
          title="上一页"
        >
          <ChevronLeft size={15} />
        </button>
        <span className="pv-pdf-pagenum">
          {page} / {numPages}
        </span>
        <button
          type="button"
          className="pv-icon-btn"
          onClick={next}
          disabled={page >= numPages}
          title="下一页"
        >
          <ChevronRight size={15} />
        </button>
      </div>
      <div className="pv-pdf-body" ref={wrapRef}>
        {loading ? (
          <div className="pv-loading">读取中…</div>
        ) : error ? (
          <div className="pv-error">
            <div className="pv-error-title">无法预览 PDF</div>
            <div className="pv-error-msg">{error}</div>
            <div className="pv-error-path">{path}</div>
          </div>
        ) : (
          <canvas ref={canvasRef} className="pv-pdf-canvas" />
        )}
      </div>
    </div>
  )
}

/**
 * 文件内容预览（无壳、可内嵌）。
 * 供 PreviewOverlay（全屏覆盖层）与 ToolsPage（同窗口内嵌面板）复用：
 * 按扩展名渲染 图片 / PDF（pdf.js 翻页）/ 视频 / 音频 / MD / HTML 沙箱 / 代码高亮，
 * 其余类型直接调系统默认程序打开（失败可见）。
 */
export function FilePreviewContent({ path }: { path: string }) {
  const ext = extOf(path)
  const isImage = IMAGE_EXTS.has(ext)
  const isPdf = ext === 'pdf'
  const isVideo = VIDEO_EXTS.has(ext)
  const isAudio = AUDIO_EXTS.has(ext)
  // HTML 经 preview:// 协议在沙箱 iframe 中运行（脚本可执行、同目录资源可引用），
  // 不走文本读取——src 直连协议 URL
  const isText = MD_EXTS.has(ext) || CODE_EXTS.has(ext)
  const [content, setContent] = useState<string | null>(null)
  const [imageData, setImageData] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(isText || isImage || isPdf)
  /** 系统打开/定位失败信息（此前被静默吞掉，表现为「点了打不开」无反馈） */
  const [openErr, setOpenErr] = useState<string | null>(null)

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
    if (!isText && !isPdf && !isVideo && !isAudio) {
      // 其余非文本类型（docx/xlsx 等）：不读内容，直接系统默认程序打开；失败必须可见
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

  return (
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
      ) : isPdf ? (
        <PdfPreview path={path} />
      ) : isVideo ? (
        <div className="pv-media">
          <video className="pv-video" src={convertFileSrc(path, 'preview')} controls autoPlay />
        </div>
      ) : isAudio ? (
        <div className="pv-media">
          <audio className="pv-audio" src={convertFileSrc(path, 'preview')} controls autoPlay />
        </div>
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
  )
}

/**
 * 全屏文件预览覆盖层（对齐工作流画布 .wfc-page 交互范式，非居中弹窗）。
 * - 关闭按钮在工具栏最左侧，防连续点击误触主窗口关闭
 * - 内容渲染复用 FilePreviewContent（图片/PDF/视频/音频/MD/HTML/代码）
 * - portal 到 body：调用方（外部 agent 状态栏）挂在 chat-input-dock 内，
 *   其祖先的 transform/backdrop-filter 会劫持 position:fixed 的定位基准，
 *   预览被压进输入框上方区域；portal 逃逸后 .pv-page 才是真正的全窗口覆盖
 */
export function PreviewOverlay({ path, onClose }: { path: string; onClose: () => void }) {
  // Esc 关闭
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  const handleReveal = () => {
    revealPath(path).catch(() => undefined)
  }
  const handleOpen = () => {
    openPath(path).catch(() => undefined)
  }

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

      <FilePreviewContent path={path} />
    </div>,
    document.body,
  )
}
