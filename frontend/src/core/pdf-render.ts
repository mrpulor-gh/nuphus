// PDF 渲染/文本提取服务 — 主窗口内嵌 pdf.js（离屏 canvas，不挂 DOM）
//
// 用途：扫描件 PDF（无文本层）的 OCR 兜底链路 + 文字版 PDF 的高速文本提取。
// Rust 侧（src-tauri/src/render/）经 window.eval 调用：
// - window.__nuphusRenderPdf(requestId, pdfBase64, maxPages, pageList?)
//     按页渲染为 PNG。pageList 为可选 1-based 页码数组（混合 PDF 仅渲染
//     无文本层页）；缺省时维持 1..=min(numPages, maxPages) 旧行为。
// - window.__nuphusRenderPdfText(requestId, pdfBase64, maxPages)
//     getTextContent 逐页提取文本层；空串 = 该页无文本层（OCR 候选）。
// 两者完成后均 invoke('pdf_render_done') / invoke('pdf_render_error') 回传，
// Rust 侧 oneshot 等待结果（60s 超时）。
//
// 约束（与 plan 对齐）：
// - 不新开窗口、不落临时文件（全程内存）
// - pdf.js 与 worker 打进 bundle（?url import），禁止 CDN 外链

import * as pdfjsLib from 'pdfjs-dist'
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl

// 渲染放大倍数：扫描件走 OCR，2.0 保证识别精度
const RENDER_SCALE = 2.0
const PNG_PREFIX = 'data:image/png;base64,'

declare global {
  interface Window {
    __nuphusRenderPdf?: (
      requestId: string,
      pdfBase64: string,
      maxPages: number,
      pageList?: number[] | null,
    ) => void
    __nuphusRenderPdfText?: (requestId: string, pdfBase64: string, maxPages: number) => void
  }
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) {
    bytes[i] = bin.charCodeAt(i)
  }
  return bytes
}

/// 逐页渲染 PDF → PNG base64 数组（不带 data URL 前缀）。
/// pageList 缺省时渲染 1..=min(numPages, maxPages)；越界页码静默跳过。
async function renderPdfPages(
  pdfBase64: string,
  maxPages: number,
  pageList?: number[] | null,
): Promise<string[]> {
  const data = base64ToBytes(pdfBase64)
  const loadingTask = pdfjsLib.getDocument({ data })
  const doc = await loadingTask.promise
  try {
    const targets =
      pageList && pageList.length > 0
        ? pageList.filter(n => n >= 1 && n <= doc.numPages)
        : Array.from({ length: Math.min(doc.numPages, Math.max(1, maxPages)) }, (_, i) => i + 1)
    const pages: string[] = []
    for (const n of targets) {
      const page = await doc.getPage(n)
      const viewport = page.getViewport({ scale: RENDER_SCALE })
      const canvas = document.createElement('canvas')
      canvas.width = Math.floor(viewport.width)
      canvas.height = Math.floor(viewport.height)
      const ctx = canvas.getContext('2d')
      if (!ctx) {
        throw new Error('无法创建 canvas 2d 上下文')
      }
      await page.render({ canvas, canvasContext: ctx, viewport }).promise
      const dataUrl = canvas.toDataURL('image/png')
      if (!dataUrl.startsWith(PNG_PREFIX)) {
        throw new Error(`第 ${n} 页 canvas 导出 PNG 失败`)
      }
      pages.push(dataUrl.slice(PNG_PREFIX.length))
      page.cleanup()
      // 主动释放位图内存（扫描件多页时单页位图可达数十 MB）
      canvas.width = 0
      canvas.height = 0
    }
    return pages
  } finally {
    await loadingTask.destroy()
  }
}

/// 逐页提取文本层：空串表示该页无文本层（扫描页，交给 Rust 侧分流 OCR）
async function extractPdfText(pdfBase64: string, maxPages: number): Promise<string[]> {
  const data = base64ToBytes(pdfBase64)
  const loadingTask = pdfjsLib.getDocument({ data })
  const doc = await loadingTask.promise
  try {
    const pageCount = Math.min(doc.numPages, Math.max(1, maxPages))
    const texts: string[] = []
    for (let i = 1; i <= pageCount; i++) {
      const page = await doc.getPage(i)
      const content = await page.getTextContent()
      let text = ''
      for (const item of content.items) {
        // TextMarkedContent 项无 str 字段，跳过
        if ('str' in item) {
          text += item.str
          if (item.hasEOL) {
            text += '\n'
          }
        }
      }
      texts.push(text)
      page.cleanup()
    }
    return texts
  } finally {
    await loadingTask.destroy()
  }
}

async function handleRender(
  requestId: string,
  pdfBase64: string,
  maxPages: number,
  pageList?: number[] | null,
): Promise<void> {
  // 动态引入：本模块被 eval 触发时必然在 Tauri webview 内，但保持与 bridge.ts 一致的容错风格
  const { invoke } = await import('@tauri-apps/api/core')
  try {
    const pages = await renderPdfPages(pdfBase64, maxPages, pageList)
    await invoke('pdf_render_done', { requestId, pages })
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e)
    await invoke('pdf_render_error', { requestId, error })
  }
}

async function handleExtract(
  requestId: string,
  pdfBase64: string,
  maxPages: number,
): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core')
  try {
    const pages = await extractPdfText(pdfBase64, maxPages)
    await invoke('pdf_render_done', { requestId, pages })
  } catch (e) {
    const error = e instanceof Error ? e.message : String(e)
    await invoke('pdf_render_error', { requestId, error })
  }
}

window.__nuphusRenderPdf = (requestId, pdfBase64, maxPages, pageList) => {
  void handleRender(requestId, pdfBase64, maxPages, pageList)
}

window.__nuphusRenderPdfText = (requestId, pdfBase64, maxPages) => {
  void handleExtract(requestId, pdfBase64, maxPages)
}
