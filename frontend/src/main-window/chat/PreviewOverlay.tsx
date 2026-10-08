import React, { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as pdfjsLib from 'pdfjs-dist'
import {
  X,
  FolderOpen,
  ExternalLink,
  ChevronLeft,
  ChevronRight,
  TriangleAlert,
  Pencil,
  Crosshair,
  Send,
  CircleX,
} from 'lucide-react'
import MarkdownContent from './MarkdownContent'
import { readFile, readFileBase64, openPath, revealPath, hudUpdate } from '../lib/api'
import {
  type AnnotationItem,
  type AnnotationMessage,
  buildAnnotationRef,
} from './annotationPayload'
import type { ChatReference } from '../../core/types'
import { useLanguage } from '../../locales'
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

/**
 * P3 持久化：标注快照按预览文件路径存**父窗口** localStorage。
 *
 * 为什么不在 iframe 内存：iframe 经 preview:// 协议加载（http://preview.localhost/…），
 * 与 app 主窗口不同源——沙箱允许写自己的 localStorage，但父窗口读取不可达，键也随
 * 每次文件路径漂移。父窗口存储 + 握手后经 `nuphus:annotations-restore` 指令下发
 * （iframe 内 selector 回定位 DOM）才是闭环。发送/放弃即清库，与 overlay 侧清 markers
 * 语义对齐（刷新不再恢复已处理完的标注）。
 */
const ANNOTATION_LS_PREFIX = 'nuphus:annotations:'

function annotationLsKey(path: string): string {
  return ANNOTATION_LS_PREFIX + path
}

function readAnnotationStore(path: string): AnnotationItem[] | null {
  try {
    const raw = localStorage.getItem(annotationLsKey(path))
    if (!raw) return null
    const parsed = JSON.parse(raw) as AnnotationItem[]
    return Array.isArray(parsed) ? parsed : null
  } catch (err) {
    // 存储损坏 / 隐私模式：不阻塞预览，仅报错（best-effort）
    console.error('[nuphus-annotator] 读取标注历史失败', err)
    return null
  }
}

function writeAnnotationStore(path: string, msg: AnnotationMessage): void {
  try {
    if (msg.annotations.length === 0) {
      // 空快照 = overlay 侧 markers 已清空（发送/删除到零）→ 清库
      localStorage.removeItem(annotationLsKey(path))
      return
    }
    localStorage.setItem(annotationLsKey(path), JSON.stringify(msg.annotations))
  } catch (err) {
    console.error('[nuphus-annotator] 写入标注历史失败', err)
  }
}

function clearAnnotationStore(path: string): void {
  try {
    localStorage.removeItem(annotationLsKey(path))
  } catch (err) {
    console.error('[nuphus-annotator] 清除标注历史失败', err)
  }
}

/**
 * P4 父窗口 → iframe 下行指令（协议面仅增不改；type 与 preview_protocol.rs 钉死）。
 *
 * 共享一条 postMessage 通道 + 同一条 source 铁律（只发给本 iframe 的 contentWindow）：
 * - `nuphus:annotate-mode`    编辑态开关（P4 的唯一入口在父窗口，iframe 内无任何按钮）
 * - `nuphus:annotate-focus`   列表项 ◎ 定位闪烁（iframe 按 selector 反查自己的 markers）
 * - `nuphus:annotate-comment` 列表项批注回写（iframe 侧 markers 是数据的真正持有方）
 * - `nuphus:annotate-remove`  列表项单条删除
 * - `nuphus:annotate-theme`   主题令牌下发（iframe 读不到父窗口 CSS 变量，唯一干净通道）
 */
export type AnnotateCommand =
  | { type: 'nuphus:annotate-mode'; on: boolean }
  | { type: 'nuphus:annotate-focus'; selector: string; seq: number }
  | { type: 'nuphus:annotate-comment'; index: number; comment: string }
  | { type: 'nuphus:annotate-remove'; index: number }
  | { type: 'nuphus:annotate-clear-all' }
  | { type: 'nuphus:annotate-theme'; tokens: AnnotationThemeTokens }

/**
 * 标注器配色令牌（从父窗口 CSS 变量读出后下发）。
 *
 * 为什么必须走 postMessage 而不是让脚本自己读：preview:// 文档运行在与主窗口
 * 不同源的沙箱里，`var(--accent)` 在 iframe 内取不到主窗口的值，只能靠硬编码
 * ——而硬编码会让标注器配色与主题脱钩（切简白/深色/tech 后标注器仍是旧色），
 * 且把某套配色焊死在脚本里。父窗口读自己的 token 下发是唯一让标注器
 * 「跟主题走」的做法：三主题自动适配，脚本与 token 体系零耦合。
 */
export interface AnnotationThemeTokens {
  /** --accent（角标/高亮主色）*/
  accent: string
  /** --on-accent（角标上的文字色）*/
  onAccent: string
  /** --warning（区域框选/橡皮筋色）*/
  warning: string
  /** 角标投影（rgba 字符串，来自 --shadow-elevated 或直接用阴影色）*/
  shadow: string
  /** 预览页滚动条 thumb 色（页面滚动条在 iframe 内，父窗口 CSS 管不到）*/
  scrollThumb: string
  /** 滚动条 hover 色*/
  scrollThumbHover: string
}

/** 读当前主题的标注器配色（覆盖层本身在主窗口文档内，直接读它的 computed style）*/
function readAnnotationThemeTokens(el: HTMLElement | null): AnnotationThemeTokens {
  const cs = getComputedStyle(el ?? document.documentElement)
  const pick = (name: string, fallback: string) => {
    const v = cs.getPropertyValue(name).trim()
    return v || fallback
  }
  //角标投影：从 --shadow-elevated 里摘不出单独色值（它是多层 box-shadow），
  // 故用 --accent 的 rgb 三元组（tokens.css 每主题都有）叠一个低透明黑，
  // 视觉等价且随主题走；两档主题都没有 rgb 三元组时回落固定值。
  const accentRgb = pick('--accent-rgb', '')
  const shadow = accentRgb ? `rgba(${accentRgb}, 0.45)` : 'rgba(0, 0, 0, 0.45)'
  // 滚动条：主窗口约定 5px 细条 + 极低透明（tokens.css 的 --line-2/--line-3 语义）
  const line2 = pick('--line-2', 'rgba(128,128,128,0.35)').trim()
  const line3 = pick('--line-3', 'rgba(128,128,128,0.55)').trim()
  return {
    accent: pick('--accent', '#3b82f6'),
    onAccent: pick('--on-accent', '#ffffff'),
    warning: pick('--warning', '#f59e0b'),
    shadow,
    scrollThumb: line2,
    scrollThumbHover: line3,
  }
}

/** 标注器上行消息的 type 白名单（与 preview_protocol.rs 注入脚本一一对应）*/
const ANNOTATOR_UP_TYPES = new Set([
  'nuphus:annotator-ready',
  'nuphus:annotations',
  'nuphus:annotate-exit',
])

/**
 * 消息形状校验（source 校验的第一道，也是放宽 source 后的安全兜底）。
 *
 * 2026-10-08：source 恒等比对在 preview:// 沙箱下不可靠（见 handler 内注释），
 * 放宽后必须用形状把「非标注器的消息」挡在门外——type 必须命中白名单，
 * 且 `nuphus:annotations` 的annotations 必须是数组、每条字段类型正确。
 * 这样即便别的窗口/扩展往主窗口 postMessage 相同形状的数据，也进不了标注状态机。
 */
function isAnnotatorMessage(data: { type?: unknown; annotations?: unknown }): boolean {
  if (typeof data.type !== 'string' || !ANNOTATOR_UP_TYPES.has(data.type)) return false
  if (data.type === 'nuphus:annotations') {
    if (!Array.isArray(data.annotations)) return false
    return data.annotations.every(
      (a: unknown) =>
        typeof a === 'object' &&
        a !== null &&
        typeof (a as { css_selector?: unknown }).css_selector === 'string' &&
        typeof (a as { outer_html_snippet?: unknown }).outer_html_snippet === 'string' &&
        typeof (a as { comment?: unknown }).comment === 'string',
    )
  }
  return true
}

function sendAnnotateCommand(frame: HTMLIFrameElement | null, cmd: AnnotateCommand): void {
  const contentWindow = frame?.contentWindow
  if (!contentWindow) return
  contentWindow.postMessage(cmd, '*')
}

function extOf(path: string): string {
  // Windows paths use backslashes; only the final dot introduces the extension.
  const m = /\.([^.\\/]+)$/.exec(path)
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
export function FilePreviewContent({
  path,
  iframeRef,
  onIframeLoad,
}: {
  path: string
  /**
   * HTML 预览桥（可选）：仅 PreviewOverlay 全屏预览传入——iframe 引用 + onLoad
   * 钩子，供父窗口在 iframe 就绪后注册 postMessage listener 收标注回流。
   * ToolsPage 内嵌面板不传，行为不变。
   */
  iframeRef?: React.RefObject<HTMLIFrameElement>
  onIframeLoad?: () => void
}) {
  const ext = extOf(path)
  const isImage = IMAGE_EXTS.has(ext)
  const isPdf = ext === 'pdf'
  const isVideo = VIDEO_EXTS.has(ext)
  const isAudio = AUDIO_EXTS.has(ext)
  const isText = MD_EXTS.has(ext) || CODE_EXTS.has(ext)
  // HTML 经 preview:// 协议在沙箱 iframe 中运行（脚本可执行、同目录资源可引用），
  // 不走文本读取——src 直连协议 URL。
  // ⚠️ isHtml 必须与 isText 一起参与下方「非文本类型 → 系统默认程序打开」的判定：
  // 漏掉它，html/htm 会在这里被甩给系统默认程序，文件末尾的 iframe 分支成死代码
  // （2026-09-20 修：该分支自 preview:// 底座上线起就不可达，html 永远落到
  // 「已请求系统默认程序打开」占位）。
  const isHtml = HTML_EXTS.has(ext)
  const [content, setContent] = useState<string | null>(null)
  const [imageData, setImageData] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(isText || isImage || isPdf)
  /** 系统打开/定位失败信息（此前被静默吞掉，表现为「点了打不开」无反馈） */
  const [openErr, setOpenErr] = useState<string | null>(null)

  // 读取内容
  useEffect(() => {
    setOpenErr(null)
    // HTML 由 preview:// iframe 直渲染：不读内容、不进 loading 态，也不许落到下方
    // 「非文本 → 系统默认程序打开」或「文本读取」任一分支（2026-09-20 修——漏掉这道
    // 早退，html 会撞进文本读取分支卡在「读取中…」）。
    if (isHtml) return
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
    if (!isText && !isHtml && !isPdf && !isVideo && !isAudio) {
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
  }, [path, isText, isHtml])

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
      ) : !isText && !isHtml ? (
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
          ref={iframeRef}
          className="pv-iframe"
          title={path}
          src={convertFileSrc(path, 'preview')}
          sandbox="allow-scripts allow-same-origin allow-pointer-lock allow-modals allow-forms"
          onLoad={onIframeLoad}
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
/**
 * 标注器握手诊断（红条展开时展示，2026-10-08）。
 *
 * 为什么需要：红条原本只有一句「注入失败或页面异常」，用户无法判断断在哪一层——
 * 是脚本没执行、消息被 source 拦了、还是形状不匹配。诊断把这三层的判定
 * 逐条记录下来（计数 + 最后一次拒绝原因），红条点开即可自证断点。
 */
interface AnnotatorDiag {
  /** iframe 是否已触发 onLoad */
  frameLoaded: boolean
  /** 收到过任意 message 事件（不论是否被接受）*/
  messagesSeen: number
  /** 通过形状校验的消息数 */
  shapeOk: number
  /** 形状对了但 source 不匹配被丢弃数 */
  rejectedBySource: number
  /** 形状不匹配被丢弃数 */
  rejectedByShape: number
  /** 最后一次拒绝的原因（人话）*/
  lastReject: string | null
  /** 握手是否成功 */
  readySeen: boolean
}

/** 新鲜诊断（每次 iframe 重载/切文件重置）*/
function freshDiag(): AnnotatorDiag {
  return {
    frameLoaded: false,
    messagesSeen: 0,
    shapeOk: 0,
    rejectedBySource: 0,
    rejectedByShape: 0,
    lastReject: null,
    readySeen: false,
  }
}

export function PreviewOverlay({
  path,
  onClose,
  onSendAnnotations,
}: {
  path: string
  onClose: () => void
  /**
   *  * 「发送给 Agent」出口（P2）：把标注引用 chip 交给对话输入链路（ChatPanel 的
   *  * addReference）。不传 = 当前宿主不支持进对话（如外部 agent 状态栏内嵌预览），
   *  * 发送按钮禁用并提示——绝不让用户点了没反应。
   *  */
  onSendAnnotations?: (ref: ChatReference) => void
}) {
  // UI 文案一律走 i18n（禁中文硬编码：产品内agent 的展示名由 locale 决定，
  // 前端不得假设它叫「丞相」——那只是某个部署里的自定义称呼）
  const { t } = useLanguage()
  // Esc 关闭
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [onClose])

  /** ── HTML 标注回流（P1：postMessage 接收；P2：发送给丞相；P3：持久化；P4：父窗口 UI）──
   *  listener 注册在 iframe onLoad 之后：此刻 contentWindow 稳定，正好作为
   *  source 校验基准。只认本 iframe 发来的消息，伪造 source 一律忽略。 */
  const iframeRef = useRef<HTMLIFrameElement | null>(null)
  const [annotationMsg, setAnnotationMsg] = useState<AnnotationMessage | null>(null)
  /** 标注器握手：pending=等握手 2s；ready=已握手；missing=超时未握手（注入失败/页面异常） */
  const [annotatorStatus, setAnnotatorStatus] = useState<'pending' | 'ready' | 'missing' | null>(
    null,
  )
  /** 握手诊断（红条展开时展示断点证据，见 AnnotatorDiag）*/
  const [annotatorDiag, setAnnotatorDiag] = useState<AnnotatorDiag>(freshDiag)
  /** 握手诊断的 ref 镜像：message handler 只在 onLoad 注册一次，靠 ref 累加计数
   *  而不重挂 listener（与 annotatingRef / annotationCountRef 同思路）。 */
  const annotatorDiagRef = useRef<AnnotatorDiag>(freshDiag())
  /** 诊断面板展开态（红条默认收起，点「诊断详情」才展开——默认不干扰正常预览）*/
  const [diagOpen, setDiagOpen] = useState(false)
  /** 本次预览从 localStorage 恢复了几条历史标注（面板标题角标告知，best-effort） */
  const [restoredCount, setRestoredCount] = useState(0)
  /** P4 编辑态：父窗口是唯一入口（iframe 内零 UI）。默认 false = 页面原生交互全可用 */
  const [annotating, setAnnotating] = useState(false)
  /** 批注输入焦点：指向列表项 textarea 的 ref map（P4 批注编辑迁到父窗口） */
  const commentRefs = useRef<Map<number, HTMLTextAreaElement | null>>(new Map())
  /** 定位序列号：同一 selector 连续 ◎ 也让 iframe 侧动画重启 */
  const focusSeqRef = useRef(0)
  /** 快照条数与编辑态的 ref 镜像：message handler 注册在 iframe onLoad（只跑一次），
   *  读不到当次的 state 值，用 ref 保持最新又不重挂 listener。 */
  const annotationCountRef = useRef(0)
  const annotatingRef = useRef(false)
  const frameCleanupRef = useRef<(() => void) | null>(null)
  // 关闭覆盖层 / 切换预览文件：拆掉上一条 listener 并清空旧载荷与握手状态，
  // 防上一个 iframe 的消息串话、也避免旧文件的标注残留在新预览上
  useEffect(() => {
    setAnnotationMsg(null)
    setAnnotatorStatus(null)
    setRestoredCount(0)
    // 切换文件时诊断一并重置，避免上一个文件的判定结果误导本次排查
    annotatorDiagRef.current = freshDiag()
    setAnnotatorDiag(annotatorDiagRef.current)
    // P4：编辑态也随文件重置（iframe 已经换了，旧编辑态指令没有意义）
    setAnnotating(false)
    annotationCountRef.current = 0
    annotatingRef.current = false
    commentRefs.current.clear()
    return () => frameCleanupRef.current?.()
  }, [path])

  // P4 编辑态镜像到 ref：message handler 只在 iframe onLoad 时注册一次，
  // 靠 ref 读到最新编辑态而不必重挂 listener
  useEffect(() => {
    annotatingRef.current = annotating
  }, [annotating])

  /** P4：编辑入口与修改列表只对 HTML 预览有意义（注入只发生在 HTML 文档）。 */
  const isHtmlPreview = HTML_EXTS.has(extOf(path))
  const annotations = annotationMsg?.annotations ?? []
  const annotationCount = annotations.length
  /** 待发送数：当前载荷条数；无快照时用恢复条数兜底（徽标在收起态仍提示「还没发」） */
  const pendingCount = annotationCount > 0 ? annotationCount : restoredCount

  /** P4 入口：✎ 进/退编辑态。编辑态下 iframe 侧 capture 拦截页面点击/跳转。 */
  const toggleAnnotating = () => {
    const next = !annotating
    annotatingRef.current = next
    setAnnotating(next)
    sendAnnotateCommand(iframeRef.current, { type: 'nuphus:annotate-mode', on: next })
    if (next) hudUpdate(t('preview.annotateEntered'), 'done')
  }

  /** P4 批注回写：数据真正持有方是 iframe 侧 markers，这里只下发 + 由快照回流落库。 */
  const handleCommentChange = (index: number, comment: string) => {
    sendAnnotateCommand(iframeRef.current, {
      type: 'nuphus:annotate-comment',
      index,
      comment,
    })
  }

  /** P4 单条删除：下发 → iframe 摘 marker + 重排序号 → 快照回流刷新列表。 */
  const handleRemoveAnnotation = (index: number) => {
    sendAnnotateCommand(iframeRef.current, { type: 'nuphus:annotate-remove', index })
  }

  /** P4 定位：按 selector 让 iframe 侧闪 3 轮（宿主 DOM 零写入，纯 overlay 视觉）。 */
  const handleLocateAnnotation = (item: AnnotationItem) => {
    focusSeqRef.current += 1
    sendAnnotateCommand(iframeRef.current, {
      type: 'nuphus:annotate-focus',
      selector: item.css_selector,
      seq: focusSeqRef.current,
    })
  }

  const handleFrameLoad = () => {
    frameCleanupRef.current?.()
    const frame = iframeRef.current
    if (!frame?.contentWindow) return
    // 诊断 ref 与 state 同步重置（ref 供 handler 累加，state 供渲染）
    annotatorDiagRef.current = { ...freshDiag(), frameLoaded: true }
    setAnnotatorDiag({ ...annotatorDiagRef.current })
    setAnnotatorStatus('pending')
    let readySeen = false
    const handler = (event: MessageEvent) => {
      // ── 诊断埋点：逐层记录判定结果（红条展开时可见，见 AnnotatorDiag）──
      const diag = annotatorDiagRef.current
      diag.messagesSeen += 1
      // ── 防伪校验（2026-10-08 实机「标注器未就绪」修复）──
      //
      // 原实现把 contentWindow 捕获在 onLoad 那一刻做 `event.source !== contentWindow`
      // 恒等比对，实机握手 100% 被拦。原因是 preview:// 文档在 onLoad 之后仍可能
      // 发生一次导航/代理重建，iframe.contentWindow 指向**新的** WindowProxy，
      // 而 iframe 内 window.parent.postMessage 的 event.source 是它自己的 WindowProxy
      // ——两个 proxy 对象在沙箱+自定义协议下不保证是同一 JS 对象身份。
      //
      // 放宽为两段式，仍保留防伪能力：
      //   ① 消息形状必须严格匹配本协议（type 白名单 + 载荷结构校验）；
      //   ② event.source 必须是「某个」iframe 的 contentWindow —— 用
      //      [...iframe 集合].some(f => f.contentWindow === event.source)，
      //      对比实时取到的 contentWindow，绕开「捕获时快照」的失效问题。
      // 若 source 完全对不上任何 iframe（如别的窗口/扩展注入），仍然丢弃。
      const data = event.data as { type?: unknown; annotations?: unknown } | null | undefined
      if (!data || typeof data !== 'object') {
        diag.rejectedByShape += 1
        diag.lastReject = `非对象消息（${String(event.data).slice(0, 30)}）`
        setAnnotatorDiag({ ...diag })
        return
      }
      if (!isAnnotatorMessage(data)) {
        diag.rejectedByShape += 1
        diag.lastReject = `消息形状不匹配（type=${String(data.type)}）`
        setAnnotatorDiag({ ...diag })
        return
      }
      const fromOurFrame =
        frame.contentWindow === event.source ||
        (typeof window !== 'undefined' &&
          Array.from(document.querySelectorAll('iframe')).some(
            f => f.contentWindow && f.contentWindow === event.source,
          ))
      if (!fromOurFrame) {
        diag.rejectedBySource += 1
        diag.lastReject = '消息来源不是本预览 iframe（被防伪校验拦截）'
        setAnnotatorDiag({ ...diag })
        return
      }
      diag.shapeOk += 1
      const contentWindow = frame.contentWindow
      if (!contentWindow) return
      if (data.type === 'nuphus:annotator-ready') {
        readySeen = true
        diag.readySeen = true
        setAnnotatorStatus('ready')
        setAnnotatorDiag({ ...diag })
        // 先下发主题令牌再谈恢复：标注器配色必须来自当前主题（父窗口 token），
        // 否则角标/高亮会用脚本兜底色，与界面 accent 不一致。
        contentWindow.postMessage(
          { type: 'nuphus:annotate-theme', tokens: readAnnotationThemeTokens(null) },
          '*',
        )
        // P3 持久化恢复：握手后把同文件的历史标注下发 iframe（selector 回定位 DOM）
        const restored = readAnnotationStore(path)
        if (restored && restored.length > 0) {
          contentWindow.postMessage(
            { type: 'nuphus:annotations-restore', annotations: restored },
            '*',
          )
          setRestoredCount(restored.length)
          hudUpdate(t('preview.restored', String(restored.length)), 'done')
        }
        // 父窗口持有编辑态：握手后补发一次当前态（iframe 载入前点的 ✎ 不能丢）
        contentWindow.postMessage({ type: 'nuphus:annotate-mode', on: annotatingRef.current }, '*')
        return
      }
      // P4：iframe 内按 Esc → 请求父窗口退出编辑态（焦点在 iframe 内，父窗口收不到 keydown）
      if (data.type === 'nuphus:annotate-exit') {
        if (annotatingRef.current) {
          setAnnotating(false)
          annotatingRef.current = false
        }
        return
      }
      if (data.type === 'nuphus:annotations' && Array.isArray(data.annotations)) {
        const msg = event.data as AnnotationMessage
        const prevCount = annotationCountRef.current
        setAnnotationMsg(msg)
        annotationCountRef.current = msg.annotations.length
        // markers 每次变更广播的同型快照 → 落父窗口 localStorage（发送/放弃时清）
        writeAnnotationStore(path, msg)
        // P4：新增标记时把焦点交给父窗口列表里新增的那一条，用户可以直接写批注
        // （iframe 侧已无批注框；列表 textarea 是唯一的批注输入口）。
        // 只认「编辑态下新增长」——恢复历史（前置 snapshot）与删除（数量减少）都不抢焦点。
        if (annotatingRef.current && msg.annotations.length > prevCount) {
          const idx = msg.annotations.length - 1
          window.setTimeout(() => commentRefs.current.get(idx)?.focus(), 0)
        }
      }
    }
    window.addEventListener('message', handler)
    // 握手兜底：onLoad 后 2s 未收到 ready → 提示标注器未就绪（best-effort，常见于
    // overlay 注入脚本被宿主页面清理 / 页面自身脚本异常）
    const readyTimer = window.setTimeout(() => {
      if (!readySeen) setAnnotatorStatus('missing')
    }, 2000)
    frameCleanupRef.current = () => {
      window.removeEventListener('message', handler)
      window.clearTimeout(readyTimer)
    }
  }

  /** ── P2：发送给 Agent ──
   *  序列化载荷 → quote 引用 chip 进输入链路（addReference 去重），清库 + 退出编辑态。
   *  P4：iframe 侧发送入口已删，发送是父窗口的显式裁决。
   *  与「放弃」同源修复：清库的同时必须让iframe **清数据**（annotate-clear-all），
   *  否则 markers 留在 iframe 里，下次进编辑态旧标记又回来（2026-10-08 大王报障）。 */
  const handleSendAnnotations = () => {
    if (!annotationMsg || !onSendAnnotations) return
    const ref = buildAnnotationRef(annotationMsg)
    if (!ref) return
    onSendAnnotations(ref)
    const sent = annotationMsg.annotations.length
    // 先清 iframe 数据（clear-all 会 postSnapshot 回流空数组），再退编辑态
    sendAnnotateCommand(iframeRef.current, { type: 'nuphus:annotate-clear-all' })
    annotatingRef.current = false
    setAnnotating(false)
    sendAnnotateCommand(iframeRef.current, { type: 'nuphus:annotate-mode', on: false })
    setAnnotationMsg(null)
    setRestoredCount(0)
    annotationCountRef.current = 0
    commentRefs.current.clear()
    // 发送后清库：与 overlay 侧清markers 对齐，刷新不再恢复已处理的标注
    clearAnnotationStore(path)
    hudUpdate(t('preview.sentToInput', String(sent)), 'done')
  }

  /** 放弃：清 iframe 内标记 + 清当前载荷 + 清库 + 可见反馈（不静默丢弃用户操作）。
   *  P4 起「放弃」改由父窗口裁决：先下发 annotate-clear-all 让 iframe **清数据**
   *  （此前只清 localStorage + React state，iframe 的 markers 仍在 → 再次进编辑态
   *  旧标记全回来了，大王 2026-10-08 报障），再下发 annotate-mode(false) 清视觉。
   *  注意：不能用 annotate-remove 逐条删——那会给用户「已删」的错觉但实际是全清。 */
  const handleDiscardAnnotations = () => {
    if (annotationCount === 0) return
    hudUpdate(t('preview.discarded', String(annotationCount)), 'warning')
    // 顺序要紧：先清 iframe 数据（clear-all 内部会 postSnapshot 回流空数组），
    // 再退编辑态；若反了，mode(false) 只清视觉，数据留着又会回来。
    sendAnnotateCommand(iframeRef.current, { type: 'nuphus:annotate-clear-all' })
    annotatingRef.current = false
    setAnnotating(false)
    sendAnnotateCommand(iframeRef.current, { type: 'nuphus:annotate-mode', on: false })
    setAnnotationMsg(null)
    setRestoredCount(0)
    annotationCountRef.current = 0
    commentRefs.current.clear()
    clearAnnotationStore(path)
  }

  /** 工具栏「系统打开 / 在文件夹显示」的失败反馈：此前 `.catch(() => undefined)`
   *  静默吞错，表现为「点了没反应」。内容区那侧（FilePreviewContent 的 openErr）
   *  早有反馈，这里补同一语义；横幅复用 preview-overlay.css 的 `.pv-open-error`
   *  ——该样式自上线起没有 .tsx 引用，正是这半边修复漏接的。 */
  const [openErr, setOpenErr] = useState<string | null>(null)

  const handleReveal = () => {
    setOpenErr(null)
    revealPath(path).catch(e => setOpenErr(typeof e === 'string' ? e : String(e)))
  }
  const handleOpen = () => {
    setOpenErr(null)
    openPath(path).catch(e => setOpenErr(typeof e === 'string' ? e : String(e)))
  }

  return createPortal(
    <div className="pv-page" {...(annotating ? { 'data-annotating': '' } : {})}>
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

      {/* ── P4 编辑入口：页面右侧 12% 处的悬浮 ✎（2026-10-08 大王定案）──
                  纯图标（去文字标签），绝对定位于 .pv-page 右上偏中位置：
                  工具栏同行会随页面滚动走，而「进入标注」是常驻操作，
                  固定在页面右侧更可预期；12% 而非垂直居中，避免遮挡页面中部内容。 */}
      {isHtmlPreview && (
        <button
          type="button"
          className={`pv-edit-toggle${annotating ? ' is-active' : ''}`}
          onClick={toggleAnnotating}
          aria-pressed={annotating}
          title={annotating ? t('preview.exitAnnotateMode') : t('preview.annotateMode')}
        >
          <Pencil size={15} />
          {!annotating && pendingCount > 0 && <span className="pv-edit-badge">{pendingCount}</span>}
        </button>
      )}

      {/* ── 修改列表（P4：编辑态展开，承载批注/删除/定位/放弃/发送）── */}
      {isHtmlPreview && annotating && (
        <div className="pv-annot-panel">
          <div className="pv-annot-panel-head">
            <span className="pv-annot-panel-title">
              {annotationCount > 0
                ? t('preview.annotationCount', String(annotationCount))
                : t('preview.annotationList')}
              {restoredCount > 0 && ` ${t('preview.restoredSuffix', String(restoredCount))}`}
            </span>
            {annotationMsg && (
              <span className="pv-annot-panel-file" title={annotationMsg.file}>
                {annotationMsg.file}
              </span>
            )}
          </div>
          {annotationCount === 0 ? (
            <div className="pv-annot-empty">{t('preview.annotationEmpty')}</div>
          ) : (
            <ul className="pv-annot-list">
              {annotations.map((item, i) => (
                <li
                  // key 用 selector 而非 index：删中间项后索引整体前移，用 index 会让
                  // React 把上一个组件的 DOM/state 复用给下一条（批注串条、焦点跳位）
                  key={item.css_selector || `idx-${i}`}
                  className="pv-annot-item"
                >
                  <span className="pv-annot-index">{i + 1}</span>
                  <div className="pv-annot-item-body">
                    <code className="pv-annot-sel" title={item.css_selector}>
                      {item.css_selector}
                    </code>
                    <span className="pv-annot-rect">
                      {item.rect.w}×{item.rect.h} @({item.rect.x},{item.rect.y}) dpr {item.dpr}
                      {item.region &&
                        ` · 区域 ${item.region.w}×${item.region.h} @(${item.region.x},${item.region.y})`}
                    </span>
                    <textarea
                      className="pv-annot-input"
                      value={item.comment}
                      placeholder={t('preview.annotationPlaceholder')}
                      aria-label={`${t('preview.annotationList')} ${i + 1}`}
                      ref={el => {
                        commentRefs.current.set(i, el)
                      }}
                      onChange={e => handleCommentChange(i, e.target.value)}
                    />
                    <div className="pv-annot-item-actions">
                      <button
                        type="button"
                        className="pv-annot-item-btn"
                        title={t('preview.locateTitle')}
                        aria-label={`${t('preview.locateTitle')} ${i + 1}`}
                        onClick={() => handleLocateAnnotation(item)}
                      >
                        <Crosshair size={12} />
                      </button>
                      <button
                        type="button"
                        className="pv-annot-item-btn"
                        title={t('preview.removeTitle')}
                        aria-label={`${t('preview.removeTitle')} ${i + 1}`}
                        onClick={() => handleRemoveAnnotation(i)}
                      >
                        <X size={12} />
                      </button>
                    </div>
                    <details className="pv-annot-html">
                      <summary>{t('preview.outerHtmlSummary')}</summary>
                      <code>{item.outer_html_snippet}</code>
                    </details>
                  </div>
                </li>
              ))}
            </ul>
          )}
          <div className="pv-annot-actions">
            <button
              type="button"
              className="pv-btn"
              onClick={handleDiscardAnnotations}
              disabled={annotationCount === 0}
              title={t('preview.discardTitle')}
            >
              <CircleX size={13} /> {t('preview.discard')}
            </button>
            <button
              type="button"
              className="pv-btn pv-annot-send"
              onClick={handleSendAnnotations}
              disabled={annotationCount === 0 || !onSendAnnotations}
              title={
                onSendAnnotations ? t('preview.sendToAgentTitle') : t('preview.sendUnsupported')
              }
            >
              <Send size={13} /> {t('preview.sendToAgent')}
            </button>
          </div>
        </div>
      )}
      {annotatorStatus === 'missing' && (
        <div className="pv-annot-missing" role="status">
          <button
            type="button"
            className="pv-annot-missing-head"
            onClick={() => setDiagOpen(v => !v)}
            aria-expanded={diagOpen}
            title="点击展开诊断详情：判断标注器断在哪一层"
          >
            <TriangleAlert size={13} />
            <span>{t('preview.annotatorMissing')}</span>
            <span className="pv-annot-missing-toggle">
              {diagOpen ? t('preview.diagCollapse') : t('preview.diagToggle')}
            </span>
          </button>
          {diagOpen && (
            <dl className="pv-annot-diag">
              <div className="pv-annot-diag-row">
                <dt>预览 iframe</dt>
                <dd>{annotatorDiag.frameLoaded ? '已加载' : '未加载'}</dd>
              </div>
              <div className="pv-annot-diag-row">
                <dt>收到消息</dt>
                <dd>{annotatorDiag.messagesSeen} 条</dd>
              </div>
              <div className="pv-annot-diag-row">
                <dt>通过校验</dt>
                <dd>{annotatorDiag.shapeOk} 条</dd>
              </div>
              <div className="pv-annot-diag-row">
                <dt>形状不符被拒</dt>
                <dd>{annotatorDiag.rejectedByShape} 条</dd>
              </div>
              <div className="pv-annot-diag-row">
                <dt>来源不符被拒</dt>
                <dd>{annotatorDiag.rejectedBySource} 条</dd>
              </div>
              <div className="pv-annot-diag-row">
                <dt>握手</dt>
                <dd>{annotatorDiag.readySeen ? '成功' : '未收到'}</dd>
              </div>
              {annotatorDiag.lastReject && (
                <div className="pv-annot-diag-row">
                  <dt>最后拒绝</dt>
                  <dd>{annotatorDiag.lastReject}</dd>
                </div>
              )}
              <p className="pv-annot-diag-hint">
                {annotatorDiag.messagesSeen === 0
                  ? 'iframe 完全没有回传消息 → 注入脚本未执行（页面可能清空过 body，或脚本语法错误）'
                  : annotatorDiag.rejectedBySource > 0
                    ? '消息到了但被来源校验拦截 → 预览 iframe 的 contentWindow 身份不匹配'
                    : '消息到了且通过校验，但没有握手 → 注入脚本可能在握手前抛错'}
              </p>
            </dl>
          )}
        </div>
      )}

      {openErr && (
        <div className="pv-open-error" role="alert">
          <TriangleAlert size={13} />
          <span className="pv-open-error-msg">{openErr}</span>
          <button type="button" className="pv-btn" onClick={() => setOpenErr(null)}>
            知道了
          </button>
        </div>
      )}

      <FilePreviewContent path={path} iframeRef={iframeRef} onIframeLoad={handleFrameLoad} />
    </div>,
    document.body,
  )
}
