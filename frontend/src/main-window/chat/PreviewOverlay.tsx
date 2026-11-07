import React, { useEffect, useLayoutEffect, useRef, useState } from 'react'
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
 * HTML 预览 URL：`convertFileSrc` 把整条绝对路径 encodeURIComponent 成
 * **单个 path 段**（Windows: `C%3A%5CUsers%5C...%5Cindex.html`）。浏览器解析
 * 相对资源时按 URL 的 `/` 定目录——`%5C` / `%2F` 不是 `/`，目录边界消失：
 * `assets/site.css` 会被解析到 `http://preview.localhost/assets/site.css`，
 * `../assets/site.css` 更是直接丢掉整条路径。官网这类带相对 CSS/JS 的页面
 * 因此无样式、无脚本。
 *
 * 把编码分隔符还原成真实 `/`，让 URL 恢复目录结构：
 * - Windows → `C%3A/Users/.../index.html`
 * - Unix    → `%2FUsers/.../index.html`（首个 `%2F` 保留：协议端
 *   `trim_start_matches('/')` 会剥掉真实前导斜杠，保留编码形式才能还原绝对路径）
 *
 * 协议端 percent_decode 后仍是合法绝对路径（Windows 接受 `/` 与 `\` 混用）。
 * 文件名本身不可能包含 `\` / `/`，替换不会误伤。
 */
export function previewFileSrc(path: string): string {
  const url = convertFileSrc(path, 'preview')
  if (url.includes('%5C')) return url.replace(/%5C/gi, '/')
  const first = url.indexOf('%2F')
  if (first < 0) return url
  // 保留首个编码斜杠（Unix 绝对路径根），其余分隔符还原为真实 '/'
  return url.slice(0, first + 3) + url.slice(first + 3).replace(/%2F/gi, '/')
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
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return null
    // localStorage 是不可信输入（损坏/手改/旧版本残留）：渲染直接读 item.rect.w，
    // 缺 rect 的条目会让整个预览树 TypeError——逐条校验，坏条目丢弃不阻塞其余。
    const valid = parsed.filter(isStoredAnnotationItem)
    return valid.length > 0 ? valid : null
  } catch (err) {
    // 存储损坏 / 隐私模式：不阻塞预览，仅报错（best-effort）
    console.error('[nuphus-annotator] 读取标注历史失败', err)
    return null
  }
}

/** localStorage 历史条目的最小形状校验（比协议校验宽松：snippet/dpr 缺失只降级展示） */
function isStoredAnnotationItem(a: unknown): a is AnnotationItem {
  if (typeof a !== 'object' || a === null) return false
  const it = a as { css_selector?: unknown; outer_html_snippet?: unknown; comment?: unknown }
  return (
    typeof it.css_selector === 'string' &&
    typeof it.outer_html_snippet === 'string' &&
    typeof it.comment === 'string' &&
    isRectLike((a as { rect?: unknown }).rect)
  )
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
 * 且 `nuphus:annotations` 的annotations 必须是数组、每条字段类型正确
 * （含 rect 四元数字——渲染直接读 item.rect.w，缺 rect 会 TypeError 崩掉整个预览树）。
 * 这样即便别的窗口/扩展往主窗口 postMessage 相同形状的数据，也进不了标注状态机。
 *
 * 上行三条消息均要求 `file` 字符串：handler 会再与当前预览文件的 basename 比对，
 * 挡掉「别的 iframe / 旧文档残留」的同形状消息（双 PreviewOverlay 并存时防串话）。
 */
function isAnnotatorMessage(data: {
  type?: unknown
  file?: unknown
  annotations?: unknown
}): boolean {
  if (typeof data.type !== 'string' || !ANNOTATOR_UP_TYPES.has(data.type)) return false
  if (typeof data.file !== 'string' || !data.file) return false
  if (data.type === 'nuphus:annotations') {
    if (!Array.isArray(data.annotations)) return false
    return data.annotations.every(
      (a: unknown) =>
        typeof a === 'object' &&
        a !== null &&
        typeof (a as { css_selector?: unknown }).css_selector === 'string' &&
        typeof (a as { outer_html_snippet?: unknown }).outer_html_snippet === 'string' &&
        typeof (a as { comment?: unknown }).comment === 'string' &&
        isRectLike((a as { rect?: unknown }).rect),
    )
  }
  return true
}

/** rect / region 形状：x/y/w/h 必须是有限数字（渲染与序列化都会直接读这四个字段）*/
function isRectLike(v: unknown): boolean {
  if (typeof v !== 'object' || v === null) return false
  const r = v as { x?: unknown; y?: unknown; w?: unknown; h?: unknown }
  return (
    typeof r.x === 'number' &&
    Number.isFinite(r.x) &&
    typeof r.y === 'number' &&
    Number.isFinite(r.y) &&
    typeof r.w === 'number' &&
    Number.isFinite(r.w) &&
    typeof r.h === 'number' &&
    Number.isFinite(r.h)
  )
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
 * 供 PreviewOverlay（全屏覆盖层）与 ToolsPage（独立整页里的预览区）复用：
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
   * ToolsPage 的预览区不传，行为不变。
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
          src={previewFileSrc(path)}
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
   *  listener 在挂载时（useLayoutEffect）注册，**不在 onLoad**：
   *  注入脚本在 iframe 的 DOMContentLoaded 就发出 ready 握手，而 iframe 的 load
   *  事件（资源加载完）在其之后——挂载才注册会把先行到达的握手直接丢弃
   *  （postMessage 不缓冲、不补投），2s 超时恒触发「标注器未就绪」。
   *  防伪不依赖注册时机：形状白名单 + 实时 iframe 集合扫描 + file basename 比对。 */
  const iframeRef = useRef<HTMLIFrameElement | null>(null)
  const [annotationMsg, setAnnotationMsg] = useState<AnnotationMessage | null>(null)
  /** 标注器握手：pending=等握手 2s；ready=已握手；missing=超时未握手（注入失败/页面异常） */
  const [annotatorStatus, setAnnotatorStatus] = useState<'pending' | 'ready' | 'missing' | null>(
    null,
  )
  /** 握手诊断（红条展开时展示断点证据，见 AnnotatorDiag）*/
  const [annotatorDiag, setAnnotatorDiag] = useState<AnnotatorDiag>(freshDiag)
  /** 握手诊断的 ref 镜像：message handler 挂载注册一次，靠 ref 累加计数
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
  /** 批注本地草稿（key = 标注项稳定键）。textarea 不能直接受控于 iframe 快照：
   *  输入链路是 onChange → postMessage → iframe setComment → 快照回流，每个按键一次
   *  异步往返；回流前 React 会拿滞后值重渲染 textarea，IME 组合期（拼音）被反复重置，
   *  表现为字母重复、打不成句（2026-11-07 大王报障）。草稿本地同步更新，回流不覆盖。 */
  const [commentDrafts, setCommentDrafts] = useState<Record<string, string>>({})
  /** IME 组合期闸门：组合中只写本地草稿，compositionend 再下发一次，减少无谓往返 */
  const composingRef = useRef(false)
  /** 定位序列号：同一 selector 连续 ◎ 也让 iframe 侧动画重启 */
  const focusSeqRef = useRef(0)
  /** 快照条数与编辑态的 ref 镜像：message handler 只在挂载时注册一次，
   *  读不到当次的 state 值，用 ref 保持最新又不重挂 listener。 */
  const annotationCountRef = useRef(0)
  const annotatingRef = useRef(false)
  /** 当前预览文件路径的 ref 镜像：挂载注册的 handler 需要读到最新 path
   *  （file basename 校验 + localStorage 落库键），切换文件时随 layout effect 更新。 */
  const pathRef = useRef(path)
  /** ready 握手是否已达（ref：listener 挂载一次，跨 onLoad 存活——握手可能先于 load） */
  const readySeenRef = useRef(false)
  /** 握手超时计时器（onLoad 起 2s；ready 先到则 onLoad 不再起表） */
  const readyTimerRef = useRef<number | null>(null)
  const clearReadyTimer = () => {
    if (readyTimerRef.current != null) {
      window.clearTimeout(readyTimerRef.current)
      readyTimerRef.current = null
    }
  }

  /** P4 编辑态镜像到 ref：message handler 挂载注册一次，
   *  靠 ref 读到最新编辑态而不必重挂 listener */
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

  /** 标注项稳定键：selector + 矩形锚点 + 渲染序。
   *  同一 selector 上可能有多条区域标注（rect 区分）；序号兜底同 rect 重复标注。 */
  const annotationItemKey = (item: AnnotationItem, index: number) =>
    `${item.css_selector || 'region'}|${item.rect.x},${item.rect.y},${item.rect.w},${item.rect.h}|${index}`

  /** P4 批注回写：本地草稿先落地（textarea 的 value 唯一来源），再下发 iframe。
   *  顺序不可反：iframe 快照是异步回流的，若拿它驱动 value，输入会被滞后数据覆盖。 */
  const handleCommentChange = (index: number, comment: string) => {
    const item = annotations[index]
    if (item) {
      const key = annotationItemKey(item, index)
      setCommentDrafts(prev => ({ ...prev, [key]: comment }))
    }
    // 组合期只更新本地，compositionend 的最终值再下发，避免每个拼音字母一次往返
    if (composingRef.current) return
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

  // 关闭覆盖层 / 切换预览文件：重置载荷、握手、诊断与编辑态，防旧文件的标注
  // 残留在新预览上。与 listener 的挂载/切换在同一个 layout effect 里原子完成——
  // 拆成两个 effect 时，「新 listener 已挂、旧 state 未清」的窗口里先到的握手
  // 会被随后的 reset 冲掉（status 回 null 后 readySeenRef 已 true，onLoad 不再
  // 起表，界面既无红条也无就绪态）。
  // useLayoutEffect：在 paint 前同步执行，保证任何异步消息（postMessage 是宏任务）
  // 被处理前，reset + pathRef 更新 + listener 切换已全部就位。
  useLayoutEffect(() => {
    setAnnotationMsg(null)
    setAnnotatorStatus(null)
    setRestoredCount(0)
    annotatorDiagRef.current = freshDiag()
    setAnnotatorDiag(annotatorDiagRef.current)
    setAnnotating(false)
    annotationCountRef.current = 0
    annotatingRef.current = false
    commentRefs.current.clear()
    setCommentDrafts({})
    readySeenRef.current = false
    clearReadyTimer()
    pathRef.current = path
    if (!HTML_EXTS.has(extOf(path))) return
    const handler = (event: MessageEvent) => {
      // ── 诊断埋点：逐层记录判定结果（红条展开时可见，见 AnnotatorDiag）──
      const diag = annotatorDiagRef.current
      diag.messagesSeen += 1
      // ── 防伪校验（2026-10-08 实机「标注器未就绪」修复 + 挂载注册防丢握手）──
      //
      // 原实现把 contentWindow 捕获在 onLoad 那一刻做 `event.source !== contentWindow`
      // 恒等比对，实机握手 100% 被拦。原因是 preview:// 文档在 onLoad 之后仍可能
      // 发生一次导航/代理重建，iframe.contentWindow 指向**新的** WindowProxy，
      // 而 iframe 内 window.parent.postMessage 的 event.source 是它自己的 WindowProxy
      // ——两个 proxy 对象在沙箱+自定义协议下不保证是同一 JS 对象身份。
      //
      // 放宽为三段式，仍保留防伪能力：
      //   ① 消息形状必须严格匹配本协议（type 白名单 + file 非空 + 载荷结构校验）；
      //   ② event.source 必须是「某个」iframe 的 contentWindow —— 用
      //      [...iframe 集合].some(f => f.contentWindow === event.source)，
      //      对比实时取到的 contentWindow，绕开「捕获时快照」的失效问题；
      //   ③ file basename 必须等于当前预览文件 —— 双 PreviewOverlay 并存（ChatPanel
      //      与外部 agent 状态栏各托管一个）且 source 扫描是「任意 iframe」时，
      //      这一条把另一实例/旧文档残留的同形状消息挡在状态机外。
      const data = event.data as
        | {
            type?: unknown
            file?: unknown
            annotations?: unknown
          }
        | null
        | undefined
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
      const fromOurFrame = Array.from(document.querySelectorAll('iframe')).some(
        f => f.contentWindow && f.contentWindow === event.source,
      )
      if (!fromOurFrame) {
        diag.rejectedBySource += 1
        diag.lastReject = '消息来源不是本预览 iframe（被防伪校验拦截）'
        setAnnotatorDiag({ ...diag })
        return
      }
      if (data.file !== baseName(pathRef.current)) {
        diag.rejectedBySource += 1
        diag.lastReject = `消息文件与当前预览不匹配（${String(data.file)}）`
        setAnnotatorDiag({ ...diag })
        return
      }
      diag.shapeOk += 1
      const frame = iframeRef.current
      const contentWindow = frame?.contentWindow
      if (!contentWindow) return
      if (data.type === 'nuphus:annotator-ready') {
        readySeenRef.current = true
        diag.readySeen = true
        clearReadyTimer()
        setAnnotatorStatus('ready')
        setAnnotatorDiag({ ...diag })
        // 先下发主题令牌再谈恢复：标注器配色必须来自当前主题（父窗口 token），
        // 否则角标/高亮会用脚本兜底色，与界面 accent 不一致。
        contentWindow.postMessage(
          { type: 'nuphus:annotate-theme', tokens: readAnnotationThemeTokens(null) },
          '*',
        )
        // P3 持久化恢复：握手后把同文件的历史标注下发 iframe（selector 回定位 DOM）
        const restored = readAnnotationStore(pathRef.current)
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
        writeAnnotationStore(pathRef.current, msg)
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
    return () => {
      window.removeEventListener('message', handler)
      clearReadyTimer()
    }
  }, [path, t])

  const handleFrameLoad = () => {
    const frame = iframeRef.current
    if (!frame?.contentWindow) return
    annotatorDiagRef.current = { ...annotatorDiagRef.current, frameLoaded: true }
    setAnnotatorDiag({ ...annotatorDiagRef.current })
    // ready 可能先于 onLoad 到达（注入脚本在 DOMContentLoaded 即发握手，load 要等
    // 资源）——已握手则不再进入等待态，也不再起超时表（否则把 ready 冲成 missing）。
    if (readySeenRef.current) return
    setAnnotatorStatus('pending')
    // 握手兜底：onLoad 后 2s 未收到 ready → 提示标注器未就绪（best-effort，常见于
    // overlay 注入脚本被宿主页面清理 / 页面自身脚本异常）
    clearReadyTimer()
    readyTimerRef.current = window.setTimeout(() => {
      if (!readySeenRef.current) setAnnotatorStatus('missing')
    }, 2000)
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
    setCommentDrafts({})
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
    setCommentDrafts({})
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
                  // key 用稳定键（selector + 矩形 + 序号）：删中间项后 key 变化会重挂，
                  // 但不会把上一条的 DOM/草稿复用到下一条（批注串条、焦点跳位）
                  key={annotationItemKey(item, i)}
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
                      // 本地草稿优先：iframe 快照只作初始值/外部恢复来源，输入期间不覆盖
                      value={commentDrafts[annotationItemKey(item, i)] ?? item.comment}
                      placeholder={t('preview.annotationPlaceholder')}
                      aria-label={`${t('preview.annotationList')} ${i + 1}`}
                      ref={el => {
                        commentRefs.current.set(i, el)
                      }}
                      onChange={e => handleCommentChange(i, e.target.value)}
                      onCompositionStart={() => {
                        composingRef.current = true
                      }}
                      onCompositionEnd={e => {
                        composingRef.current = false
                        handleCommentChange(i, e.currentTarget.value)
                      }}
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
